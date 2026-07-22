//! SABRE routing pass (issue #25, SPEC §7.4).
//!
//! Maps logical qubits to physical indices and inserts SWAP gates to satisfy
//! connectivity constraints on fixed-target topology.

use std::collections::HashMap;
use std::sync::Arc;

use backend::target::{BackendTarget, FixedTarget};
use melior::StringRef;
use melior::ir::attribute::IntegerAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{AttributeLike, BlockLike, Location, OperationRef, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef};
use mlir_sys::{mlirOperationSetAttributeByName, mlirOperationSetOperand};
use thiserror::Error;

use crate::dialect::{quantum_circ, quantum_dynamic};
use crate::passes::qubit_wiring::{self, WireTracker};

fn set_i32_attr<'c>(context: &'c Context, op: OperationRef<'c, '_>, key: &str, value: i32) {
    let attribute: melior::ir::Attribute<'_> = IntegerAttribute::new(
        melior::ir::r#type::IntegerType::new(context, 32).into(),
        i64::from(value),
    )
    .into();
    unsafe {
        mlirOperationSetAttributeByName(
            op.to_raw(),
            StringRef::new(key).to_raw(),
            attribute.to_raw(),
        );
    }
}
#[derive(Clone, Copy, Debug)]
pub struct SabreCost {
    pub alpha: f64,
    pub beta: f64,
    pub gamma: f64,
    pub lookahead: usize,
}

impl Default for SabreCost {
    fn default() -> Self {
        Self {
            alpha: 1.0,
            beta: 0.5,
            gamma: 0.3,
            lookahead: 20,
        }
    }
}

#[derive(Debug, Error)]
pub enum RouteError {
    #[error("failed to build `{op}`: {message}")]
    Build { op: &'static str, message: String },
}

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn append_swap<'c, 'a>(
    context: &'c Context,
    block: melior::ir::BlockRef<'c, 'a>,
    before: OperationRef<'c, 'a>,
    q0: Value<'c, 'a>,
    q1: Value<'c, 'a>,
    location: Location<'c>,
) -> Result<(Value<'c, 'a>, Value<'c, 'a>), RouteError> {
    let op =
        quantum_circ::gate(context, "SWAP", 1, true, &[q0, q1], location).map_err(|error| {
            RouteError::Build {
                op: quantum_circ::op::GATE,
                message: error.to_string(),
            }
        })?;
    let op_ref = block.insert_operation_before(before, op);
    Ok((
        Value::from(op_ref.result(0).map_err(|_| RouteError::Build {
            op: quantum_circ::op::GATE,
            message: "missing swap result 0".to_string(),
        })?),
        Value::from(op_ref.result(1).map_err(|_| RouteError::Build {
            op: quantum_circ::op::GATE,
            message: "missing swap result 1".to_string(),
        })?),
    ))
}

#[derive(Clone)]
struct Layout {
    /// logical value key -> physical index
    mapping: HashMap<usize, usize>,
    /// physical index -> logical value key
    inverse: Vec<Option<usize>>,
}

impl Layout {
    fn new(num_qubits: usize) -> Self {
        Self {
            mapping: HashMap::new(),
            inverse: vec![None; num_qubits],
        }
    }

    fn assign(&mut self, logical: usize, physical: usize) -> Result<(), RouteError> {
        if physical >= self.inverse.len() {
            return Err(RouteError::Build {
                op: quantum_circ::op::GATE,
                message: format!("logical qubit {logical} exceeds physical device size"),
            });
        }
        self.mapping.insert(logical, physical);
        self.inverse[physical] = Some(logical);
        Ok(())
    }

    fn phys(&self, logical: usize) -> Result<usize, RouteError> {
        self.mapping
            .get(&logical)
            .copied()
            .ok_or_else(|| RouteError::Build {
                op: quantum_circ::op::GATE,
                message: format!("unassigned logical qubit {logical}"),
            })
    }

    fn logical_at(&self, physical: usize) -> Result<usize, RouteError> {
        self.inverse
            .get(physical)
            .and_then(|value| *value)
            .ok_or_else(|| RouteError::Build {
                op: quantum_circ::op::GATE,
                message: format!("empty physical slot {physical}"),
            })
    }

