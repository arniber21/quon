//! Circuit metrics collection from lowered MLIR modules.
//!
//! Walks the executed top-level program path (post-`monadic_lowering`), reusing
//! the same gate recursion as [`crate::passes::depth_scheduling`]. For programs
//! with `quantum.dynamic.if`, **both** branches are counted (conservative upper
//! bound).

use melior::ir::attribute::{IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, Module, OperationRef, RegionLike, Value};

use crate::dialect::{quantum_circ, quantum_dynamic};
use crate::passes::qubit_wiring::{self, WireTracker};
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

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
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

struct GateVisit<'c, 'a> {
    op: OperationRef<'c, 'a>,
    gate_name: String,
    schedule_time: Option<i64>,
    phys_qubits: Vec<i32>,
    is_gate: bool,
}

fn collect_gate_visits<'c, 'a>(
    block: melior::ir::BlockRef<'c, 'a>,
    tracker: &mut WireTracker,
    visits: &mut Vec<GateVisit<'c, 'a>>,
) {
    let mut op = block.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN || name == quantum_dynamic::op::YIELD {
            break;
        }
        if name == quantum_dynamic::op::BARRIER {
            tracker.observe_operation(current);
            continue;
        }
        if name == quantum_dynamic::op::UNITARY_REGION {
            recurse_region(current, 0, tracker, visits);
            continue;
        }
        if name == quantum_dynamic::op::IF {
            recurse_region(current, 0, tracker, visits);
            recurse_region(current, 1, tracker, visits);
            continue;
        }
        if name == quantum_dynamic::op::MEASURE || name == quantum_dynamic::op::RESET {
            tracker.observe_operation(current);
            continue;
        }
        if name != quantum_circ::op::GATE {
            tracker.observe_operation(current);
            continue;
        }
        let gate_name =
            read_string_attr(&current, quantum_circ::attr::GATE_NAME).unwrap_or_default();
        let schedule_time = read_i64_attr(&current, "schedule_time");
        let roots = tracker.roots_for_operands(current);
        let mut phys = if roots.is_empty() {
            read_i64_attr(&current, "phys_qubit")
                .map(|value| vec![value as i32])
                .unwrap_or_default()
        } else {
            roots.into_iter().map(|root| root as i32).collect()
        };
        if let Some(attr_phys) = read_i64_attr(&current, "phys_qubit")
            && !phys.contains(&(attr_phys as i32))
        {
            phys.push(attr_phys as i32);
        }
        visits.push(GateVisit {
            op: current,
            gate_name,
            schedule_time,
            phys_qubits: phys,
            is_gate: true,
        });
        tracker.observe_operation(current);
    }
}

fn recurse_region<'c, 'a>(
    op: OperationRef<'c, 'a>,
    region_index: usize,
    tracker: &mut WireTracker,
    visits: &mut Vec<GateVisit<'c, 'a>>,
) {
    let operand_roots = tracker.roots_for_operands(op);
    let Ok(region) = op.region(region_index) else {
        return;
    };
    let Some(inner_block) = region.first_block() else {
        return;
    };
    for (index, root) in operand_roots.iter().enumerate() {
        if let Ok(argument) = inner_block.argument(index) {
            tracker.alias(Value::from(argument), *root);
        }
    }
    collect_gate_visits(inner_block, tracker, visits);
    for (result, root) in qubit_wiring::qubit_results(op)
        .into_iter()
        .zip(operand_roots.iter())
    {
        tracker.alias(result, *root);
    }
}

fn aggregate_visits(visits: &[GateVisit<'_, '_>], count_t: bool) -> CircuitMetricsRaw {
    let mut gate_count = 0u64;
    let mut t_count = 0u64;
    let mut swap_count = 0u64;
    let mut max_schedule = None::<i64>;
    let mut max_phys = None::<i32>;

    for visit in visits {
        if !visit.is_gate {
            continue;
        }
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

fn visits_from_module<'c>(module: &'c Module<'c>) -> Vec<GateVisit<'c, 'c>> {
    let mut visits = Vec::new();
    let Some(body) = module
        .as_operation()
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return visits;
    };

    let mut tracker = WireTracker::new();
    tracker.seed_block_args(&body);
    collect_gate_visits(body, &mut tracker, &mut visits);
    visits
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
    visits
        .iter()
        .filter(|v| v.is_gate && is_t_gate(&v.gate_name))
        .count() as u64
}
