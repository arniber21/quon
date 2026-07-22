//! Rotation merging pass (issue #19, SPEC §7.2).

use std::collections::HashMap;
use std::f64::consts::TAU;

use melior::ir::attribute::{FloatAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{Attribute, AttributeLike, BlockLike, OperationRef, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter, StringRef};
use mlir_sys::mlirOperationSetAttributeByName;
use quon_core::DepthExpr;

use crate::dialect::{
    quantum_circ::{self, attr},
    quantum_dynamic,
};

#[derive(Clone, Copy)]
struct GateRef<'c, 'a> {
    operation: OperationRef<'c, 'a>,
    depth_contribution: i64,
}

#[derive(Clone)]
struct RotationInfo {
    axis: String,
    angle: f64,
    operand_keys: Vec<usize>,
    result_keys: Vec<usize>,
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

fn read_f64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<f64> {
    let value = operation.attribute(key).ok()?;
    FloatAttribute::try_from(value)
        .ok()
        .map(|float| float.value())
}

fn read_depth_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> quon_core::DepthExpr {
    read_string_attr(operation, attr::DEPTH)
        .and_then(|text| quon_core::DepthExpr::parse(&text).ok())
        .unwrap_or(quon_core::DepthExpr::Nat(0))
}

fn rotation_axis(name: &str) -> Option<&'static str> {
    match name {
        "Rz" => Some("Rz"),
        "Rx" => Some("Rx"),
        "Ry" => Some("Ry"),
        _ => None,
    }
}

fn normalize_angle(angle: f64) -> f64 {
    let mut value = angle % TAU;
    if value < 0.0 {
        value += TAU;
    }
    value
}

fn is_zero_mod_2pi(angle: f64) -> bool {
    let value = normalize_angle(angle);
    value.abs() < 1e-9 || (value - TAU).abs() < 1e-9
}

fn parse_rotation<'c, 'a>(
    operation: OperationRef<'c, 'a>,
) -> Option<(GateRef<'c, 'a>, RotationInfo)> {
    if op_name(&operation) != quantum_circ::op::GATE {
        return None;
    }
    let name = read_string_attr(&operation, attr::GATE_NAME)?;
    let axis = rotation_axis(&name)?.to_string();
    let angle = read_f64_attr(&operation, attr::ANGLE)?;
    let operand_keys = operation
        .operands()
        .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
        .map(|operand| value_key(&operand))
        .collect();
    let result_keys = operation
        .results()
        .filter(|result| quantum_circ::is_qubit_type(result.r#type()))
        .map(|result| value_key(&result))
        .collect();
    Some((
        GateRef {
            operation,
            depth_contribution: read_i64_attr(&operation, attr::DEPTH_CONTRIBUTION).unwrap_or(1),
        },
        RotationInfo {
            axis,
            angle,
            operand_keys,
            result_keys,
        },
    ))
}

fn set_func_depth<'c, 'a>(context: &'c Context, func: OperationRef<'c, 'a>, depth: &DepthExpr) {
    let attribute: Attribute<'c> = StringAttribute::new(context, &depth.to_sexpr()).into();
    unsafe {
        mlirOperationSetAttributeByName(
            func.to_raw(),
            StringRef::new(attr::DEPTH).to_raw(),
            attribute.to_raw(),
        );
    }
}

fn merge_pair<'c, 'a>(
    context: &'c Context,
    module_block: melior::ir::BlockRef<'c, 'a>,
    anchor: OperationRef<'c, 'a>,
    previous: GateRef<'c, 'a>,
    current: GateRef<'c, 'a>,
    previous_info: &RotationInfo,
    current_info: &RotationInfo,
) -> (i64, bool) {
    let rewriter = IrRewriter::new(context);
    let base = rewriter.as_rewriter_base();
    let inputs: Vec<Value<'c, 'a>> = previous
        .operation
        .operands()
        .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
        .collect();
    let outputs: Vec<Value<'c, 'a>> = (0..current.operation.result_count())
        .filter_map(|index| current.operation.result(index).ok())
        .filter(|result| quantum_circ::is_qubit_type(result.r#type()))
        .map(Value::from)
        .collect();
    let total = previous_info.angle + current_info.angle;
    if is_zero_mod_2pi(total) {
        for (input, output) in inputs.into_iter().zip(outputs) {
            base.replace_all_uses_with(output, input);
        }
        base.erase_op(current.operation);
        base.erase_op(previous.operation);
        return (
            previous.depth_contribution + current.depth_contribution,
            true,
        );
    }
    let location = current.operation.location();
    let merged = module_block.insert_operation_before(
        anchor,
        quantum_circ::rotation_gate(
            context,
            &previous_info.axis,
            total,
            previous.depth_contribution,
            false,
            inputs[0],
            location,
        )
        .expect("merged rotation builds"),
    );
    let merged_out = Value::from(merged.result(0).expect("merged result"));
    for output in outputs {
        base.replace_all_uses_with(output, merged_out);
    }
    base.erase_op(current.operation);
    base.erase_op(previous.operation);
    (
        previous.depth_contribution + current.depth_contribution,
        false,
    )
}

fn merge_once_in_block<'c, 'a>(context: &'c Context, block: melior::ir::BlockRef<'c, 'a>) -> i64 {
    let mut producer: HashMap<usize, (GateRef<'c, 'a>, RotationInfo)> = HashMap::new();
    let mut removed_depth = 0i64;

    let mut op = block.first_operation();
    while let Some(current_op) = op {
        op = current_op.next_in_block();
        if op_name(&current_op) == quantum_circ::op::RETURN {
            break;
        }
        let Some((gate_ref, gate_info)) = parse_rotation(current_op) else {
            continue;
        };

        if !gate_info.operand_keys.is_empty()
            && let Some((previous_ref, previous_info)) =
                producer.get(&gate_info.operand_keys[0]).cloned()
            && gate_info.operand_keys.iter().all(|key| {
                producer
                    .get(key)
                    .is_some_and(|entry| entry.0.operation == previous_ref.operation)
            })
            && previous_info.axis == gate_info.axis
            && previous_info.result_keys == gate_info.operand_keys
        {
            let (removed, _) = merge_pair(
                context,
                block,
                current_op,
                previous_ref,
                gate_ref,
                &previous_info,
                &gate_info,
            );
            removed_depth += removed;
            for key in &previous_info.result_keys {
                producer.remove(key);
            }
            return removed_depth;
        }

        for key in &gate_info.result_keys {
            producer.insert(*key, (gate_ref, gate_info.clone()));
        }
    }

    removed_depth
}

fn merge_in_block<'c, 'a>(context: &'c Context, block: melior::ir::BlockRef<'c, 'a>) -> i64 {
    let mut total = 0i64;
    loop {
        let removed = merge_once_in_block(context, block);
        if removed == 0 {
            break;
        }
        total += removed;
    }
    total
}

fn merge_in_func<'c, 'a>(context: &'c Context, func: OperationRef<'c, 'a>) -> i64 {
    if op_name(&func) != quantum_circ::op::FUNC {
        return 0;
    }
    let Ok(region) = func.region(0) else {
        return 0;
    };
    let Some(block) = region.first_block() else {
        return 0;
    };
    let removed = merge_in_block(context, block);
    if removed > 0
        && let DepthExpr::Nat(n) = read_depth_attr(&func)
    {
        // Flux-verified non-increasing depth (quon_core::optimization).
        set_func_depth(
            context,
            func,
            &DepthExpr::Nat(quon_core::depth_after_removal(n, removed as u64)),
        );
    }
    removed
}

fn merge_module<'c, 'a>(context: &'c Context, module: OperationRef<'c, 'a>) {
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
        let name = op_name(&current);
        if name == quantum_circ::op::FUNC {
            merge_in_func(context, current);
        } else if name == quantum_dynamic::op::UNITARY_REGION {
            // Circ gates now live in `unitary_region`/`if` bodies after the
            // staging-dialect collapse (#213 / ADR-0037).
            if let Some(block) = current
                .region(0)
                .ok()
                .and_then(|region| region.first_block())
            {
                merge_in_block(context, block);
            }
        } else if name == quantum_dynamic::op::IF {
            for region_index in 0..2 {
                if let Some(block) = current
                    .region(region_index)
                    .ok()
                    .and_then(|region| region.first_block())
                {
                    merge_in_block(context, block);
                }
            }
        }
    }
}

/// Runs rotation merging on every `quantum.circ.func` in `module`.
pub fn run_on_module<'c>(context: &'c Context, module: &melior::ir::Module<'c>) {
    merge_module(context, module.as_operation());
}

#[repr(align(8))]
struct PassId;

static ROTATION_MERGING_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct RotationMerging {
    context: usize,
}

impl RotationMerging {
    fn new() -> Self {
        Self { context: 0 }
    }
}

impl<'c> RunExternalPass<'c> for RotationMerging {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        if self.context == 0 {
            pass.signal_failure();
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        merge_module(context, operation);
    }
}

/// Creates the rotation merging pass.
pub fn create_pass() -> Pass {
    create_external(
        RotationMerging::new(),
        TypeId::create(&ROTATION_MERGING_PASS_ID),
        "rotation-merging",
        "rotation-merging",
        "Merge consecutive same-axis rotations in quantum.circ.func regions",
        "",
        &[],
    )
}
