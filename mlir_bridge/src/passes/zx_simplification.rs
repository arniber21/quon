//! ZX simplification pass (issue #20).

use melior::ir::attribute::{FloatAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::r#type::TypeId;
use melior::ir::{BlockLike, OperationRef, RegionLike, Value};
use melior::pass::{ExternalPass, Pass, RunExternalPass, create_external};
use melior::{Context, ContextRef, IrRewriter};
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

/// Replaces the gate sequence of `func` with `gates` in place, threading fresh
/// wires from the block arguments and repointing the terminator. Returns `true`
/// when the body was rewritten.
///
/// The work is done on the existing block — the original gate ops are erased
/// only after the new ones and the new terminator are wired up, so the IR is
/// never left with dangling uses. (The previous version built a detached block
/// and dropped it, leaving the pass a silent no-op.)
fn rebuild_func_body<'c, 'a>(
    context: &'c Context,
    func: OperationRef<'c, 'a>,
    gates: &[GateRef],
) -> bool {
    let Ok(region) = func.region(0) else {
        return false;
    };
    let Some(block) = region.first_block() else {
        return false;
    };
    let location = func.location();

    // Find the terminator and snapshot the gate ops we are about to replace.
    let mut return_op = None;
    let mut old_ops = Vec::new();
    let mut op = block.first_operation();
    while let Some(current) = op {
        op = current.next_in_block();
        if op_name(&current) == quantum_circ::op::RETURN {
            return_op = Some(current);
            break;
        }
        old_ops.push(current);
    }
    let Some(return_op) = return_op else {
        return false;
    };

    let mut wires: Vec<Value<'c, 'a>> = (0..block.argument_count())
        .map(|index| Value::from(block.argument(index).expect("arg")))
        .collect();
    for gate in gates {
        let (Some(&first), true) = (
            gate.qubits.first(),
            gate.qubits.iter().all(|index| *index < wires.len()),
        ) else {
            return false;
        };
        let operands: Vec<Value<'c, 'a>> = gate.qubits.iter().map(|index| wires[*index]).collect();
        let built = if let Some(angle) = gate.angle {
            quantum_circ::rotation_gate(
                context,
                &gate.name,
                angle,
                1,
                false,
                wires[first],
                location,
            )
        } else {
            quantum_circ::gate(context, &gate.name, 1, true, &operands, location)
        };
        let Ok(built) = built else {
            return false;
        };
        let appended = block.insert_operation_before(return_op, built);
        for (index, target) in gate.qubits.iter().enumerate() {
            if let Ok(result) = appended.result(index) {
                wires[*target] = Value::from(result);
            }
        }
    }

    // Insert the new terminator, then erase the old terminator and gates. The
    // new gates read block arguments only, so erasing the old ops (deepest
    // first) can never strand a live use.
    block.insert_operation_before(
        return_op,
        quantum_circ::r#return(&wires, location).expect("return"),
    );
    let rewriter = IrRewriter::new(context);
    let base = rewriter.as_rewriter_base();
    base.erase_op(return_op);
    for old in old_ops.into_iter().rev() {
        base.erase_op(old);
    }
    true
}

/// Returns true when ZX rewriting shrinks the gate list for `func`.
///
/// Restricted to single-wire funcs: `extract_gates` keys gates by operand
/// position rather than SSA wire, and [`zx_to_circuit`] only round-trips
/// independent single-qubit chains. Outside that regime the encode/extract
/// round trip is not faithful, so we decline rather than risk emitting a
/// shorter-but-different circuit. Multi-qubit ZX extraction is a follow-up.
/// Gates the ZX encode/extract path can handle without silently dropping ops.
/// Unsupported names (S, T, Y, SWAP, …) are ignored by `circuit_to_zx`, so a
/// rewrite that only checked `simplified.len() < before` could delete them.
fn zx_encodable(name: &str) -> bool {
    matches!(name, "H" | "X" | "Z" | "Rx" | "Rz" | "CNOT" | "CX")
}

pub fn simplify_func<'c, 'a>(context: &'c Context, func: OperationRef<'c, 'a>) -> bool {
    if func_qubit_count(func) != 1 {
        return false;
    }
    let gates = extract_gates(func);
    if gates.len() < 2 {
        return false;
    }
    if gates.iter().any(|gate| !zx_encodable(&gate.name)) {
        return false;
    }
    let mut zx = circuit_to_zx(&gates);
    let before = gates.len();
    simplify(&mut zx);
    let simplified = zx_to_circuit(&zx);
    // Empty means extract declined (H edges / branching); never treat as identity.
    if simplified.is_empty() || simplified.len() >= before {
        return false;
    }
    rebuild_func_body(context, func, &simplified)
}

/// Number of qubit arguments on `func`'s entry block (0 if it has no body).
fn func_qubit_count<'c, 'a>(func: OperationRef<'c, 'a>) -> usize {
    func.region(0)
        .ok()
        .and_then(|region| region.first_block())
        .map(|block| block.argument_count())
        .unwrap_or(0)
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