    fn swap_phys(&mut self, a: usize, b: usize) -> Result<(usize, usize), RouteError> {
        if a >= self.inverse.len() || b >= self.inverse.len() {
            return Err(RouteError::Build {
                op: quantum_circ::op::GATE,
                message: "swap endpoint outside physical device".to_string(),
            });
        }
        let la = self.inverse[a];
        let lb = self.inverse[b];
        if let Some(logical) = la {
            self.mapping.insert(logical, b);
        }
        if let Some(logical) = lb {
            self.mapping.insert(logical, a);
        }
        self.inverse[a] = lb;
        self.inverse[b] = la;
        let Some(la) = la else {
            return Err(RouteError::Build {
                op: quantum_circ::op::GATE,
                message: format!("empty physical slot {a}"),
            });
        };
        let Some(lb) = lb else {
            return Err(RouteError::Build {
                op: quantum_circ::op::GATE,
                message: format!("empty physical slot {b}"),
            });
        };
        Ok((la, lb))
    }
}

fn set_qubit_operands<'c, 'a>(gate: OperationRef<'c, 'a>, values: &[Value<'c, 'a>]) {
    let mut qubit_index = 0usize;
    for operand_index in 0..gate.operand_count() {
        let Ok(operand) = gate.operand(operand_index) else {
            continue;
        };
        if !quantum_circ::is_qubit_type(operand.r#type()) {
            continue;
        }
        if let Some(value) = values.get(qubit_index) {
            unsafe {
                mlirOperationSetOperand(gate.to_raw(), operand_index as isize, value.to_raw());
            }
        }
        qubit_index += 1;
    }
}

#[allow(clippy::too_many_arguments)]
fn route_two_qubit<'c, 'a>(
    context: &'c Context,
    target: &FixedTarget,
    cost: SabreCost,
    layout: &mut Layout,
    block: melior::ir::BlockRef<'c, 'a>,
    gate: OperationRef<'c, 'a>,
    logical_a: usize,
    logical_b: usize,
    wires: &mut HashMap<usize, Value<'c, 'a>>,
    tracker: &mut WireTracker,
) -> Result<(), RouteError> {
    let location = gate.location();
    let mut p_a = layout.phys(logical_a)?;
    let mut p_b = layout.phys(logical_b)?;

    // The lookahead window `W` (SPEC §7.4): the next `cost.lookahead` two-qubit
    // interactions after this gate, by logical-qubit identity. Fixed once per
    // gate (not recomputed per hop) since it's independent of which physical
    // swap we're currently scoring — only the *candidate mapping* varies
    // across `best_swap`'s calls, not the set of upcoming interactions.
    let window = collect_lookahead_window(gate, cost.lookahead, tracker);

    while target.topology.dist(p_a, p_b) > 1 {
        let (u, v) = best_swap(target, cost, layout, &window, p_a, p_b)?;

        let logical_u = layout.logical_at(u)?;
        let logical_v = layout.logical_at(v)?;
        let wire_u = wires[&logical_u];
        let wire_v = wires[&logical_v];
        let (out_u, out_v) = append_swap(context, block, gate, wire_u, wire_v, location)?;
        let (new_u, new_v) = layout.swap_phys(u, v)?;
        // A register slot is a fixed physical location: SWAP exchanges the
        // *contents* of slots u and v, it does not relabel the slots
        // themselves — so `out_u` (the SWAP result continuing slot u) is
        // where `logical_v`'s state now lives, and `out_v` (slot v) is where
        // `logical_u`'s state now lives. `wires[logical]` must track "the
        // value to use as this logical qubit's next operand", which is the
        // *other* result — assigning it positionally (`out_u` to `logical_u`)
        // would silently hand every later gate the wrong qubit's state.
        wires.insert(new_u, out_v);
        wires.insert(new_v, out_u);
        // The SWAP's results are freshly-built SSA values with no established
        // root yet; without this, any later gate reading them would fall
        // through `WireTracker::root`'s "unseen value" default (its own raw
        // pointer key) instead of resolving back to the correct logical qubit,
        // silently fabricating an extra logical qubit past the device's count.
        tracker.alias(out_v, new_u);
        tracker.alias(out_u, new_v);
        p_a = layout.phys(logical_a)?;
        p_b = layout.phys(logical_b)?;
    }

    set_qubit_operands(gate, &[wires[&logical_a], wires[&logical_b]]);
    Ok(())
}

fn best_swap(
    target: &FixedTarget,
    cost: SabreCost,
    layout: &Layout,
    window: &[(usize, usize)],
    p_a: usize,
    p_b: usize,
) -> Result<(usize, usize), RouteError> {
    // Lexicographic score: (post-swap front distance, secondary). Distance is
    // primary so β / γ can never prefer a hop that lengthens the front-layer
    // pair — otherwise `while dist > 1` can oscillate forever. Secondary folds
    // β·critical_path_delta and γ·noise only among equal-distance candidates.
    let mut best: Option<((usize, usize), usize, f64)> = None;
    let current_dist = target.topology.dist(p_a, p_b);
    let baseline_window_cost = window_swap_depth(target, layout, window);
    for &(u, v) in &target.topology.edges {
        // Only SWAPs that move at least one front-layer endpoint can reduce
        // `current_dist`; unrelated edges preserve distance and would spin.
        if u != p_a && u != p_b && v != p_a && v != p_b {
            continue;
        }
        if layout.inverse.get(u).and_then(|value| *value).is_none()
            || layout.inverse.get(v).and_then(|value| *value).is_none()
        {
            continue;
        }
        let swapped_a = if p_a == u {
            v
        } else if p_a == v {
            u
        } else {
            p_a
        };
        let swapped_b = if p_b == u {
            v
        } else if p_b == v {
            u
        } else {
            p_b
        };
        let distance = target.topology.dist(swapped_a, swapped_b);
        // Hard progress: never accept a connectivity regression for the gate
        // we are currently routing.
        if distance > current_dist {
            continue;
        }
        let critical_path_delta = if cost.beta == 0.0 || window.is_empty() {
            0.0
        } else {
            let mut candidate = layout.clone();
            match candidate.swap_phys(u, v) {
                Ok(_) => window_swap_depth(target, &candidate, window) - baseline_window_cost,
                Err(_) => 0.0,
            }
        };
        let secondary = cost.beta * critical_path_delta + cost.gamma * noise_penalty(target, u, v);
        // `alpha` remains on SabreCost for SPEC §7.4 / CLI parity; lexicographic
        // distance-first makes it redundant for ranking (distance is primary).
        let better = match best {
            None => true,
            Some((_, best_dist, best_secondary)) => {
                distance < best_dist || (distance == best_dist && secondary < best_secondary)
            }
        };
        if better {
            best = Some(((u, v), distance, secondary));
        }
    }
    best.map(|(edge, _, _)| edge)
        .ok_or_else(|| RouteError::Build {
            op: quantum_circ::op::GATE,
            message: "no legal swap candidate".to_string(),
        })
}

/// Collects the lookahead window `W` (SPEC §7.4): the logical-qubit pairs of
/// up to `limit` two-qubit `quantum.circ.gate` ops following `after` in
/// program order, within the same block. `limit == 0` (the CLI/SPEC knob for
/// disabling lookahead) yields an empty window, which zeroes out `beta`'s
/// contribution below regardless of its value.
///
/// Only tracks straight-line successors: it does not recurse into
/// `quantum.dynamic.unitary_region`/`if` bodies encountered along the way, so
/// interactions nested inside those regions are not part of the window. This
/// is a scoring heuristic only — it never affects the SWAPs actually emitted
/// for correctness, only which of several equally-distant candidates is
/// preferred, so under-counting the window here costs some routing quality,
/// never correctness.
fn collect_lookahead_window<'c, 'a>(
    after: OperationRef<'c, 'a>,
    limit: usize,
    tracker: &WireTracker,
) -> Vec<(usize, usize)> {
    // Peek with a clone so scoring never mutates the live tracker. Observe
    // `after` first: successors often consume this gate's results, and those
    // SSA values are not yet aliased on the live tracker (observe happens
    // after routing in `route_block`).
    let mut peek = tracker.clone();
    peek.observe_operation(after);
    let mut window = Vec::with_capacity(limit.min(64));
    let mut op = after.next_in_block();
    while let Some(current) = op {
        if window.len() >= limit {
            break;
        }
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN || name == quantum_dynamic::op::YIELD {
            break;
        }
        op = current.next_in_block();
        if name != quantum_circ::op::GATE {
            continue;
        }
        let roots = peek.roots_for_operands(current);
        if let [logical_a, logical_b] = roots[..] {
            window.push((logical_a, logical_b));
        }
        peek.observe_operation(current);
    }
    window
}

/// Estimated additional SWAP-induced depth (SPEC §7.4 `critical_path_delta`)
/// to execute the lookahead window's interactions under `layout`: each
/// interaction whose endpoints are `d` hops apart needs `d - 1` more SWAPs
/// (hence `d - 1` more depth layers) before it is directly executable.
///
/// This approximates `depth(schedule_ASAP(apply_mapping(M, W)))` from the
/// SPEC formula via connectivity distance rather than running a full ASAP
/// scheduler per candidate mapping — cheap enough to evaluate once per
/// topology edge in `best_swap`'s inner loop, while still being sensitive to
/// exactly the thing `beta` is meant to penalize: a swap that moves a qubit
/// away from its next partner(s).
fn window_swap_depth(target: &FixedTarget, layout: &Layout, window: &[(usize, usize)]) -> f64 {
    window
        .iter()
        .filter_map(|&(logical_a, logical_b)| {
            let phys_a = layout.phys(logical_a).ok()?;
            let phys_b = layout.phys(logical_b).ok()?;
            Some(target.topology.dist(phys_a, phys_b).saturating_sub(1) as f64)
        })
        .sum()
}

/// Noise cost for swapping / using the physical edge `(a, b)`.
///
/// Looks up two-qubit fidelity for any native 2Q gate in either direction
/// (`(a,b)` or `(b,a)`), falls back to `cx` when the native set is empty of
/// named 2Q entries in the noise map, and adds a light readout-error influence
/// so noisy measurement qubits are slightly disfavored as SWAP endpoints.
fn noise_penalty(target: &FixedTarget, a: usize, b: usize) -> f64 {
    let gate_penalty = two_qubit_noise_penalty(target, a, b);
    let readout_penalty = 0.5 * (readout_penalty(target, a) + readout_penalty(target, b));
    gate_penalty + readout_penalty
}

fn fidelity_to_penalty(fidelity: f64) -> f64 {
    if fidelity <= 0.0 {
        // Treat non-positive fidelity as maximally bad without panicking on ln.
        f64::INFINITY
    } else if fidelity >= 1.0 {
        0.0
    } else {
        -fidelity.ln()
    }
}

fn two_qubit_noise_penalty(target: &FixedTarget, a: usize, b: usize) -> f64 {
    if let Some(f) = best_two_qubit_fidelity(target, a, b) {
        return fidelity_to_penalty(f);
    }
    0.0
}

fn best_two_qubit_fidelity(target: &FixedTarget, a: usize, b: usize) -> Option<f64> {
    let mut best: Option<f64> = None;
    let native_two_qubit: Vec<&str> = target
        .native_gates
        .iter()
        .filter(|g| g.num_qubits == 2)
        .map(|g| g.name.as_str())
        // SWAP is a routing primitive; noise models publish entangling-gate
        // fidelities (cx/ecr/cz/…), not SWAP error rates.
        .filter(|name| !name.eq_ignore_ascii_case("swap"))
        .collect();

    let gate_names: Vec<&str> = if native_two_qubit.is_empty() {
        vec!["cx"]
    } else {
        native_two_qubit
    };

    for gate in gate_names {
        for &(u, v) in &[(a, b), (b, a)] {
            if let Some(f) = target
                .noise
                .two_qubit_fidelity
                .get(&(gate.to_string(), u, v))
                .copied()
            {
                best = Some(best.map_or(f, |cur| cur.max(f)));
            }
        }
    }
    best
}

fn readout_penalty(target: &FixedTarget, q: usize) -> f64 {
    target
        .noise
        .readout_error
        .get(&q)
        .copied()
        .map(|e| {
            // Map readout error e ∈ [0,1] onto a soft penalty comparable to
            // -ln(fidelity) for typical CX fidelities (~0.005–0.02).
            let e = e.clamp(0.0, 1.0);
            if e <= 0.0 {
                0.0
            } else {
                -((1.0 - e).max(1e-12)).ln()
            }
        })
        .unwrap_or(0.0)
}

/// Physical-qubit-continuity state threaded across an entire module traversal.
///
/// A qubit's physical assignment is a hardware fact, not a per-block one: the
/// same logical qubit must resolve to the same physical index whether it's
/// referenced inside a top-level block, a nested `quantum.dynamic.unitary_region`,
/// or either arm of a `quantum.dynamic.if`. This bundles the state that must
/// therefore survive across those region boundaries rather than reset at each
/// one (see `recurse_region`, which aliases a region's block arguments to the
/// caller's already-established roots instead of seeding fresh ones).
struct RouteState<'c, 'a> {
    layout: Layout,
    wires: HashMap<usize, Value<'c, 'a>>,
    tracker: WireTracker,
    next_phys: usize,
}

impl<'c, 'a> RouteState<'c, 'a> {
    fn new(num_qubits: usize) -> Self {
        Self {
            layout: Layout::new(num_qubits),
            wires: HashMap::new(),
            tracker: WireTracker::new(),
            next_phys: 0,
        }
    }
}

fn route_block<'c, 'a>(
    context: &'c Context,
    target: &FixedTarget,
    cost: SabreCost,
    block: melior::ir::BlockRef<'c, 'a>,
    state: &mut RouteState<'c, 'a>,
) {
    let mut op = block.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN || name == quantum_dynamic::op::YIELD {
            // A qubit that is never a gate operand within this block (a pure
            // pass-through, e.g. an unused register threaded straight to the
            // return) still has a `wires` entry once `recurse_region` (or a
            // swap that displaced it as a bystander) has touched it — but
            // nothing else in this loop resyncs a value that's *only* ever
            // consumed by the terminator. Without this, the terminator keeps
            // referencing the pre-swap value, silently returning/measuring
            // the wrong physical qubit's state.
            let raw_operands = qubit_wiring::qubit_operands(current);
            let raw_roots = state.tracker.roots_for_operands(current);
            let synced: Vec<Value<'c, 'a>> = raw_roots
                .iter()
                .zip(raw_operands.iter())
                .map(|(root, value)| state.wires.get(root).copied().unwrap_or(*value))
                .collect();
            set_qubit_operands(current, &synced);
            break;
        }
        if name == quantum_dynamic::op::UNITARY_REGION {
            recurse_region(context, target, cost, current, 0, state);
            continue;
        }
        if name == quantum_dynamic::op::IF {
            recurse_region(context, target, cost, current, 0, state);
            recurse_region(context, target, cost, current, 1, state);
            continue;
        }
        if name != quantum_circ::op::GATE {
            continue;
        }

        // This gate's operands, as they stand in the IR, may be stale: an
        // earlier 2-qubit gate's routing can have swapped a *different*
        // ("bystander") logical qubit's physical position while walking past
        // it, which produces a fresh SSA value for that logical qubit (see
        // `route_two_qubit`) without touching gates further down the block
        // that still reference the pre-swap value. Resync every qubit operand
        // to the authoritative `wires` entry for its root before using it —
        // otherwise this gate would silently operate on a value the inserted
        // SWAP has already consumed, corrupting the circuit's semantics
        // without tripping the linearity verifier (both are still each used
        // exactly once — just the wrong one, at the wrong point in time).
        let raw_operands = qubit_wiring::qubit_operands(current);
        let raw_roots = state.tracker.roots_for_operands(current);
        let synced: Vec<Value<'c, 'a>> = raw_roots
            .iter()
            .zip(raw_operands.iter())
            .map(|(root, value)| state.wires.get(root).copied().unwrap_or(*value))
            .collect();
        set_qubit_operands(current, &synced);

        let qubits: Vec<(usize, Value<'c, 'a>)> = raw_roots.into_iter().zip(synced).collect();

        for (logical, _) in &qubits {
            if !state.layout.mapping.contains_key(logical) {
                if let Err(error) = state.layout.assign(*logical, state.next_phys) {
                    eprintln!("sabre-routing: {error}");
                    return;
                }
                state.next_phys += 1;
            }
        }
        for (logical, value) in &qubits {
            state.wires.insert(*logical, *value);
        }

        if qubits.len() == 2 {
            let la = qubits[0].0;
            let lb = qubits[1].0;
            if let Err(error) = route_two_qubit(
                context,
                target,
                cost,
                &mut state.layout,
                block,
                current,
                la,
                lb,
                &mut state.wires,
                &mut state.tracker,
            ) {
                eprintln!("sabre-routing: {error}");
            }
        }

        if let Some((logical, _)) = qubits.first()
            && let Ok(phys) = state.layout.phys(*logical)
        {
            set_i32_attr(context, current, "phys_qubit", phys as i32);
        }
        state.tracker.observe_operation(current);
        // `wires[logical]` must track each root's latest live value — this
        // gate's own *result*, not the operand it just consumed. Without
        // this, the next gate on the same logical qubit would resync (via
        // the staleness check above) back to a value this gate has already
        // consumed, double-using it and silently corrupting the circuit.
        for (index, (logical, _)) in qubits.iter().enumerate() {
            if let Ok(result) = current.result(index) {
                state.wires.insert(*logical, Value::from(result));
            }
        }
    }
}

