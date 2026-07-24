//! Classical region fusion pass (issues #23 / #97, SPEC §7.3).
//!
//! Fuses adjacent `quantum.dynamic.if` ops that act on disjoint qubit sets
//! with no data dependency between their `!bit` conditions into a single
//! boundary-crossing region, reducing the classical/quantum crossing count.
//!
//! Two cases:
//! - **Same condition** (#23): the ifs branch on the *same* bit, so the bodies
//!   merge into one `if` with parallel unitary regions.
//! - **Independent conditions** (#97): the ifs branch on *different*,
//!   non-dependent bits. A same-condition merge would lose the independent
//!   branching, so the fusion uses a **nested-if** shape — one outer `if` on
//!   the first condition holds the first body plus an inner `if` on the second
//!   condition holding the second body (the inner `if` is materialized in
//!   both outer branches to keep all four condition combinations reachable).
//!   Only the outer `if` crosses the classical/quantum boundary; the inner
//!   `if` lives inside the quantum region, so the crossing count drops from
//!   two to one. This is the simplest, most conservative fusion shape; a joint
//!   4-way dispatch was rejected as unnecessarily complex.

use std::collections::HashMap;

use melior::ir::attribute::{BoolAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{
    Block, BlockLike, Location, Operation, OperationRef, Region, RegionLike, Value, ValueLike,
};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter};
use thiserror::Error;

use crate::dialect::{quantum_circ, quantum_dynamic};
use crate::passes::qubit_wiring::{self, WireTracker};

