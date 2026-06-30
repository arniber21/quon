//! Depth-optimal scheduling pass (issue #26, SPEC §7.4).
//!
//! ASAP/ALAP scheduling of routed circuits with `phys_qubit` attributes.

use std::collections::HashMap;
use std::sync::Arc;

use backend::target::BackendTarget;
use melior::ir::attribute::IntegerAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{AttributeLike, BlockLike, OperationRef, RegionLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, StringRef};
use mlir_sys::mlirOperationSetAttributeByName;

use crate::dialect::{quantum_circ, quantum_dynamic};

const GATE_TIME_US: f64 = 0.1;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScheduleMode {
    Asap,
    Alap,
}

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn read_i32_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<i32> {
    let value = operation.attribute(key).ok()?;
    IntegerAttribute::try_from(value)
        .ok()
        .map(|integer| integer.value() as i32)
}

fn set_schedule_time<'c>(context: &'c Context, op: OperationRef<'c, '_>, time: i64) {
    let attribute: melior::ir::Attribute<'_> = IntegerAttribute::new(
        melior::ir::r#type::IntegerType::new(context, 64).into(),
        time,
    )
    .into();
    unsafe {
        mlirOperationSetAttributeByName(
            op.to_raw(),
            StringRef::new("schedule_time").to_raw(),
            attribute.to_raw(),
        );
    }
}

struct GateStep<'c, 'a> {
    op: OperationRef<'c, 'a>,
    phys_qubits: Vec<i32>,
    barrier: bool,
}

fn collect_gates<'c, 'a>(block: melior::ir::BlockRef<'c, 'a>) -> Vec<GateStep<'c, 'a>> {
    let mut steps = Vec::new();
    let mut op = block.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN {
            break;
        }
        if name == quantum_dynamic::op::BARRIER {
            steps.push(GateStep {
                op: current,
                phys_qubits: vec![],
                barrier: true,
            });
            continue;
        }
        if name != quantum_circ::op::GATE {
            continue;
        }
        let phys = read_i32_attr(&current, "phys_qubit")
            .map(|value| vec![value])
            .unwrap_or_default();
        steps.push(GateStep {
            op: current,
            phys_qubits: phys,
            barrier: false,
        });
    }
    steps
}

fn schedule_segment<'c, 'a>(
    context: &'c Context,
    gates: &[GateStep<'c, 'a>],
    mode: ScheduleMode,
) -> i64 {
    if gates.is_empty() {
        return 0;
    }
    let n = gates.len();
    let mut preds: Vec<Vec<usize>> = vec![vec![]; n];
    let mut last_on_qubit: HashMap<i32, usize> = HashMap::new();

    for (index, gate) in gates.iter().enumerate() {
        if gate.barrier {
            last_on_qubit.clear();
            continue;
        }
        for q in &gate.phys_qubits {
            if let Some(pred) = last_on_qubit.get(q) {
                preds[index].push(*pred);
            }
        }
        for q in &gate.phys_qubits {
            last_on_qubit.insert(*q, index);
        }
    }

    let mut times = vec![0i64; n];
    for index in 0..n {
        if gates[index].barrier {
            continue;
        }
        let earliest = preds[index]
            .iter()
            .map(|pred| times[*pred] + 1)
            .max()
            .unwrap_or(0);
        times[index] = earliest;
    }

    if mode == ScheduleMode::Alap {
        let max_time = *times.iter().max().unwrap_or(&0);
        for index in (0..n).rev() {
            if gates[index].barrier {
                times[index] = max_time;
                continue;
            }
            let succ_constraint = max_time;
            let pred_constraint = preds[index]
                .iter()
                .map(|pred| times[*pred] + 1)
                .max()
                .unwrap_or(0);
            times[index] = succ_constraint.max(pred_constraint);
        }
    }

    for (index, gate) in gates.iter().enumerate() {
        if !gate.barrier {
            set_schedule_time(context, gate.op, times[index]);
        }
    }
    *times.iter().max().unwrap_or(&0) + 1
}

fn select_mode(target: &BackendTarget, circuit_depth: i64) -> ScheduleMode {
    let depth_time = f64::from(circuit_depth as u32) * GATE_TIME_US;
    for t1 in target.noise.t1_us.values() {
        if *t1 < depth_time {
            return ScheduleMode::Alap;
        }
    }
    ScheduleMode::Asap
}

fn schedule_module<'c, 'a>(context: &'c Context, target: &BackendTarget, module: OperationRef<'c, 'a>) {
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

        let gates = collect_gates(block);
        let mut segments: Vec<Vec<GateStep<'c, 'a>>> = vec![vec![]];
        for gate in gates {
            if gate.barrier {
                segments.push(vec![]);
                continue;
            }
            segments.last_mut().expect("segment").push(gate);
        }

        let mut total_depth = 0i64;
        for segment in &segments {
            if segment.is_empty() {
                continue;
            }
            let mode = select_mode(target, total_depth);
            total_depth += schedule_segment(context, segment, mode);
        }
    }
}

/// Runs depth scheduling on `module`.
pub fn run_on_module<'c>(context: &'c Context, target: &BackendTarget, module: &melior::ir::Module<'c>) {
    schedule_module(context, target, module.as_operation());
}

#[repr(align(8))]
struct PassId;

static DEPTH_SCHEDULING_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct DepthScheduling {
    context: usize,
    target: Arc<BackendTarget>,
}

impl DepthScheduling {
    fn new(target: BackendTarget) -> Self {
        Self {
            context: 0,
            target: Arc::new(target),
        }
    }
}

impl<'c> RunExternalPass<'c> for DepthScheduling {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        if self.context == 0 {
            pass.signal_failure();
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        schedule_module(context, &self.target, operation);
    }
}

/// Creates the depth scheduling pass.
pub fn create_pass(target: BackendTarget) -> Pass {
    create_external(
        DepthScheduling::new(target),
        TypeId::create(&DEPTH_SCHEDULING_PASS_ID),
        "depth-scheduling",
        "depth-scheduling",
        "Schedule quantum.circ gates with ASAP/ALAP depth optimization",
        "",
        &[],
    )
}
