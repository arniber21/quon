//! Compiler-assisted uncomputation pass (issue #21).

use melior::ir::attribute::StringAttribute;
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{BlockLike, OperationRef, RegionLike, Value};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef};

use crate::dialect::quantum_circ::{self, attr};

#[derive(Clone)]
struct RecordedGate {
    name: String,
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
    let inverse = match name {
        "H" | "X" | "Y" | "Z" | "CNOT" | "CX" | "CZ" | "SWAP" => name,
        "S" => "S†",
        "S†" | "Sdag" => "S",
        "T" => "T†",
        "T†" | "Tdag" => "T",
        _ => return None,
    };
    Some(inverse.to_string())
}

fn uncompute_borrow<'c, 'a>(context: &'c Context, borrow: OperationRef<'c, 'a>) -> bool {
    let Ok(region) = borrow.region(0) else {
        return false;
    };
    let Some(block) = region.first_block() else {
        return false;
    };
    let location = borrow.location();
    let mut gates = Vec::new();
    let mut op = block.first_operation();
    while let Some(current) = op {
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN {
            break;
        }
        if name != quantum_circ::op::GATE {
            return false;
        }
        let gate_name = read_string_attr(&current, attr::GATE_NAME).unwrap_or_default();
        if inverse_name(&gate_name).is_none() {
            return false;
        }
        let targets: Vec<usize> = (0..current.operand_count()).collect();
        gates.push(RecordedGate {
            name: gate_name,
            targets,
        });
        op = current.next_in_block();
    }

    let mut wires: Vec<Value<'c, 'a>> = (0..block.argument_count())
        .map(|index| Value::from(block.argument(index).expect("arg")))
        .collect();
    let return_op = {
        let mut cursor = block.first_operation();
        let mut found = None;
        while let Some(current) = cursor {
            if op_name(&current) == quantum_circ::op::RETURN {
                found = Some(current);
                break;
            }
            cursor = current.next_in_block();
        }
        found.expect("borrow return")
    };
    for gate in gates.iter().rev() {
        let inverse = inverse_name(&gate.name).expect("inverse");
        let operands: Vec<Value<'c, 'a>> = gate.targets.iter().map(|index| wires[*index]).collect();
        let built = quantum_circ::gate(context, &inverse, 1, true, &operands, location)
            .expect("adjoint gate");
        let appended = block.insert_operation_before(return_op, built);
        for (index, target) in gate.targets.iter().enumerate() {
            if let Ok(result) = appended.result(index) {
                wires[*target] = Value::from(result);
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