/// Recurses into region `region_index` of a `quantum.dynamic.unitary_region`
/// or `quantum.dynamic.if` op, aliasing the region's block arguments to the
/// *caller's* already-established logical roots for the op's qubit operands
/// (rather than the fresh per-block roots `WireTracker::seed_block_args` would
/// assign) so physical qubit identity survives the boundary. After the region
/// is processed, the op's own qubit results are aliased back to those same
/// roots so the surrounding block sees a continuous wire.
fn recurse_region<'c, 'a>(
    context: &'c Context,
    target: &FixedTarget,
    cost: SabreCost,
    op: OperationRef<'c, 'a>,
    region_index: usize,
    state: &mut RouteState<'c, 'a>,
) {
    let operand_roots = state.tracker.roots_for_operands(op);
    for root in &operand_roots {
        if !state.layout.mapping.contains_key(root) {
            if let Err(error) = state.layout.assign(*root, state.next_phys) {
                eprintln!("sabre-routing: {error}");
                return;
            }
            state.next_phys += 1;
        }
    }
    let Ok(region) = op.region(region_index) else {
        return;
    };
    let Some(inner_block) = region.first_block() else {
        return;
    };
    for (index, root) in operand_roots.iter().enumerate() {
        if let Ok(argument) = inner_block.argument(index) {
            let value = Value::from(argument);
            state.tracker.alias(value, *root);
            state.wires.insert(*root, value);
        }
    }
    route_block(context, target, cost, inner_block, state);
    for (result, root) in qubit_wiring::qubit_results(op)
        .into_iter()
        .zip(operand_roots.iter())
    {
        state.tracker.alias(result, *root);
        state.wires.insert(*root, result);
    }
}