#[derive(Debug, Error)]
pub enum FusionError {
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

fn read_bool_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<bool> {
    let value = operation.attribute(key).ok()?;
    BoolAttribute::try_from(value).ok().map(|b| b.value())
}

#[derive(Clone, Debug)]
struct GateStep {
    name: String,
    targets: Vec<usize>,
    depth_contribution: i64,
    clifford: bool,
}

fn extract_gates_from_region<'c, 'a>(
    region: melior::ir::RegionRef<'c, 'a>,
) -> Option<Vec<GateStep>> {
    let mut gates = Vec::new();
    let Some(block) = region.first_block() else {
        return Some(gates);
    };
    let mut tracker = WireTracker::new();
    tracker.seed_block_args(&block);
    let mut op = block.first_operation();
    while let Some(inner) = op {
        let name = op_name(&inner);
        if name == quantum_circ::op::GATE {
            gates.push(GateStep {
                name: read_string_attr(&inner, quantum_circ::attr::GATE_NAME)
                    .unwrap_or_else(|| "?".to_string()),
                targets: tracker.roots_for_operands(inner),
                depth_contribution: read_i64_attr(&inner, quantum_circ::attr::DEPTH_CONTRIBUTION)
                    .unwrap_or(1),
                clifford: read_bool_attr(&inner, quantum_circ::attr::CLIFFORD).unwrap_or(false),
            });
        } else if name != quantum_dynamic::op::YIELD {
            return None;
        }
        tracker.observe_operation(inner);
        op = inner.next_in_block();
    }
    Some(gates)
}

fn qubit_operands<'c, 'a>(operation: OperationRef<'c, 'a>) -> Vec<Value<'c, 'a>> {
    operation
        .operands()
        .skip(1)
        .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
        .collect()
}

fn region_produces_bit<'c, 'a>(region: melior::ir::RegionRef<'c, 'a>, bit_key: usize) -> bool {
    let mut block = region.first_block();
    while let Some(current) = block {
        let mut op = current.first_operation();
        while let Some(inner) = op {
            if op_name(&inner) == quantum_dynamic::op::MEASURE
                && let Ok(result) = inner.result(0)
                && qubit_wiring::value_key(&result) == bit_key
            {
                return true;
            }
            op = inner.next_in_block();
        }
        block = current.next_in_region();
    }
    false
}

fn if_depends_on_other_bit<'c, 'a>(
    consumer: OperationRef<'c, 'a>,
    producer: OperationRef<'c, 'a>,
) -> bool {
    let Ok(condition) = consumer.operand(0) else {
        return false;
    };
    let bit_key = qubit_wiring::value_key(&condition);
    for region_index in 0..producer.region_count() {
        if let Ok(region) = producer.region(region_index)
            && region_produces_bit(region, bit_key)
        {
            return true;
        }
    }
    false
}

fn disjoint_qubits<'c, 'a>(left: OperationRef<'c, 'a>, right: OperationRef<'c, 'a>) -> bool {
    let Some(left_block) = left.block() else {
        return false;
    };
    let Some(right_block) = right.block() else {
        return false;
    };
    let left_keys = qubit_wiring::roots_before(&left_block, left);
    let right_keys = qubit_wiring::roots_before(&right_block, right);
    left_keys.iter().all(|left| !right_keys.contains(left))
}

fn same_condition<'c, 'a>(left: OperationRef<'c, 'a>, right: OperationRef<'c, 'a>) -> bool {
    let Ok(left_bit) = left.operand(0) else {
        return false;
    };
    let Ok(right_bit) = right.operand(0) else {
        return false;
    };
    qubit_wiring::value_key(&left_bit) == qubit_wiring::value_key(&right_bit)
}

fn build_branch_region<'c>(
    context: &'c Context,
    location: Location<'c>,
    left_gates: &[GateStep],
    right_gates: &[GateStep],
    left_count: usize,
    right_count: usize,
) -> Result<Region<'c>, FusionError> {
    let qubit = quantum_circ::qubit_type(context);
    let total = left_count + right_count;
    let block = Block::new(&vec![(qubit, location); total]);
    let mut wires: Vec<Value<'c, '_>> = Vec::with_capacity(total);
    for index in 0..total {
        let argument = block.argument(index).map_err(|_| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: format!("missing block argument #{index}"),
        })?;
        wires.push(Value::from(argument));
    }

    for gate in left_gates {
        let operands: Vec<Value<'c, '_>> = gate.targets.iter().map(|i| wires[*i]).collect();
        let op = block.append_operation(
            quantum_circ::gate(
                context,
                &gate.name,
                gate.depth_contribution,
                gate.clifford,
                &operands,
                location,
            )
            .map_err(|error| FusionError::Build {
                op: quantum_circ::op::GATE,
                message: error.to_string(),
            })?,
        );
        for (index, target) in gate.targets.iter().enumerate() {
            wires[*target] = Value::from(op.result(index).map_err(|_| FusionError::Build {
                op: quantum_circ::op::GATE,
                message: "missing gate result".to_string(),
            })?);
        }
    }
    for gate in right_gates {
        let targets: Vec<usize> = gate.targets.iter().map(|i| i + left_count).collect();
        let operands: Vec<Value<'c, '_>> = targets.iter().map(|i| wires[*i]).collect();
        let op = block.append_operation(
            quantum_circ::gate(
                context,
                &gate.name,
                gate.depth_contribution,
                gate.clifford,
                &operands,
                location,
            )
            .map_err(|error| FusionError::Build {
                op: quantum_circ::op::GATE,
                message: error.to_string(),
            })?,
        );
        for (index, target) in targets.iter().enumerate() {
            wires[*target] = Value::from(op.result(index).map_err(|_| FusionError::Build {
                op: quantum_circ::op::GATE,
                message: "missing gate result".to_string(),
            })?);
        }
    }

    block.append_operation(quantum_dynamic::r#yield(&wires, location).map_err(|error| {
        FusionError::Build {
            op: quantum_dynamic::op::YIELD,
            message: error.to_string(),
        }
    })?);
    let region = Region::new();
    region.append_block(block);
    Ok(region)
}
/// Builds one branch region of the outer fused `if` for the nested-if shape
/// (#97). The block threads `left_count + right_count` qubits: the first body's
/// gates run on the left wires, then an inner `if` on `condition2` runs the
/// second body on the right wires. The inner `if` lives inside this region, so
/// it does not cross the classical/quantum boundary — only the outer `if` does.
#[allow(clippy::too_many_arguments)]
fn build_nested_branch_region<'c, 'a>(
    context: &'c Context,
    location: Location<'c>,
    left_gates: &[GateStep],
    right_then_gates: &[GateStep],
    right_else_gates: &[GateStep],
    left_count: usize,
    right_count: usize,
    condition2: Value<'c, 'a>,
) -> Result<Region<'c>, FusionError> {
    let qubit = quantum_circ::qubit_type(context);
    let total = left_count + right_count;
    let block = Block::new(&vec![(qubit, location); total]);
    let mut wires: Vec<Value<'c, '_>> = Vec::with_capacity(total);
    for index in 0..total {
        let argument = block.argument(index).map_err(|_| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: format!("missing block argument #{index}"),
        })?;
        wires.push(Value::from(argument));
    }

    // First body: run on the left qubit wires (targets index into [0, left_count)).
    for gate in left_gates {
        let operands: Vec<Value<'c, '_>> = gate.targets.iter().map(|i| wires[*i]).collect();
        let op = block.append_operation(
            quantum_circ::gate(
                context,
                &gate.name,
                gate.depth_contribution,
                gate.clifford,
                &operands,
                location,
            )
            .map_err(|error| FusionError::Build {
                op: quantum_circ::op::GATE,
                message: error.to_string(),
            })?,
        );
        for (index, target) in gate.targets.iter().enumerate() {
            wires[*target] = Value::from(op.result(index).map_err(|_| FusionError::Build {
                op: quantum_circ::op::GATE,
                message: "missing gate result".to_string(),
            })?);
        }
    }

    // Second body: wrapped in an inner `if` on the second condition. This is
    // the nested-if design choice — the inner `if` stays inside the outer
    // boundary-crossing region, so it adds no new classical/quantum crossing.
    let inner_then = build_branch_region(context, location, right_then_gates, &[], right_count, 0)?;
    let inner_else = build_branch_region(context, location, right_else_gates, &[], right_count, 0)?;
    let right_operands: Vec<Value<'c, '_>> = wires[left_count..total].to_vec();
    let inner_if = quantum_dynamic::r#if(
        context,
        condition2,
        &right_operands,
        inner_then,
        inner_else,
        location,
    )
    .map_err(|error| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: error.to_string(),
    })?;
    let inner_ref = block.append_operation(inner_if);
    for index in 0..right_count {
        wires[left_count + index] =
            Value::from(inner_ref.result(index).map_err(|_| FusionError::Build {
                op: quantum_dynamic::op::IF,
                message: "missing inner if result".to_string(),
            })?);
    }

    block.append_operation(quantum_dynamic::r#yield(&wires, location).map_err(|error| {
        FusionError::Build {
            op: quantum_dynamic::op::YIELD,
            message: error.to_string(),
        }
    })?);
    let region = Region::new();
    region.append_block(block);
    Ok(region)
}

