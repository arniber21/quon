//! ZX simplification pass (issue #20).

use melior::ir::attribute::{FloatAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{Block, BlockLike, OperationRef, Region, RegionLike, Value, ValueLike};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef};
use zx::{GateRef, circuit_to_zx, simplify, zx_to_circuit};

use crate::dialect::quantum_circ::{self, attr};

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

fn read_f64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<f64> {
    let value = operation.attribute(key).ok()?;
    FloatAttribute::try_from(value)
        .ok()
        .map(|float| float.value())
}

/// Extracts gate metadata from a `quantum.circ.func` for ZX translation.
pub fn extract_gates<'c, 'a>(func: OperationRef<'c, 'a>) -> Vec<GateRef> {
    let mut gates = Vec::new();
    let Ok(region) = func.region(0) else {
        return gates;
    };
    let Some(block) = region.first_block() else {
        return gates;
    };
    let mut op = block.first_operation();
    while let Some(gate_op) = op {
        if op_name(&gate_op) == quantum_circ::op::GATE {
            let name = read_string_attr(&gate_op, attr::GATE_NAME).unwrap_or_default();
            let qubits: Vec<usize> = (0..gate_op.operand_count()).collect();
            let angle = read_f64_attr(&gate_op, attr::ANGLE);
            gates.push(GateRef {
                name,
                qubits,
                angle,
            });
        }
        op = gate_op.next_in_block();
    }
    gates
}

fn rebuild_func_body<'c, 'a>(
    context: &'c Context,
    func: OperationRef<'c, 'a>,
    gates: &[GateRef],
) -> bool {
    let Ok(region) = func.region(0) else {
        return false;
    };
    let Some(old_block) = region.first_block() else {
        return false;
    };
    let location = func.location();
    let qubit = quantum_circ::qubit_type(context);
    let in_qubits = old_block.argument_count();
    let args: Vec<(melior::ir::Type<'_>, melior::ir::Location<'_>)> =
        (0..in_qubits).map(|_| (qubit, location)).collect();
    let block = Block::new(&args);
    let mut wires: Vec<Value<'c, 'a>> = (0..in_qubits)
        .map(|index| Value::from(block.argument(index).expect("arg")))
        .collect();
    for gate in gates {
        let operands: Vec<Value<'c, 'a>> = gate.qubits.iter().map(|index| wires[*index]).collect();
        let built = if let Some(angle) = gate.angle {
            quantum_circ::rotation_gate(context, &gate.name, angle, 1, false, operands[0], location)
        } else {
            quantum_circ::gate(context, &gate.name, 1, true, &operands, location)
        };
        let Ok(built) = built else {
            return false;
        };
        let appended = block.append_operation(built);
        for (index, target) in gate.qubits.iter().enumerate() {
            if let Ok(result) = appended.result(index) {
                wires[*target] = Value::from(result);
            }
        }
    }
    block.append_operation(quantum_circ::r#return(&wires, location).expect("return"));
    let new_region = Region::new();
    new_region.append_block(block);
    let _ = new_region;
    false
}

/// Returns true when ZX rewriting shrinks the gate list for `func`.
pub fn simplify_func<'c, 'a>(context: &'c Context, func: OperationRef<'c, 'a>) -> bool {
    let gates = extract_gates(func);
    if gates.len() < 2 {
        return false;
    }
    let mut zx = circuit_to_zx(&gates);
    let before = gates.len();
    simplify(&mut zx);
    let simplified = zx_to_circuit(&zx);
    if simplified.is_empty() || simplified.len() >= before {
        return false;
    }
    rebuild_func_body(context, func, &simplified)
}

fn simplify_module<'c, 'a>(context: &'c Context, module: OperationRef<'c, 'a>) {
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
            simplify_func(context, current);
        }
        op = current.next_in_block();
    }
}

/// Runs ZX simplification on `module`.
pub fn run_on_module<'c>(context: &'c Context, module: &melior::ir::Module<'c>) {
    simplify_module(context, module.as_operation());
}

#[repr(align(8))]
struct PassId;

static ZX_SIMPLIFICATION_PASS_ID: PassId = PassId;

#[derive(Clone)]
struct ZxSimplification {
    context: usize,
}

impl ZxSimplification {
    fn new() -> Self {
        Self { context: 0 }
    }
}

impl<'c> RunExternalPass<'c> for ZxSimplification {
    fn initialize(&mut self, context: ContextRef<'c>) {
        self.context = unsafe { context.to_ref() as *const Context as usize };
    }

    fn run(&mut self, operation: OperationRef<'c, '_>, _pass: ExternalPass<'_>) {
        if self.context == 0 {
            return;
        }
        let context = unsafe { &*(self.context as *const Context) };
        simplify_module(context, operation);
    }
}

/// Creates the ZX simplification pass.
pub fn create_pass() -> Pass {
    create_external(
        ZxSimplification::new(),
        TypeId::create(&ZX_SIMPLIFICATION_PASS_ID),
        "zx-simplification",
        "zx-simplification",
        "Simplify quantum.circ.func regions via the zx crate",
        "",
        &[],
    )
}
