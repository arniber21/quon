//! Circuit metrics collection from lowered MLIR modules.
//!
//! Walks the executed top-level program path (post-`monadic_lowering`) with
//! [`crate::dynamic_walk`], the same shared walker
//! [`crate::passes::depth_scheduling`] uses. For programs with
//! `quantum.dynamic.if`, **both** branches are counted (conservative upper
//! bound). Qubit identity is derived from the canonical SSA wiring channel
//! (WireTracker roots); the `phys_qubit` attr is a derived annotation
//! (ADR-0034, issue #316).

use melior::ir::attribute::{IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::{Module, OperationRef, RegionLike};

use crate::dialect::quantum_circ;
use crate::dynamic_walk::{self, DynamicVisitor};
use backend::target::BackendTarget;

/// Raw circuit metrics extracted from IR.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CircuitMetricsRaw {
    pub depth: u64,
    pub gate_count: u64,
    pub t_count: u64,
    pub qubit_count: u64,
    pub swap_count: u64,
    pub depth_bound: Option<String>,
}

fn read_string_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    key: &str,
) -> Option<String> {
    let value = operation.attribute(key).ok()?;
    StringAttribute::try_from(value)
        .ok()
        .map(|string| string.value().to_string())
}

fn read_i64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<i64> {
    let value = operation.attribute(key).ok()?;
    IntegerAttribute::try_from(value)
        .ok()
        .map(|integer| integer.value())
}

fn normalize_gate_name(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

fn is_t_gate(name: &str) -> bool {
    matches!(
        normalize_gate_name(name).as_str(),
        "t" | "tdg" | "t†" | "t_dg"
    )
}

fn is_swap_gate(name: &str) -> bool {
    matches!(normalize_gate_name(name).as_str(), "swap")
}

struct GateVisit {
    gate_name: String,
    schedule_time: Option<i64>,
    phys_qubits: Vec<i32>,
}

#[derive(Default)]
struct MetricsVisitor {
    visits: Vec<GateVisit>,
}

impl<'c, 'a> DynamicVisitor<'c, 'a> for MetricsVisitor {
    fn gate(&mut self, op: OperationRef<'c, 'a>, qubit_roots: &[usize]) {
        let gate_name = read_string_attr(&op, quantum_circ::attr::GATE_NAME).unwrap_or_default();
        let schedule_time = read_i64_attr(&op, "schedule_time");
        let phys_qubits = dynamic_walk::resolve_phys_qubits(&op, qubit_roots);
        self.visits.push(GateVisit {
            gate_name,
            schedule_time,
            phys_qubits,
        });
    }
}

fn aggregate_visits(visits: &[GateVisit], count_t: bool) -> CircuitMetricsRaw {
    let mut gate_count = 0u64;
    let mut t_count = 0u64;
    let mut swap_count = 0u64;
    let mut max_schedule = None::<i64>;
    let mut max_phys = None::<i32>;

    for visit in visits {
        gate_count += 1;
        if count_t && is_t_gate(&visit.gate_name) {
            t_count += 1;
        }
        if is_swap_gate(&visit.gate_name) {
            swap_count += 1;
        }
        if let Some(time) = visit.schedule_time {
            max_schedule = Some(max_schedule.map_or(time, |m| m.max(time)));
        }
        for q in &visit.phys_qubits {
            max_phys = Some(max_phys.map_or(*q, |m| m.max(*q)));
        }
    }

    let depth = max_schedule.map(|t| (t + 1) as u64).unwrap_or(0);
    let qubit_count = max_phys.map(|q| (q + 1) as u64).unwrap_or(0);

    CircuitMetricsRaw {
        depth,
        gate_count,
        t_count,
        swap_count,
        qubit_count,
        depth_bound: None,
    }
}

fn visits_from_module(module: &Module<'_>) -> Vec<GateVisit> {
    let Some(body) = module
        .as_operation()
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return Vec::new();
    };

    let mut visitor = MetricsVisitor::default();
    dynamic_walk::walk_block(body, &mut visitor);
    visitor.visits
}

/// Collect circuit metrics from `module` after the full physical pipeline.
pub fn collect_module_metrics(module: &Module<'_>, _target: &BackendTarget) -> CircuitMetricsRaw {
    let visits = visits_from_module(module);
    let mut metrics = aggregate_visits(&visits, true);
    // Symbolic depth bounds live on dead `quantum.circ.func` wrappers; the
    // executed program after monadic lowering has no meaningful bound.
    metrics.depth_bound = None;
    metrics
}

/// Count T gates on a module snapshot taken after routing, before the second
/// native gate decomposition pass.
pub fn count_t_gates(module: &Module<'_>) -> u64 {
    let visits = visits_from_module(module);
    visits.iter().filter(|v| is_t_gate(&v.gate_name)).count() as u64
}
