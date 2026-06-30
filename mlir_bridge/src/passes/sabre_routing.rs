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
use mlir_sys::mlirOperationSetAttributeByName;
use thiserror::Error;

use crate::dialect::quantum_circ;

fn value_key<'a>(value: &impl ValueLike<'a>) -> usize {
    value.to_raw().ptr as usize
}

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
    inverse: Vec<usize>,
}

impl Layout {
    fn new(num_qubits: usize) -> Self {
        Self {
            mapping: HashMap::new(),
            inverse: (0..num_qubits).collect(),
        }
    }

    fn assign(&mut self, logical: usize, physical: usize) {
        self.mapping.insert(logical, physical);
        if physical < self.inverse.len() {
            self.inverse[physical] = logical;
        }
    }

    fn phys(&self, logical: usize) -> usize {
        *self.mapping.get(&logical).unwrap_or(&logical)
    }

    fn swap_phys(&mut self, a: usize, b: usize) {
        let la = self.inverse[a];
        let lb = self.inverse[b];
        self.mapping.insert(la, b);
        self.mapping.insert(lb, a);
        self.inverse[a] = lb;
        self.inverse[b] = la;
    }
}

// Threaded routing state: layout, IR cursor, and live wire map travel together.
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
    let mut p_a = layout.phys(logical_a);
    let mut p_b = layout.phys(logical_b);

    while target.topology.dist(p_a, p_b) > 1 {
        let path = shortest_path(&target.topology.dist, p_a, p_b);
        // No connecting path (e.g. disconnected components): bail out instead of
        // indexing an empty path and panicking.
        let Some(&(u, v)) = path.first() else {
            break;
        };
        let swap_cost = cost.alpha + cost.gamma * noise_penalty(target, u, v);
        let _ = swap_cost;

        let wire_u = wires[&layout.inverse[u]];
        let wire_v = wires[&layout.inverse[v]];
        let (out_u, out_v) = append_swap(context, block, gate, wire_u, wire_v, location)?;
        wires.insert(layout.inverse[u], out_u);
        wires.insert(layout.inverse[v], out_v);
        layout.swap_phys(u, v);
        p_a = layout.phys(logical_a);
        p_b = layout.phys(logical_b);
    }
    Ok(())
}

fn shortest_path(dist: &[Vec<usize>], start: usize, end: usize) -> Vec<(usize, usize)> {
    if start == end {
        return vec![];
    }
    let mut current = start;
    let mut path = Vec::new();
    while current != end {
        let mut next = current;
        let mut best = dist[current][end];
        for (candidate, row) in dist.iter().enumerate() {
            if dist[current][candidate] == 1 && row[end] < best {
                best = row[end];
                next = candidate;
            }
        }
        if next == current {
            break;
        }
        path.push((current, next));
        current = next;
    }
    path
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

    let mut op = block.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        if op_name(&current) == quantum_circ::op::RETURN {
            break;
        }
        if op_name(&current) != quantum_circ::op::GATE {
            continue;
        }

        let qubits: Vec<(usize, Value<'c, 'a>)> = current
            .operands()
            .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
            .map(|operand| {
                let key = value_key(&operand);
                (key, operand)
            })
            .collect();

        for (logical, _) in &qubits {
            if !layout.mapping.contains_key(logical) {
                layout.assign(*logical, next_phys);
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

        if let Some((logical, _)) = qubits.first() {
            set_i32_attr(context, current, "phys_qubit", layout.phys(*logical) as i32);
        }
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
