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

