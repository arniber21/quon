//! SABRE routing pass (issue #25, SPEC §7.4).
//!
//! Maps logical qubits to physical indices and inserts SWAP gates to satisfy
//! connectivity constraints on the [`BackendTarget`] topology.

use std::collections::HashMap;
use std::sync::Arc;

use backend::target::BackendTarget;
use melior::StringRef;
use melior::ir::attribute::IntegerAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{AttributeLike, BlockLike, Location, OperationRef, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef};
use mlir_sys::{mlirOperationSetAttributeByName, mlirOperationSetOperand};
use thiserror::Error;

use crate::dialect::quantum_circ;
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
    target: &BackendTarget,
    cost: SabreCost,
    layout: &mut Layout,
    block: melior::ir::BlockRef<'c, 'a>,
    gate: OperationRef<'c, 'a>,
    logical_a: usize,
    logical_b: usize,
    wires: &mut HashMap<usize, Value<'c, 'a>>,
) -> Result<(), RouteError> {
    let location = gate.location();
    let mut p_a = layout.phys(logical_a)?;
    let mut p_b = layout.phys(logical_b)?;

    while target.topology.dist(p_a, p_b) > 1 {
        let (u, v) = best_swap(target, cost, layout, p_a, p_b)?;

        let logical_u = layout.logical_at(u)?;
        let logical_v = layout.logical_at(v)?;
        let wire_u = wires[&logical_u];
        let wire_v = wires[&logical_v];
        let (out_u, out_v) = append_swap(context, block, gate, wire_u, wire_v, location)?;
        let (new_u, new_v) = layout.swap_phys(u, v)?;
        wires.insert(new_u, out_u);
        wires.insert(new_v, out_v);
        p_a = layout.phys(logical_a)?;
        p_b = layout.phys(logical_b)?;
    }

    set_qubit_operands(gate, &[wires[&logical_a], wires[&logical_b]]);
    Ok(())
}

fn best_swap(
    target: &BackendTarget,
    cost: SabreCost,
    layout: &Layout,
    p_a: usize,
    p_b: usize,
) -> Result<(usize, usize), RouteError> {
    let mut best: Option<((usize, usize), f64)> = None;
    for &(u, v) in &target.topology.edges {
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
        let distance = target.topology.dist(swapped_a, swapped_b) as f64;
        let score = cost.alpha * distance + cost.gamma * noise_penalty(target, u, v);
        if best.is_none_or(|(_, best_score)| score < best_score) {
            best = Some(((u, v), score));
        }
    }
    best.map(|(edge, _)| edge).ok_or_else(|| RouteError::Build {
        op: quantum_circ::op::GATE,
        message: "no legal swap candidate".to_string(),
    })
}

fn noise_penalty(target: &BackendTarget, a: usize, b: usize) -> f64 {
    target
        .noise
        .two_qubit_fidelity
        .get(&("cx".to_string(), a, b))
        .copied()
        .map(|f| -f.ln())
        .unwrap_or(0.0)
}

fn route_block<'c, 'a>(
    context: &'c Context,
    target: &BackendTarget,
    cost: SabreCost,
    block: melior::ir::BlockRef<'c, 'a>,
) {
    let mut layout = Layout::new(target.num_qubits);
    let mut next_phys = 0usize;
    let mut wires: HashMap<usize, Value<'c, 'a>> = HashMap::new();
    let mut tracker = WireTracker::new();
    tracker.seed_block_args(&block);

    let mut op = block.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        if op_name(&current) == quantum_circ::op::RETURN {
            break;
        }
        if op_name(&current) != quantum_circ::op::GATE {
            continue;
        }

        let operands = qubit_wiring::qubit_operands(current);
        let roots = tracker.roots_for_operands(current);
        let qubits: Vec<(usize, Value<'c, 'a>)> = roots.into_iter().zip(operands).collect();

        for (logical, _) in &qubits {
            if !layout.mapping.contains_key(logical) {
                if let Err(error) = layout.assign(*logical, next_phys) {
                    eprintln!("sabre-routing: {error}");
                    return;
                }
                next_phys += 1;
            }
        }
        for (logical, value) in &qubits {
            wires.insert(*logical, *value);
        }

        if qubits.len() == 2 {
            let la = qubits[0].0;
            let lb = qubits[1].0;
            if let Err(error) = route_two_qubit(
                context,
                target,
                cost,
                &mut layout,
                block,
                current,
                la,
                lb,
                &mut wires,
            ) {
                eprintln!("sabre-routing: {error}");
            }
        }

        if let Some((logical, _)) = qubits.first()
            && let Ok(phys) = layout.phys(*logical)
        {
            set_i32_attr(context, current, "phys_qubit", phys as i32);
        }
        tracker.observe_operation(current);
    }
}

fn route_module<'c, 'a>(
    context: &'c Context,
    target: &BackendTarget,
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
        route_block(context, target, cost, block);
    }
}

/// Runs SABRE routing on `module`.
pub fn run_on_module<'c>(
    context: &'c Context,
    target: &BackendTarget,
    cost: SabreCost,
    module: &melior::ir::Module<'c>,
) {
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
        let context = unsafe { &*(self.context as *const Context) };
        route_module(context, &self.target, self.cost, operation);
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
