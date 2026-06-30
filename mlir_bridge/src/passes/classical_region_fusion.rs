//! Classical region fusion pass (issue #23, SPEC §7.3).
//!
//! Merges adjacent `quantum.dynamic.if` ops that share a condition bit and act
//! on disjoint qubit sets into a single `if` with parallel unitary regions.

use std::collections::{HashMap, HashSet};

use melior::ir::attribute::{BoolAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{Block, BlockLike, Location, OperationRef, Region, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter};
use thiserror::Error;

use crate::dialect::{quantum_circ, quantum_dynamic};

#[derive(Debug, Error)]
pub enum FusionError {
    #[error("failed to build `{op}`: {message}")]
    Build { op: &'static str, message: String },
}

fn value_key<'a>(value: &impl ValueLike<'a>) -> usize {
    value.to_raw().ptr as usize
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
}

fn extract_gates_from_region<'c, 'a>(region: melior::ir::RegionRef<'c, 'a>) -> Vec<GateStep> {
    let mut gates = Vec::new();
    let Some(block) = region.first_block() else {
        return gates;
    };
    let mut op = block.first_operation();
    while let Some(inner) = op {
        if op_name(&inner) == quantum_circ::op::GATE {
            let gate_name = read_string_attr(&inner, quantum_circ::attr::GATE_NAME)
                .unwrap_or_else(|| "?".to_string());
            let targets: Vec<usize> = inner
                .operands()
                .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
                .enumerate()
                .map(|(index, _)| index)
                .collect();
            gates.push(GateStep {
                name: gate_name,
                targets,
            });
        }
        op = inner.next_in_block();
    }
    gates
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
                && value_key(&result) == bit_key
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
    let bit_key = value_key(&condition);
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
    let left_keys: HashSet<_> = qubit_operands(left).iter().map(value_key).collect();
    let right_keys: HashSet<_> = qubit_operands(right).iter().map(value_key).collect();
    left_keys.is_disjoint(&right_keys)
}

fn same_condition<'c, 'a>(left: OperationRef<'c, 'a>, right: OperationRef<'c, 'a>) -> bool {
    let Ok(left_bit) = left.operand(0) else {
        return false;
    };
    let Ok(right_bit) = right.operand(0) else {
        return false;
    };
    value_key(&left_bit) == value_key(&right_bit)
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
            quantum_circ::gate(context, &gate.name, 1, true, &operands, location).map_err(
                |error| FusionError::Build {
                    op: quantum_circ::op::GATE,
                    message: error.to_string(),
                },
            )?,
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
            quantum_circ::gate(context, &gate.name, 1, true, &operands, location).map_err(
                |error| FusionError::Build {
                    op: quantum_circ::op::GATE,
                    message: error.to_string(),
                },
            )?,
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

fn fuse_pair<'c, 'a>(
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
    })?);
    let else_left = extract_gates_from_region(first.region(1).map_err(|_| FusionError::Build {
        op: quantum_dynamic::op::IF,
        message: "missing else region".to_string(),
    })?);
    let then_right =
        extract_gates_from_region(second.region(0).map_err(|_| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: "missing then region".to_string(),
        })?);
    let else_right =
        extract_gates_from_region(second.region(1).map_err(|_| FusionError::Build {
            op: quantum_dynamic::op::IF,
            message: "missing else region".to_string(),
        })?);

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
    if !same_condition(first, second) {
        return false;
    }
    if !disjoint_qubits(first, second) {
        return false;
    }
    if if_depends_on_other_bit(second, first) || if_depends_on_other_bit(first, second) {
        return false;
    }
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