fn fuse_pair<'c, 'a>(
    context: &'c Context,
    body: melior::ir::BlockRef<'c, 'a>,
    first: OperationRef<'c, 'a>,
    second: OperationRef<'c, 'a>,
) -> Result<(), FusionError> {
    // Same-condition ifs merge into one `if` with parallel regions (#23);
    // independent-condition ifs fuse via the nested-if shape (#97).
    if same_condition(first, second) {
        fuse_pair_same(context, body, first, second)
    } else {
        fuse_pair_independent(context, body, first, second)
    }
}

/// Same-condition fusion (#23): both ifs branch on the *same* bit, so their
/// bodies merge into a single `if` with parallel unitary regions.
fn fuse_pair_same<'c, 'a>(
    context: &'c Context,
    body: melior::ir::BlockRef<'c, 'a>,
    first: OperationRef<'c, 'a>,
    second: OperationRef<'c, 'a>,
) -> Result<(), FusionError> {
    let location = first.location();
    let condition = first.operand(0).map_err(|_| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "missing condition".to_string(),
    })?;
    let left_qubits = qubit_operands(first);
    let right_qubits = qubit_operands(second);
    let left_count = left_qubits.len();
    let right_count = right_qubits.len();

    let then_left = extract_gates_from_region(first.region(0).map_err(|_| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "missing then region".to_string(),
    })?)
    .ok_or_else(|| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "unsupported non-gate op in branch".to_string(),
    })?;
    let else_left = extract_gates_from_region(first.region(1).map_err(|_| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "missing else region".to_string(),
    })?)
    .ok_or_else(|| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "unsupported non-gate op in branch".to_string(),
    })?;
    let then_right =
        extract_gates_from_region(second.region(0).map_err(|_| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: "missing then region".to_string(),
        })?)
        .ok_or_else(|| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: "unsupported non-gate op in branch".to_string(),
        })?;
    let else_right =
        extract_gates_from_region(second.region(1).map_err(|_| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: "missing else region".to_string(),
        })?)
        .ok_or_else(|| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: "unsupported non-gate op in branch".to_string(),
        })?;

    let then_region = build_branch_region(
        context,
        location,
        &then_left,
        &then_right,
        left_count,
        right_count,
    )?;
    let else_region = build_branch_region(
        context,
        location,
        &else_left,
        &else_right,
        left_count,
        right_count,
    )?;

    let mut operands = vec![condition];
    operands.extend(left_qubits.iter().copied());
    operands.extend(right_qubits.iter().copied());

    let fused = quantum_dynamic::r#if(
        context,
        condition,
        &operands[1..],
        then_region,
        else_region,
        location,
    )
    .map_err(|error| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: error.to_string(),
    })?;

    install_fused(
        context,
        body,
        first,
        second,
        fused,
        &left_qubits,
        &right_qubits,
    )
}

