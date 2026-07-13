//! Compiler-assisted uncomputation pass (issue #21).
//!
//! Appends the adjoint of a reversible `quantum.circ.borrow` body and rewires
//! the region's `return` so the composed body is identity on the ancilla
//! (when started in `|0…0⟩`).

use melior::ir::attribute::StringAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{BlockLike, OperationRef, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter};
use mlir_sys::mlirOperationSetOperand;

use crate::dialect::quantum_circ::{self, attr};

#[derive(Clone)]
struct RecordedGate {
    name: String,
    /// Logical qubit indices (block-argument order) this gate acts on.
    targets: Vec<usize>,
}

fn op_name<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> String {
    operation
        .name()
        .as_string_ref()
        .as_str()
        .unwrap_or("")
        .to_string()
}

fn value_key<'a>(value: &impl ValueLike<'a>) -> usize {
    value.to_raw().ptr as usize
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

fn inverse_name(name: &str) -> Option<String> {
    // Only invertible discrete gates participate in uncomputation; parametric
    // rotations need angle negation and are refused here (matches prior behaviour).
    let info = quon_core::gates::lookup(name)?;
    if info.parametric {
        return None;
    }
    Some(info.inverse.to_string())
}

fn set_return_operands<'c, 'a>(return_op: OperationRef<'c, 'a>, wires: &[Value<'c, 'a>]) {
    for (index, value) in wires.iter().enumerate() {
        unsafe {
            mlirOperationSetOperand(return_op.to_raw(), index as isize, value.to_raw());
        }
    }
}

fn uncompute_borrow<'c, 'a>(context: &'c Context, borrow: OperationRef<'c, 'a>) -> bool {
    let Ok(region) = borrow.region(0) else {
        return false;
    };
    let Some(block) = region.first_block() else {
        return false;
    };
    let location = borrow.location();
    let width = block.argument_count();

    // Map each live SSA value to its logical qubit index, and track the latest
    // value per logical wire through the forward body.
    let mut value_to_logical = std::collections::HashMap::new();
    let mut wires: Vec<Value<'c, 'a>> = Vec::with_capacity(width);
    for index in 0..width {
        let Ok(arg) = block.argument(index) else {
            return false;
        };
        let value = Value::from(arg);
        value_to_logical.insert(value_key(&value), index);
        wires.push(value);
    }

    let mut gates = Vec::new();
    let mut return_op = None;
    let mut op = block.first_operation();
    while let Some(current) = op {
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN {
            return_op = Some(current);
            break;
        }
        if name != quantum_circ::op::GATE {
            return false;
        }
        let gate_name = read_string_attr(&current, attr::GATE_NAME).unwrap_or_default();
        if inverse_name(&gate_name).is_none() {
            return false;
        }
        let mut targets = Vec::with_capacity(current.operand_count());
        for i in 0..current.operand_count() {
            let Ok(operand) = current.operand(i) else {
                return false;
            };
            let Some(&logical) = value_to_logical.get(&value_key(&operand)) else {
                return false;
            };
            targets.push(logical);
        }
        for (i, &logical) in targets.iter().enumerate() {
            let Ok(result) = current.result(i) else {
                return false;
            };
            let value = Value::from(result);
            value_to_logical.insert(value_key(&value), logical);
            wires[logical] = value;
        }
        gates.push(RecordedGate {
            name: gate_name,
            targets,
        });
        op = current.next_in_block();
    }

    let Some(return_op) = return_op else {
        return false;
    };

    // Append adjoints in reverse, starting from the forward body's final wires.
    for gate in gates.iter().rev() {
        let Some(inverse) = inverse_name(&gate.name) else {
            return false;
        };
        let operands: Vec<Value<'c, 'a>> = gate.targets.iter().map(|index| wires[*index]).collect();
        let Ok(built) = quantum_circ::gate(context, &inverse, 1, true, &operands, location) else {
            return false;
        };
        let appended = block.insert_operation_before(return_op, built);
        for (index, target) in gate.targets.iter().enumerate() {
            if let Ok(result) = appended.result(index) {
                wires[*target] = Value::from(result);
            }
        }
    }

    // Point `return` at the post-adjoint wires so the body is U;U†, not dead code.
    if return_op.operand_count() == wires.len() {
        set_return_operands(return_op, &wires);
    } else {
        // Fall back to replace_all_uses if arity somehow differs.
        let rewriter = IrRewriter::new(context);
        let base = rewriter.as_rewriter_base();
        for (index, wire) in wires.iter().enumerate() {
            if let Ok(old) = return_op.operand(index) {
                base.replace_all_uses_with(old, *wire);
            }
        }
    }
    true
}

fn uncompute_module<'c, 'a>(context: &'c Context, module: OperationRef<'c, 'a>) {
    let Some(body) = module
        .region(0)
        .ok()
        .and_then(|region| region.first_block())
    else {
        return;
    };
    let mut op = body.first_operation();
    while let Some(current) = op {
        if op_name(&current) == quantum_circ::op::BORROW {
            uncompute_borrow(context, current);
        }
        op = current.next_in_block();
    }
}

/// Runs compiler-assisted uncomputation on `module`.
pub fn run_on_module<'c>(context: &'c Context, module: &melior::ir::Module<'c>) {
    uncompute_module(context, module.as_operation());
}

#[repr(align(8))]
struct PassId;

static COMPILER_UNCOMPUTATION_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct CompilerUncomputation {
    context: usize,
}

impl CompilerUncomputation {
    fn new() -> Self {
        Self { context: 0 }
    }
}

impl<'c> RunExternalPass<'c> for CompilerUncomputation {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, _pass: ExternalPass<'_>) {
        if self.context == 0 {
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        uncompute_module(context, operation);
    }
}

/// Creates the compiler-assisted uncomputation pass.
pub fn create_pass() -> Pass {
    create_external(
        CompilerUncomputation::new(),
        TypeId::create(&COMPILER_UNCOMPUTATION_PASS_ID),
        "compiler-uncomputation",
        "compiler-uncomputation",
        "Append adjoint circuits to reversible borrow regions",
        "",
        &[],
    )
}
