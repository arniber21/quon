//! Depth-optimal scheduling pass (issue #26, SPEC §7.4).
//!
//! ASAP/ALAP scheduling of routed circuits. Qubit identity for the dependency
//! graph is derived from the **canonical SSA wiring channel** (WireTracker
//! roots via [`crate::dynamic_walk::resolve_phys_qubits`]), not from the
//! `phys_qubit` attr — which is a derived annotation (ADR-0034, issue #316).

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

use crate::dialect::quantum_circ;
use crate::dynamic_walk::{self, DynamicVisitor};

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

#[derive(Default)]
struct SchedulingVisitor<'c, 'a> {
    steps: Vec<GateStep<'c, 'a>>,
}

impl<'c, 'a> DynamicVisitor<'c, 'a> for SchedulingVisitor<'c, 'a> {
    fn gate(&mut self, op: OperationRef<'c, 'a>, qubit_roots: &[usize]) {
        let phys_qubits = dynamic_walk::resolve_phys_qubits(&op, qubit_roots);
        self.steps.push(GateStep {
            op,
            phys_qubits,
            barrier: false,
        });
    }

    fn barrier(&mut self, op: OperationRef<'c, 'a>, _qubit_roots: &[usize]) {
        self.steps.push(GateStep {
            op,
            phys_qubits: vec![],
            barrier: true,
        });
    }
}

/// Collects every gate/barrier reachable from `block`, in program order, via
/// [`dynamic_walk::walk_block`] — which recurses into nested
/// `quantum.dynamic.unitary_region` and `quantum.dynamic.if` bodies so a
/// qubit's dependency chain stays continuous across those boundaries.
fn collect_gates<'c, 'a>(block: melior::ir::BlockRef<'c, 'a>) -> Vec<GateStep<'c, 'a>> {
    let mut visitor = SchedulingVisitor::default();
    dynamic_walk::walk_block(block, &mut visitor);
    visitor.steps
}

fn schedule_segment<'c, 'a>(
    context: &'c Context,
    gates: &[GateStep<'c, 'a>],
    mode: ScheduleMode,
    offset: i64,
) -> i64 {
    if gates.is_empty() {
        return 0;
    }
    let n = gates.len();
    let mut preds: Vec<Vec<usize>> = vec![vec![]; n];
    let mut succs: Vec<Vec<usize>> = vec![vec![]; n];
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
    for (index, gate_preds) in preds.iter().enumerate() {
        for pred in gate_preds {
            succs[*pred].push(index);
        }
    }

    let mut times = vec![0i64; n];
    let mut asap = vec![0i64; n];
    for index in 0..n {
        if gates[index].barrier {
            continue;
        }
        let earliest = preds[index]
            .iter()
            .map(|pred| times[*pred] + 1)
            .max()
            .unwrap_or(0);
        asap[index] = earliest;
        times[index] = earliest;
    }

    if mode == ScheduleMode::Alap {
        let max_time = *times.iter().max().unwrap_or(&0);
        times.fill(max_time);
        for index in (0..n).rev() {
            if gates[index].barrier {
                continue;
            }
            let latest = succs[index]
                .iter()
                .map(|succ| times[*succ] - 1)
                .min()
                .unwrap_or(max_time);
            times[index] = latest.max(asap[index]);
        }
    }

    for (index, gate) in gates.iter().enumerate() {
        if !gate.barrier {
            set_schedule_time(context, gate.op, times[index] + offset);
        }
    }
    *times.iter().max().unwrap_or(&0) + 1
}

fn select_mode(target: &backend::target::FixedTarget, circuit_depth: i64) -> ScheduleMode {
    let depth_time = f64::from(circuit_depth as u32) * GATE_TIME_US;
    for t1 in target.noise.t1_us.values() {
        if *t1 < depth_time {
            return ScheduleMode::Alap;
        }
    }
    ScheduleMode::Asap
}

/// Splits a flat, program-order gate list on barriers and ASAP/ALAP-schedules
/// each barrier-free segment in sequence.
fn schedule_gates<'c, 'a>(
    context: &'c Context,
    target: &backend::target::FixedTarget,
    gates: Vec<GateStep<'c, 'a>>,
) {
    let mut segments: Vec<Vec<GateStep<'c, 'a>>> = vec![vec![]];
    for gate in gates {
        if gate.barrier {
            segments.push(vec![]);
            continue;
        }
        if let Some(segment) = segments.last_mut() {
            segment.push(gate);
        }
    }

    let mut total_depth = 0i64;
    for segment in &segments {
        if segment.is_empty() {
            continue;
        }
        let mode = select_mode(target, total_depth);
        total_depth += schedule_segment(context, segment, mode, total_depth);
    }
}

fn schedule_module<'c, 'a>(
    context: &'c Context,
    target: &backend::target::FixedTarget,
    module: OperationRef<'c, 'a>,
) {
    let Some(body) = module
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return;
    };

    // The module's own top-level block is the real, executed program after
    // `lowering` (see `native_gate_decomp::decompose_block`'s doc
    // comment) — a `quantum.circ.func` may no longer wrap it at all.
    schedule_gates(context, target, collect_gates(body));

    // Named `quantum.circ.func`s are dead code post-inlining for `main`'s
    // callees, but standalone `quantum.circ.func`-only modules (this pass's
    // own lit tests) rely on this path, each with its own qubit register.
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
        schedule_gates(context, target, collect_gates(block));
    }
}

/// Runs depth scheduling on `module`.
pub fn run_on_module<'c>(
    context: &'c Context,
    target: &BackendTarget,
    module: &melior::ir::Module<'c>,
) {
    let Some(target) = target.fixed_target() else {
        return;
    };
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
        let Some(target) = self.target.fixed_target() else {
            return;
        };
        let context = unsafe { &*(self.context as *const Context) };
        schedule_module(context, target, operation);
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