fn route_module<'c, 'a>(
    context: &'c Context,
    target: &FixedTarget,
    cost: SabreCost,
    module: OperationRef<'c, 'a>,
) {
    let Some(body) = module
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return;
    };

    // One shared `RouteState` for the module's own top-level block (the real,
    // executed program after lowering — see `native_gate_decomp`'s
    // `decompose_block` doc comment for why this must be walked directly, not
    // just each named `quantum.circ.func`).
    let mut top_level_state = RouteState::new(target.num_qubits);
    top_level_state.tracker.seed_block_args(&body);
    route_block(context, target, cost, body, &mut top_level_state);

    // Each named `quantum.circ.func` is an independent circuit (its own qubit
    // register), so it gets a fresh `RouteState`. Post-inlining these are dead
    // code for `main`'s callees, but standalone `quantum.circ.func`-only
    // modules (e.g. this pass's own lit tests) rely on this path.
    let mut op = body.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        if op_name(&current) != quantum_circ::op::FUNC {
            continue;
        }
        let Ok(region) = current.region(0) else {
            continue;
        };
        let Some(block) = region.first_block() else {
            continue;
        };
        let mut state = RouteState::new(target.num_qubits);
        state.tracker.seed_block_args(&block);
        route_block(context, target, cost, block, &mut state);
    }
}

