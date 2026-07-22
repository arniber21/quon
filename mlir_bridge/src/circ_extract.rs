//! Faithful `quantum.circ` extract/rebuild seam (issue #320).
//!
//! Shared by ZX simplification (#75) and Clifford+T optimization (#96 /
//! ADR-0013). Extracts a `quantum.circ.func` body into a Melior-free
//! [`CircIr`] — a flat gate+wire sequence keyed by the gate registry
//! (`quon_core::gates`) — and rebuilds it back into verified `quantum.circ`
//! ops.
//!
//! ## Faithfulness contract
//!
//! Extraction fails CLOSED: [`extract`] returns [`Err`] when the round-trip
//! cannot be faithful — circuit-level ops (`compose`/`tensor`/`adjoint`/
//! `controlled`/`borrow`), gates absent from the registry, or arity mismatches.
//! It NEVER silently drops a gate that would change semantics. Consumers
//! (ZX, Clifford+T) treat `Err` as "decline rewrite, leave the func unchanged".
//!
//! ## Wire tracking
//!
//! Logical qubit indices are recovered from SSA values via [`WireTracker`],
//! not from operand positions. This makes multi-qubit extraction faithful:
//! after a `CNOT` on wires `[0, 1]`, a subsequent gate on wire `1` correctly
//! records `qubits: [1]`, whereas the old operand-position encoding always
//! recorded `[0]`.