/// Independent-condition fusion (#97): the ifs branch on *different*,
/// non-dependent bits and act on disjoint qubits. Uses the **nested-if**
/// shape — a single outer `if` on the first condition wraps the first body and
/// an inner `if` on the second condition wrapping the second body. The inner
/// `if` is materialized in *both* outer branches so all four (b1, b2)
/// combinations stay reachable; only the outer `if` crosses the
/// classical/quantum boundary, dropping the crossing count from two to one.
fn fuse_pair_independent<'c, 'a>(
    context: &'c Context,
    body: melior::ir::BlockRef<'c, 'a>,
    first: OperationRef<'c, 'a>,
    second: OperationRef<'c, 'a>,
) -> Result<(), FusionError> {
    let location = first.location();
    let condition = first.operand(0).map_err(|_| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "missing condition".to_string(),
    })?;
    let condition2 = second.operand(0).map_err(|_| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "missing condition".to_string(),
    })?;
    let left_qubits = qubit_operands(first);
    let right_qubits = qubit_operands(second);
    let left_count = left_qubits.len();
    let right_count = right_qubits.len();

    let then_left = extract_gates_from_region(first.region(0).map_err(|_| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "missing then region".to_string(),
    })?)
    .ok_or_else(|| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "unsupported non-gate op in branch".to_string(),
    })?;
    let else_left = extract_gates_from_region(first.region(1).map_err(|_| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "missing else region".to_string(),
    })?)
    .ok_or_else(|| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "unsupported non-gate op in branch".to_string(),
    })?;
    let then_right =
        extract_gates_from_region(second.region(0).map_err(|_| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: "missing then region".to_string(),
        })?)
        .ok_or_else(|| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: "unsupported non-gate op in branch".to_string(),
        })?;
    let else_right =
        extract_gates_from_region(second.region(1).map_err(|_| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: "missing else region".to_string(),
        })?)
        .ok_or_else(|| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: "unsupported non-gate op in branch".to_string(),
        })?;

    let then_region = build_nested_branch_region(
        context,
        location,
        &then_left,
        &then_right,
        &else_right,
        left_count,
        right_count,
        condition2,
    )?;
    let else_region = build_nested_branch_region(
        context,
        location,
        &else_left,
        &then_right,
        &else_right,
        left_count,
        right_count,
        condition2,
    )?;

    let mut operands = vec![condition];
    operands.extend(left_qubits.iter().copied());
    operands.extend(right_qubits.iter().copied());

    let fused = quantum_dynamic::r#if(
        context,
        condition,
        &operands[1..],
        then_region,
        else_region,
        location,
    )
    .map_err(|error| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: error.to_string(),
    })?;

    install_fused(
        context,
        body,
        first,
        second,
        fused,
        &left_qubits,
        &right_qubits,
    )
}

