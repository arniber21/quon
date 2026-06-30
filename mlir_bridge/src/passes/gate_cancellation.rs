//! Gate cancellation pass (issue #18, SPEC §7.2).
//!
//! Peephole optimization over each `quantum.circ.func` body: adjacent self-inverse
//! gate pairs on the same qubit wire(s) are erased and their SSA uses rewired.

use std::collections::HashMap;

use melior::StringRef;
use melior::ir::attribute::{IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{Attribute, AttributeLike, BlockLike, OperationRef, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter};
use mlir_sys::mlirOperationSetAttributeByName;
use quon_core::DepthExpr;

use crate::dialect::quantum_circ::{self, attr};

#[derive(Clone, Copy)]
struct GateRef<'c, 'a> {
    operation: OperationRef<'c, 'a>,
    depth_contribution: i64,
}

#[derive(Clone)]
struct GateInfo {
    name: String,
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

fn read_depth_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> quon_core::DepthExpr {
    read_string_attr(operation, attr::DEPTH)
        .and_then(|text| quon_core::DepthExpr::parse(&text).ok())
        .unwrap_or(quon_core::DepthExpr::Nat(0))
}

fn normalize_gate_name(name: &str) -> &str {
    match name {
        "CX" => "CNOT",
        other => other,
    }
}

fn inverse_pair(a: &str, b: &str) -> bool {
    let a = normalize_gate_name(a);
    let b = normalize_gate_name(b);
    if a == b {
        return matches!(a, "H" | "X" | "Y" | "Z" | "CNOT" | "CZ" | "SWAP");
    }
    matches!(
        (a, b),
        ("S", "S†")
            | ("S†", "S")
            | ("S", "Sdag")
            | ("Sdag", "S")
            | ("T", "T†")
            | ("T†", "T")
            | ("T", "Tdag")
            | ("Tdag", "T")
    )
}

fn parse_gate<'c, 'a>(operation: OperationRef<'c, 'a>) -> Option<(GateRef<'c, 'a>, GateInfo)> {
    if op_name(&operation) != quantum_circ::op::GATE {
        return None;
    }
    let name = read_string_attr(&operation, attr::GATE_NAME)?;
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
    let depth_contribution = read_i64_attr(&operation, attr::DEPTH_CONTRIBUTION).unwrap_or(1);
    Some((
        GateRef {
            operation,
            depth_contribution,
        },
        GateInfo {
            name,
            operand_keys,
            result_keys,
        },
    ))
}

fn can_cancel(previous: &GateInfo, current: &GateInfo) -> bool {
    previous.result_keys == current.operand_keys && inverse_pair(&previous.name, &current.name)
}

fn cancel_pair<'c, 'a>(
    context: &'c Context,
    previous: GateRef<'c, 'a>,
    current: GateRef<'c, 'a>,
    _previous_info: &GateInfo,
    _current_info: &GateInfo,
) -> i64 {
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
    for (input, output) in inputs.into_iter().zip(outputs) {
        base.replace_all_uses_with(output, input);
    }
    base.erase_op(current.operation);
    base.erase_op(previous.operation);
    previous.depth_contribution + current.depth_contribution
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

fn cancel_in_block<'c, 'a>(context: &'c Context, block: melior::ir::BlockRef<'c, 'a>) -> i64 {
    let mut total_removed = 0i64;
    loop {
        let removed = cancel_once_in_block(context, block);
        if removed == 0 {
            break;
        }
        total_removed += removed;
    }
    total_removed
}

fn cancel_once_in_block<'c, 'a>(context: &'c Context, block: melior::ir::BlockRef<'c, 'a>) -> i64 {
    let mut removed_depth = 0i64;
    let mut producer: HashMap<usize, (GateRef<'c, 'a>, GateInfo)> = HashMap::new();

    let mut op = block.first_operation();
    while let Some(current_op) = op {
        op = current_op.next_in_block();
        if op_name(&current_op) == quantum_circ::op::RETURN {
            break;
        }
        let Some((gate_ref, gate_info)) = parse_gate(current_op) else {
            continue;
        };

        let mut cancelled = false;
        if !gate_info.operand_keys.is_empty()
            && let Some((previous_ref, previous_info)) =
                producer.get(&gate_info.operand_keys[0]).cloned()
            && gate_info.operand_keys.iter().all(|key| {
                producer
                    .get(key)
                    .is_some_and(|entry| entry.0.operation == previous_ref.operation)
            })
            && can_cancel(&previous_info, &gate_info)
        {
            removed_depth +=
                cancel_pair(context, previous_ref, gate_ref, &previous_info, &gate_info);
            for key in &previous_info.result_keys {
                producer.remove(key);
            }
            cancelled = true;
        }

        if cancelled {
            continue;
        }

        for key in &gate_info.result_keys {
            producer.insert(*key, (gate_ref, gate_info.clone()));
        }
    }

    removed_depth
}

pub fn cancel_in_func<'c, 'a>(context: &'c Context, func: OperationRef<'c, 'a>) -> i64 {
    if op_name(&func) != quantum_circ::op::FUNC {
        return 0;
    }
    let Ok(region) = func.region(0) else {
        return 0;
    };
    let Some(block) = region.first_block() else {
        return 0;
    };
    let removed = cancel_in_block(context, block);
    if removed > 0
        && let DepthExpr::Nat(n) = read_depth_attr(&func)
    {
        let updated = n.saturating_sub(removed as u64);
        set_func_depth(context, func, &DepthExpr::Nat(updated));
    }
    removed
}

pub fn cancel_module<'c, 'a>(context: &'c Context, module: OperationRef<'c, 'a>) {
    let Some(body) = module
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return;
    };
    let mut op = body.first_operation();
    while let Some(current) = op {
        if op_name(&current) == quantum_circ::op::FUNC {
            cancel_in_func(context, current);
        }
        op = current.next_in_block();
    }
}

/// Runs gate cancellation on every `quantum.circ.func` in `module`.
pub fn run_on_module<'c>(context: &'c Context, module: &melior::ir::Module<'c>) {
    cancel_module(context, module.as_operation());
}

#[repr(align(8))]
struct PassId;

static GATE_CANCELLATION_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct GateCancellation {
    context: usize,
}

impl GateCancellation {
    fn new() -> Self {
        Self { context: 0 }
    }
}

impl<'c> RunExternalPass<'c> for GateCancellation {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, pass: ExternalPass<'_>) {
        if self.context == 0 {
            pass.signal_failure();
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        cancel_module(context, operation);
    }
}

/// Creates the gate cancellation pass.
pub fn create_pass() -> Pass {
    create_external(
        GateCancellation::new(),
        TypeId::create(&GATE_CANCELLATION_PASS_ID),
        "gate-cancellation",
        "gate-cancellation",
        "Cancel adjacent self-inverse gate pairs in quantum.circ.func regions",
        "",
        &[],
    )
}