use melior::Context;
use melior::IrRewriter;
use melior::ir::attribute::{BoolAttribute, FloatAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationLike;
use melior::ir::{BlockLike, OperationRef, RegionLike, Value};

use thiserror::Error;

use quon_core::gates;

use crate::dialect::quantum_circ::{self, attr};
use crate::passes::qubit_wiring::WireTracker;

// --- Melior-free IR --------------------------------------------------------

/// A single gate application in the extracted circuit IR.
#[derive(Clone, Debug, PartialEq)]
pub struct CircGate {
    /// Canonical registry id (from `quon_core::gates`).
    pub name: String,
    /// Logical wire indices in operand order.
    pub qubits: Vec<usize>,
    /// Rotation angle in radians, if the gate is parametric.
    pub angle: Option<f64>,
    /// Per-gate depth contribution (preserved from the source op).
    pub depth_contribution: i64,
    /// Clifford classification (preserved from the source op).
    pub clifford: bool,
}

/// Melior-free flat gate+wire IR extracted from a `quantum.circ.func`.
///
/// This is the **CircIr** (CONTEXT.md): a sequence of [`CircGate`] applications
/// on `n_qubits` logical wires. ZX rewriting (#75) and Clifford+T optimization
/// (#96) consume this IR, apply their kernels, and call [`rebuild`] to write
/// the result back.
#[derive(Clone, Debug, PartialEq)]
pub struct CircIr {
    /// Number of qubit wires (the func's block argument count).
    pub n_qubits: usize,
    /// Gate applications in execution order.
    pub gates: Vec<CircGate>,
}

// --- Errors ----------------------------------------------------------------

/// Why extraction or rebuild could not proceed faithfully.
///
/// Every variant means "decline the rewrite" — the consumer leaves the func
/// unchanged. No variant silently drops a gate.
#[derive(Debug, Error, PartialEq, Eq)]
pub enum SeamError {
    /// The func has no body region or entry block.
    #[error("func has no body region")]
    NoBody,
    /// The func body contains a construct other than `quantum.circ.gate` or
    /// `quantum.circ.return`. Only flat gate sequences are extractable;
    /// structural ops (`compose`, `tensor`, `adjoint`, `controlled`, `borrow`)
    /// would lose semantics if flattened.
    #[error("unsupported construct `{0}` in func body — only quantum.circ.gate is extractable")]
    UnsupportedConstruct(String),
    /// The gate name is not in the gate registry (`quon_core::gates`).
    #[error("gate `{0}` is not in the gate registry")]
    UnknownGate(String),
    /// The gate's operand count does not match the registry arity.
    #[error("gate `{name}` has arity {actual} but the registry says {expected}")]
    ArityMismatch {
        name: String,
        expected: usize,
        actual: usize,
    },
    /// A required attribute is missing from a gate op.
    #[error("missing required attribute `{0}` on gate op")]
    MissingAttr(&'static str),
    /// The rebuild target func has no return terminator.
    #[error("rebuild target has no return terminator")]
    NoTerminator,
    /// A wire index in the [`CircIr`] is out of range.
    #[error("wire index {index} is out of range (n_qubits = {n_qubits})")]
    WireOutOfRange { index: usize, n_qubits: usize },
    /// A parametric (rotation) gate was applied to more than one wire.
    #[error("rotation gate `{0}` must be single-qubit")]
    MultiQubitRotation(String),
    /// The underlying Melior builder or verifier rejected the rebuilt op, or the
    /// [`CircIr`] is inconsistent with the target func (e.g. n_qubits mismatch).
    #[error("rebuild failed: {0}")]
    Build(String),
}

// --- Attribute readers -----------------------------------------------------

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

fn read_i64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<i64> {
    let value = operation.attribute(key).ok()?;
    IntegerAttribute::try_from(value)
        .ok()
        .map(|integer| integer.value())
}

fn read_bool_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O, key: &str) -> Option<bool> {
    let value = operation.attribute(key).ok()?;
    BoolAttribute::try_from(value)
        .ok()
        .map(|boolean| boolean.value())
}

// --- Extract ---------------------------------------------------------------

/// Faithfully extracts a `quantum.circ.func` body into a Melior-free [`CircIr`].
///
/// Walks the func's single block in order, tracking logical wire indices through
/// SSA values via [`WireTracker`]. Each `quantum.circ.gate` op becomes a
/// [`CircGate`] whose `name` is the **canonical registry id** (looked up
/// through `quon_core::gates`), whose `qubits` are the faithful logical wire
/// indices, and whose `angle`/`depth_contribution`/`clifford` are preserved
/// from the op.
///
/// Returns `Err` (decline) when:
/// - the func has no body,
/// - the body contains any op other than `gate` or `return` (structural ops
///   like `compose`, `borrow`, … cannot be flattened without losing semantics),
/// - a gate name is not in the registry,
/// - a gate's operand count does not match the registry arity.
///
/// Never silently drops a gate.
pub fn extract<'c, 'a>(func: OperationRef<'c, 'a>) -> Result<CircIr, SeamError> {
    let region = func.region(0).map_err(|_| SeamError::NoBody)?;
    let block = region.first_block().ok_or(SeamError::NoBody)?;

    let n_qubits = block.argument_count();
    let mut tracker = WireTracker::new();
    tracker.seed_block_args(&block);

    let mut gates = Vec::new();
    let mut op = block.first_operation();
    while let Some(current) = op {
        let name = op_name(&current);
        if name == quantum_circ::op::RETURN {
            break;
        }
        if name != quantum_circ::op::GATE {
            return Err(SeamError::UnsupportedConstruct(name));
        }

        let gate_name = read_string_attr(&current, attr::GATE_NAME)
            .ok_or(SeamError::MissingAttr(attr::GATE_NAME))?;
        let info = gates::lookup(&gate_name).ok_or(SeamError::UnknownGate(gate_name.clone()))?;
        let operand_count = current.operand_count();
        if operand_count != info.arity {
            return Err(SeamError::ArityMismatch {
                name: gate_name,
                expected: info.arity,
                actual: operand_count,
            });
        }

        let qubits = tracker.roots_for_operands(current);
        let angle = read_f64_attr(&current, attr::ANGLE);
        let depth_contribution = read_i64_attr(&current, attr::DEPTH_CONTRIBUTION)
            .ok_or(SeamError::MissingAttr(attr::DEPTH_CONTRIBUTION))?;
        let clifford = read_bool_attr(&current, attr::CLIFFORD)
            .ok_or(SeamError::MissingAttr(attr::CLIFFORD))?;

        gates.push(CircGate {
            name: info.id.to_string(),
            qubits,
            angle,
            depth_contribution,
            clifford,
        });
        tracker.observe_operation(current);
        op = current.next_in_block();
    }

    Ok(CircIr { n_qubits, gates })
}

// --- Rebuild ---------------------------------------------------------------

/// Rebuilds a `quantum.circ.func` body from a [`CircIr`], replacing the
/// existing gate sequence in place.
///
/// Threads fresh wires from the block arguments, applies each [`CircGate`] in
/// order using the verified `quantum.circ` builders, and repoints the
/// terminator. The original gate ops are erased only after the new ones and
/// the new terminator are wired up, so the IR is never left with dangling uses.
///
/// Returns `Err` when the func has no body/terminator, a wire index is out of
/// range, or the underlying builder fails.
pub fn rebuild<'c, 'a>(
    context: &'c Context,
    func: OperationRef<'c, 'a>,
    circ: &CircIr,
) -> Result<(), SeamError> {
    let region = func.region(0).map_err(|_| SeamError::NoBody)?;
    let block = region.first_block().ok_or(SeamError::NoBody)?;
    let location = func.location();

    let n_qubits = block.argument_count();
    if n_qubits != circ.n_qubits {
        return Err(SeamError::Build(format!(
            "n_qubits mismatch: CircIr has {} but func has {n_qubits} block args",
            circ.n_qubits
        )));
    }

    // Validate wire indices up front so we never half-rewrite the body.
    for gate in &circ.gates {
        for &index in &gate.qubits {
            if index >= n_qubits {
                return Err(SeamError::WireOutOfRange { index, n_qubits });
            }
        }
        if gate.angle.is_some() && gate.qubits.len() != 1 {
            return Err(SeamError::MultiQubitRotation(gate.name.clone()));
        }
    }

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
    let return_op = return_op.ok_or(SeamError::NoTerminator)?;

    // Thread fresh wires from the block arguments.
    let mut wires: Vec<Value<'c, 'a>> = Vec::with_capacity(n_qubits);
    for index in 0..n_qubits {
        let arg = block
            .argument(index)
            .map_err(|e| SeamError::Build(e.to_string()))?;
        wires.push(Value::from(arg));
    }

    for gate in &circ.gates {
        let operands: Vec<Value<'c, 'a>> = gate.qubits.iter().map(|&i| wires[i]).collect();
        let built = if let Some(angle) = gate.angle {
            quantum_circ::rotation_gate(
                context,
                &gate.name,
                angle,
                gate.depth_contribution,
                gate.clifford,
                wires[gate.qubits[0]],
                location,
            )
        } else {
            quantum_circ::gate(
                context,
                &gate.name,
                gate.depth_contribution,
                gate.clifford,
                &operands,
                location,
            )
        };
        let built = built.map_err(|e| SeamError::Build(e.to_string()))?;
        let appended = block.insert_operation_before(return_op, built);
        for (index, &target) in gate.qubits.iter().enumerate() {
            if let Ok(result) = appended.result(index) {
                wires[target] = Value::from(result);
            }
        }
    }

    // Insert the new terminator, then erase the old terminator and gates.
    // The new gates read block arguments only, so erasing the old ops (deepest
    // first) can never strand a live use.
    let new_return =
        quantum_circ::r#return(&wires, location).map_err(|e| SeamError::Build(e.to_string()))?;
    block.insert_operation_before(return_op, new_return);
    let rewriter = IrRewriter::new(context);
    let base = rewriter.as_rewriter_base();
    base.erase_op(return_op);
    for old in old_ops.into_iter().rev() {
        base.erase_op(old);
    }
    Ok(())
}

// --- ZX kernel interop -----------------------------------------------------

/// Converts a [`CircGate`] to the `zx` crate's [`GateRef`] for ZX translation.
///
/// Drops `depth_contribution`/`clifford` (the ZX kernel does not use them).
/// Used by the ZX simplification pass to feed the shared seam's [`CircIr`]
/// into the ZX graph kernel.
pub fn circ_gate_to_gate_ref(gate: &CircGate) -> zx::GateRef {
    zx::GateRef {
        name: gate.name.clone(),
        qubits: gate.qubits.clone(),
        angle: gate.angle,
    }
}

/// Converts a `zx` crate [`GateRef`] back to a [`CircGate`] for rebuilding.
///
/// Looks up the registry for the Clifford classification and defaults
/// `depth_contribution` to 1 (one gate per step). Used by the ZX
/// simplification pass to feed ZX results back through [`rebuild`].
pub fn gate_ref_to_circ_gate(gate: &zx::GateRef) -> CircGate {
    let clifford =
        gates::lookup(&gate.name).is_some_and(|info| info.class == gates::GateClass::Clifford);
    CircGate {
        name: gate.name.clone(),
        qubits: gate.qubits.clone(),
        angle: gate.angle,
        depth_contribution: 1,
        clifford,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use melior::Context;
    use melior::ir::Block;
    use melior::ir::BlockLike;
    use melior::ir::Identifier;
    use melior::ir::Location;
    use melior::ir::Module;
    use melior::ir::Region;
    use melior::ir::RegionLike;
    use melior::ir::Value;
    use melior::ir::attribute::BoolAttribute;
    use melior::ir::attribute::IntegerAttribute;
    use melior::ir::attribute::StringAttribute;
    use melior::ir::operation::OperationBuilder;
    use melior::ir::r#type::IntegerType;
    use quon_core::DepthExpr;

    /// A context with the `quantum.circ` dialect registered.
    fn ctx() -> Context {
        let context = Context::new();
        quantum_circ::register_dialect(&context);
        context
    }

    /// A test gate specification for building circuits.
    struct TestGate {
        name: &'static str,
        qubits: Vec<usize>,
        angle: Option<f64>,
    }

    /// Builds a module containing a single `quantum.circ.func @main` with the
    /// given qubit count and gate sequence. Non-rotation gates use
    /// `clifford = true`; rotations use `clifford = false`.
    fn build_module<'c>(context: &'c Context, n_qubits: usize, gates: &[TestGate]) -> Module<'c> {
        let location = Location::unknown(context);
        let qubit = quantum_circ::qubit_type(context);
        let block = Block::new(&vec![(qubit, location); n_qubits]);
        let mut wires: Vec<Value> = (0..n_qubits)
            .map(|i| Value::from(block.argument(i).expect("arg")))
            .collect();
        for gate in gates {
            let operands: Vec<Value> = gate.qubits.iter().map(|&q| wires[q]).collect();
            let op = if let Some(angle) = gate.angle {
                block.append_operation(
                    quantum_circ::rotation_gate(
                        context,
                        gate.name,
                        angle,
                        1,
                        false,
                        operands[0],
                        location,
                    )
                    .expect("rotation"),
                )
            } else {
                block.append_operation(
                    quantum_circ::gate(context, gate.name, 1, true, &operands, location)
                        .expect("gate"),
                )
            };
            for (i, &q) in gate.qubits.iter().enumerate() {
                wires[q] = Value::from(op.result(i).expect("result"));
            }
        }
        block.append_operation(quantum_circ::r#return(&wires, location).expect("return"));
        let region = Region::new();
        region.append_block(block);
        let all_clifford = gates.iter().all(|g| g.angle.is_none());
        let func = quantum_circ::func(
            context,
            "main",
            n_qubits as i64,
            n_qubits as i64,
            &DepthExpr::Nat(gates.len() as u64),
            all_clifford,
            region,
            location,
        )
        .expect("func");
        let module = Module::new(location);
        module.body().append_operation(func);
        module
    }

    /// Returns the func op from a module built by [`build_module`].
    fn func_op<'c, 'a>(module: &'a Module<'c>) -> OperationRef<'c, 'a> {
        module.body().first_operation().expect("func")
    }

    // ── extract → rebuild identity ────────────────────────────────────────

    #[test]
    fn extract_rebuild_identity_single_qubit_cliffords() {
        let context = ctx();
        let module = build_module(
            &context,
            1,
            &[
                TestGate {
                    name: "H",
                    qubits: vec![0],
                    angle: None,
                },
                TestGate {
                    name: "X",
                    qubits: vec![0],
                    angle: None,
                },
                TestGate {
                    name: "H",
                    qubits: vec![0],
                    angle: None,
                },
                TestGate {
                    name: "Z",
                    qubits: vec![0],
                    angle: None,
                },
            ],
        );
        let func = func_op(&module);

        let circ1 = extract(func).expect("extract");
        assert_eq!(circ1.n_qubits, 1);
        assert_eq!(circ1.gates.len(), 4);
        assert_eq!(circ1.gates[0].name, "H");
        assert_eq!(circ1.gates[1].name, "X");

        rebuild(&context, func, &circ1).expect("rebuild");
        let circ2 = extract(func).expect("extract again");
        assert_eq!(circ1, circ2);
    }

    #[test]
    fn extract_rebuild_identity_rotations() {
        let context = ctx();
        let module = build_module(
            &context,
            1,
            &[
                TestGate {
                    name: "Rz",
                    qubits: vec![0],
                    angle: Some(0.5),
                },
                TestGate {
                    name: "Rx",
                    qubits: vec![0],
                    angle: Some(0.3),
                },
                TestGate {
                    name: "Rz",
                    qubits: vec![0],
                    angle: Some(0.7),
                },
            ],
        );
        let func = func_op(&module);

        let circ1 = extract(func).expect("extract");
        assert_eq!(circ1.gates.len(), 3);
        assert_eq!(circ1.gates[0].angle, Some(0.5));

        rebuild(&context, func, &circ1).expect("rebuild");
        let circ2 = extract(func).expect("extract again");
        assert_eq!(circ1, circ2);
    }

    #[test]
    fn extract_rebuild_identity_cnot() {
        let context = ctx();
        let module = build_module(
            &context,
            2,
            &[
                TestGate {
                    name: "H",
                    qubits: vec![0],
                    angle: None,
                },
                TestGate {
                    name: "CNOT",
                    qubits: vec![0, 1],
                    angle: None,
                },
                TestGate {
                    name: "H",
                    qubits: vec![1],
                    angle: None,
                },
            ],
        );
        let func = func_op(&module);

        let circ1 = extract(func).expect("extract");
        assert_eq!(circ1.n_qubits, 2);
        assert_eq!(circ1.gates.len(), 3);

        rebuild(&context, func, &circ1).expect("rebuild");
        let circ2 = extract(func).expect("extract again");
        assert_eq!(circ1, circ2);
    }

    #[test]
    fn extract_rebuild_identity_cz() {
        let context = ctx();
        let module = build_module(
            &context,
            2,
            &[
                TestGate {
                    name: "CZ",
                    qubits: vec![0, 1],
                    angle: None,
                },
                TestGate {
                    name: "H",
                    qubits: vec![0],
                    angle: None,
                },
                TestGate {
                    name: "H",
                    qubits: vec![1],
                    angle: None,
                },
                TestGate {
                    name: "CZ",
                    qubits: vec![1, 0],
                    angle: None,
                },
            ],
        );
        let func = func_op(&module);

        let circ1 = extract(func).expect("extract");
        rebuild(&context, func, &circ1).expect("rebuild");
        let circ2 = extract(func).expect("extract again");
        assert_eq!(circ1, circ2);
    }

    #[test]
    fn extract_rebuild_identity_swap_sequence() {
        let context = ctx();
        let module = build_module(
            &context,
            3,
            &[
                TestGate {
                    name: "SWAP",
                    qubits: vec![0, 1],
                    angle: None,
                },
                TestGate {
                    name: "CNOT",
                    qubits: vec![1, 2],
                    angle: None,
                },
                TestGate {
                    name: "SWAP",
                    qubits: vec![2, 0],
                    angle: None,
                },
                TestGate {
                    name: "H",
                    qubits: vec![1],
                    angle: None,
                },
            ],
        );
        let func = func_op(&module);

        let circ1 = extract(func).expect("extract");
        rebuild(&context, func, &circ1).expect("rebuild");
        let circ2 = extract(func).expect("extract again");
        assert_eq!(circ1, circ2);
    }

    #[test]
    fn extract_rebuild_identity_empty_func() {
        let context = ctx();
        let module = build_module(&context, 1, &[]);
        let func = func_op(&module);

        let circ1 = extract(func).expect("extract");
        assert!(circ1.gates.is_empty());

        rebuild(&context, func, &circ1).expect("rebuild");
        let circ2 = extract(func).expect("extract again");
        assert_eq!(circ1, circ2);
    }

    // ── faithful multi-qubit wire tracking ────────────────────────────────

    #[test]
    fn extract_tracks_multi_qubit_wires_faithfully() {
        // CNOT(0, 1) then H(1): the H is on wire 1, not wire 0.
        // The old operand-position encoding would have recorded [0] for the H.
        let context = ctx();
        let module = build_module(
            &context,
            2,
            &[
                TestGate {
                    name: "CNOT",
                    qubits: vec![0, 1],
                    angle: None,
                },
                TestGate {
                    name: "H",
                    qubits: vec![1],
                    angle: None,
                },
            ],
        );
        let func = func_op(&module);
        let circ = extract(func).expect("extract");

        assert_eq!(circ.gates[0].qubits, vec![0, 1]); // CNOT on wires 0, 1
        assert_eq!(circ.gates[1].qubits, vec![1]); // H on wire 1 (faithful!)
    }

    #[test]
    fn extract_tracks_wires_through_three_qubit_circuit() {
        // H(0) → CNOT(0,1) → CNOT(1,2) → H(2): wire tracking across a chain.
        let context = ctx();
        let module = build_module(
            &context,
            3,
            &[
                TestGate {
                    name: "H",
                    qubits: vec![0],
                    angle: None,
                },
                TestGate {
                    name: "CNOT",
                    qubits: vec![0, 1],
                    angle: None,
                },
                TestGate {
                    name: "CNOT",
                    qubits: vec![1, 2],
                    angle: None,
                },
                TestGate {
                    name: "H",
                    qubits: vec![2],
                    angle: None,
                },
            ],
        );
        let func = func_op(&module);
        let circ = extract(func).expect("extract");

        assert_eq!(circ.gates[0].qubits, vec![0]);
        assert_eq!(circ.gates[1].qubits, vec![0, 1]);
        assert_eq!(circ.gates[2].qubits, vec![1, 2]);
        assert_eq!(circ.gates[3].qubits, vec![2]);
    }

    // ── canonical name resolution ─────────────────────────────────────────

    #[test]
    fn extract_canonicalizes_gate_aliases() {
        // "CX" is an alias for "CNOT" in the registry.
        let context = ctx();
        let module = build_module(
            &context,
            2,
            &[TestGate {
                name: "CX",
                qubits: vec![0, 1],
                angle: None,
            }],
        );
        let func = func_op(&module);
        let circ = extract(func).expect("extract");

        // The canonical id is "CNOT", not "CX".
        assert_eq!(circ.gates[0].name, "CNOT");
    }

    // ── decline (fail closed) ─────────────────────────────────────────────

    #[test]
    fn extract_declines_unknown_gate() {
        let context = ctx();
        let module = build_module(
            &context,
            1,
            &[TestGate {
                name: "FOO",
                qubits: vec![0],
                angle: None,
            }],
        );
        let func = func_op(&module);
        let result = extract(func);
        assert!(matches!(result, Err(SeamError::UnknownGate(_))));
    }

    #[test]
    fn extract_declines_non_gate_op() {
        // A func body containing a non-gate, non-return op must be declined.
        let context = ctx();
        let location = Location::unknown(&context);
        let qubit = quantum_circ::qubit_type(&context);
        let block = Block::new(&[(qubit, location)]);
        let wire = Value::from(block.argument(0).expect("arg"));

        // A gate, then a foreign op (not quantum.circ.gate / .return).
        let gate = block.append_operation(
            quantum_circ::gate(&context, "H", 1, true, &[wire], location).expect("gate"),
        );
        let wire = Value::from(gate.result(0).expect("result"));

        let foreign = OperationBuilder::new("quantum.circ.compose", location)
            .add_operands(&[wire])
            .add_results(&[qubit])
            .build()
            .expect("build foreign");
        let foreign = block.append_operation(foreign);
        let wire = Value::from(foreign.result(0).expect("result"));

        block.append_operation(quantum_circ::r#return(&[wire], location).expect("return"));
        let region = Region::new();
        region.append_block(block);
        let func = quantum_circ::func(
            &context,
            "main",
            1,
            1,
            &DepthExpr::Nat(2),
            true,
            region,
            location,
        )
        .expect("func");
        let module = Module::new(location);
        module.body().append_operation(func);
        let func_ref = module.body().first_operation().expect("func");

        let result = extract(func_ref);
        match &result {
            Err(SeamError::UnsupportedConstruct(name)) => {
                assert_eq!(name, "quantum.circ.compose");
            }
            _ => panic!("expected UnsupportedConstruct, got {result:?}"),
        }
    }

    #[test]
    fn extract_declines_no_body() {
        // A func with an empty region (no entry block) should decline.
        // Built with OperationBuilder to bypass the func verifier (which
        // requires a block).
        let context = ctx();
        let location = Location::unknown(&context);
        let i64_type = IntegerType::new(&context, 64).into();
        let region = Region::new(); // no block appended
        let func = OperationBuilder::new(quantum_circ::op::FUNC, location)
            .add_attributes(&[
                (
                    Identifier::new(&context, quantum_circ::attr::SYM_NAME),
                    StringAttribute::new(&context, "main").into(),
                ),
                (
                    Identifier::new(&context, quantum_circ::attr::IN_QUBITS),
                    IntegerAttribute::new(i64_type, 1).into(),
                ),
                (
                    Identifier::new(&context, quantum_circ::attr::OUT_QUBITS),
                    IntegerAttribute::new(i64_type, 1).into(),
                ),
                (
                    Identifier::new(&context, quantum_circ::attr::DEPTH),
                    StringAttribute::new(&context, "0").into(),
                ),
                (
                    Identifier::new(&context, quantum_circ::attr::CLIFFORD),
                    BoolAttribute::new(&context, true).into(),
                ),
            ])
            .add_regions([region])
            .build()
            .expect("func without body");
        let module = Module::new(location);
        module.body().append_operation(func);
        let func_ref = module.body().first_operation().expect("func");

        let result = extract(func_ref);
        assert!(matches!(result, Err(SeamError::NoBody)));
    }

    // ── rebuild validation ────────────────────────────────────────────────

    #[test]
    fn rebuild_rejects_out_of_range_wire() {
        let context = ctx();
        let module = build_module(&context, 1, &[]);
        let func = func_op(&module);

        let bad_circ = CircIr {
            n_qubits: 1,
            gates: vec![CircGate {
                name: "H".to_string(),
                qubits: vec![5], // out of range
                angle: None,
                depth_contribution: 1,
                clifford: true,
            }],
        };
        let result = rebuild(&context, func, &bad_circ);
        assert!(matches!(
            result,
            Err(SeamError::WireOutOfRange { index: 5, .. })
        ));
    }

    #[test]
    fn rebuild_rejects_multi_qubit_rotation() {
        let context = ctx();
        let module = build_module(&context, 2, &[]);
        let func = func_op(&module);

        let bad_circ = CircIr {
            n_qubits: 2,
            gates: vec![CircGate {
                name: "Rz".to_string(),
                qubits: vec![0, 1], // rotation on 2 qubits
                angle: Some(0.5),
                depth_contribution: 1,
                clifford: false,
            }],
        };
        let result = rebuild(&context, func, &bad_circ);
        assert!(matches!(result, Err(SeamError::MultiQubitRotation(_))));
    }

    // ── ZX kernel interop ─────────────────────────────────────────────────

    #[test]
    fn circ_gate_to_gate_ref_roundtrip() {
        let circ_gate = CircGate {
            name: "CNOT".to_string(),
            qubits: vec![0, 1],
            angle: None,
            depth_contribution: 1,
            clifford: true,
        };
        let gate_ref = circ_gate_to_gate_ref(&circ_gate);
        assert_eq!(gate_ref.name, "CNOT");
        assert_eq!(gate_ref.qubits, vec![0, 1]);
        assert_eq!(gate_ref.angle, None);

        let back = gate_ref_to_circ_gate(&gate_ref);
        assert_eq!(back.name, "CNOT");
        assert_eq!(back.qubits, vec![0, 1]);
        assert!(back.clifford); // CNOT is Clifford
    }

    #[test]
    fn gate_ref_to_circ_gate_marks_non_clifford() {
        let gate_ref = zx::GateRef::new("T", vec![0]);
        let circ_gate = gate_ref_to_circ_gate(&gate_ref);
        assert!(!circ_gate.clifford); // T is Universal
    }
}