/// Installs `fused` before `first` in `body`, rewrites every use of the two
/// originals' results to the fused op's results (left qubits map to the first
/// `left_count` results, right qubits to the remainder), then erases the
/// originals. Shared by both fusion shapes — only the fused region differs.
fn install_fused<'c, 'a>(
    context: &'c Context,
    body: melior::ir::BlockRef<'c, 'a>,
    first: OperationRef<'c, 'a>,
    second: OperationRef<'c, 'a>,
    fused: Operation<'c>,
    left_qubits: &[Value<'c, 'a>],
    right_qubits: &[Value<'c, 'a>],
) -> Result<(), FusionError> {
    let left_count = left_qubits.len();
    let fused_ref = body.insert_operation_before(first, fused);
    let mut output_map: HashMap<usize, Value<'c, 'a>> = HashMap::new();
    for (index, result) in fused_ref.results().enumerate() {
        output_map.insert(index, Value::from(result));
    }

    let rewriter = IrRewriter::new(context);
    let base = rewriter.as_rewriter_base();
    for (index, _qubit) in left_qubits.iter().enumerate() {
        if let Some(mapped) = output_map.get(&index)
            && let Ok(result) = first.result(index)
        {
            base.replace_all_uses_with(Value::from(result), *mapped);
        }
    }
    for (index, _qubit) in right_qubits.iter().enumerate() {
        if let Some(mapped) = output_map.get(&(index + left_count))
            && let Ok(result) = second.result(index)
        {
            base.replace_all_uses_with(Value::from(result), *mapped);
        }
    }

    base.erase_op(second);
    base.erase_op(first);
    Ok(())
}

fn can_fuse<'c, 'a>(first: OperationRef<'c, 'a>, second: OperationRef<'c, 'a>) -> bool {
    if op_name(&first) != quantum_dynamic::op::IF || op_name(&second) != quantum_dynamic::op::IF {
        return false;
    }
    if !disjoint_qubits(first, second) {
        return false;
    }
    // Decline when either condition bit is produced by the other's body — the
    // ifs are not independent and must stay ordered (#97 acceptance).
    if if_depends_on_other_bit(second, first) || if_depends_on_other_bit(first, second) {
        return false;
    }
    // The ifs are independent. Same-condition ifs merge into one `if` with
    // parallel regions (#23); different-condition ifs take the nested-if shape
    // (#97). `fuse_pair` dispatches on `same_condition`.
    true
}

fn fuse_module<'c, 'a>(context: &'c Context, module: OperationRef<'c, 'a>) {
    let Some(body) = module
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return;
    };

    loop {
        let mut fused = false;
        let mut prev_if: Option<OperationRef<'c, 'a>> = None;
        let mut op = body.first_operation();
        while let Some(current) = op {
            op = current.next_in_block();
            if op_name(&current) != quantum_dynamic::op::IF {
                prev_if = None;
                continue;
            }
            if let Some(previous) = prev_if
                && can_fuse(previous, current)
            {
                if let Err(error) = fuse_pair(context, body, previous, current) {
                    eprintln!("classical-region-fusion: {error}");
                } else {
                    fused = true;
                    break;
                }
            }
            prev_if = Some(current);
        }
        if !fused {
            break;
        }
    }
}

/// Runs classical region fusion on `module`.
pub fn run_on_module<'c>(context: &'c Context, module: &melior::ir::Module<'c>) {
    fuse_module(context, module.as_operation());
}

#[repr(align(8))]
struct PassId;

static CLASSICAL_REGION_FUSION_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct ClassicalRegionFusion {
    context: usize,
}

impl ClassicalRegionFusion {
    fn new() -> Self {
        Self { context: 0 }
    }
}

impl<'c> RunExternalPass<'c> for ClassicalRegionFusion {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        if self.context == 0 {
            pass.signal_failure();
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        fuse_module(context, operation);
    }
}

/// Creates the classical region fusion pass.
pub fn create_pass() -> Pass {
    create_external(
        ClassicalRegionFusion::new(),
        TypeId::create(&CLASSICAL_REGION_FUSION_PASS_ID),
        "classical-region-fusion",
        "classical-region-fusion",
        "Fuse adjacent quantum.dynamic.if ops on disjoint qubits",
        "",
        &[],
    )
}