/// Runs SABRE routing on `module`.
pub fn run_on_module<'c>(
    context: &'c Context,
    target: &BackendTarget,
    cost: SabreCost,
    module: &melior::ir::Module<'c>,
) {
    let Some(target) = target.fixed_target() else {
        return;
    };
    route_module(context, target, cost, module.as_operation());
}

#[repr(align(8))]
struct PassId;

static SABRE_ROUTING_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct SabreRouting {
    context: usize,
    target: Arc<BackendTarget>,
    cost: SabreCost,
}

impl SabreRouting {
    fn new(target: BackendTarget, cost: SabreCost) -> Self {
        Self {
            context: 0,
            target: Arc::new(target),
            cost,
        }
    }
}

impl<'c> RunExternalPass<'c> for SabreRouting {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        if self.context == 0 {
            pass.signal_failure();
            return;
        }
        let Some(target) = self.target.fixed_target() else {
            return;
        };
        let context = unsafe { &*(self.context as *const Context) };
        route_module(context, target, self.cost, operation);
    }
}

/// Creates the SABRE routing pass.
pub fn create_pass(target: BackendTarget, cost: SabreCost) -> Pass {
    create_external(
        SabreRouting::new(target, cost),
        TypeId::create(&SABRE_ROUTING_PASS_ID),
        "sabre-routing",
        "sabre-routing",
        "Route quantum.circ gates onto BackendTarget topology with SWAP insertion",
        "",
        &[],
    )
}
