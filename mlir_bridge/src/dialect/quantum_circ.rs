//! `quantum.circ` dialect — registration, types, op builders, and verifiers.
//!
//! `quantum.circ` is the purely unitary IR dialect (CONTEXT.md, SPEC §6.2). All
//! ops are unitary; there is no measurement. Every `!qubit` SSA value has
//! exactly one use, enforced by the standalone linearity pass
//! ([`crate::passes::linearity_verifier`]).
//!
//! ## Registration strategy
//!
//! ADR-0001 specifies Melior's `#[melior::dialect]` proc-macro for custom
//! dialects. In practice that macro generates *typed Rust wrappers* for a
//! dialect whose ops and types are already defined in C++/TableGen — it cannot
//! introduce a brand-new dialect from pure Rust. Rather than vendor a C++
//! TableGen build, we take ADR-0001's stated fallback ("dropping to raw C API
//! calls for dialect registration"): `quantum.circ` is modeled as an MLIR
//! **unregistered dialect**.
//!
//! Concretely:
//!
//!   * Ops round-trip through MLIR's generic operation form
//!     (`"quantum.circ.gate"(%q) {...} : (!quantum.qubit) -> !quantum.qubit`).
//!   * Qubit and circuit values use opaque dialect types — `!quantum.qubit`
//!     (the source-language `!qubit`) and `!quantum.circ` (the unparameterized
//!     circuit value type, per ADR-0002).
//!   * Op verifier callbacks run as explicit Rust functions ([`verify`]) rather
//!     than C++ verification hooks. The op *builders* in this module run the
//!     verifier on every op they construct, so a malformed op cannot escape.
//!
//! Index attributes (`in_qubits`, `out_qubits`, `depth`, `clifford`) live on the
//! ops, not on a parameterized type (ADR-0002). `depth` carries a serialized
//! [`DepthExpr`] S-expression.

use melior::Context;
use melior::ir::attribute::{AttributeLike, BoolAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::{OperationBuilder, OperationLike};
use melior::ir::r#type::IntegerType;
use melior::ir::{
    Attribute, BlockLike, Identifier, Location, Operation, Region, RegionLike, Type, Value,
    ValueLike,
};
use thiserror::Error;

use quon_core::DepthExpr;

/// The dialect namespace.
pub const NAMESPACE: &str = "quantum.circ";

/// Opaque MLIR type name for a single linear qubit (source-language `!qubit`).
pub const QUBIT_TYPE: &str = "!quantum.qubit";

/// Opaque MLIR type name for a circuit value (`Circuit<n, m, d, C>`, ADR-0002).
pub const CIRCUIT_TYPE: &str = "!quantum.circ";

/// Operation names.
pub mod op {
    /// A circuit definition: a named region with `in_qubits` qubit arguments.
    pub const FUNC: &str = "quantum.circ.func";
    /// A primitive gate threading qubits: `[!qubit] -> [!qubit]`.
    pub const GATE: &str = "quantum.circ.gate";
    /// Sequential composition of two circuits (depth adds).
    pub const COMPOSE: &str = "quantum.circ.compose";
    /// Parallel composition of two circuits (depth is the max).
    pub const TENSOR: &str = "quantum.circ.tensor";
    /// Circuit inverse (depth preserved).
    pub const ADJOINT: &str = "quantum.circ.adjoint";
    /// Control a circuit on a qubit (depth + 1).
    pub const CONTROLLED: &str = "quantum.circ.controlled";
    /// Scoped ancilla allocation producing fresh qubits.
    pub const BORROW: &str = "quantum.circ.borrow";
    /// Terminator of a `func` region, consuming the output qubits. Auxiliary —
    /// not one of the seven dialect ops, but required to make a region linear.
    pub const RETURN: &str = "quantum.circ.return";
}

/// The seven `quantum.circ` ops that register at dialect initialization.
pub const OPS: [&str; 7] = [
    op::FUNC,
    op::GATE,
    op::COMPOSE,
    op::TENSOR,
    op::ADJOINT,
    op::CONTROLLED,
    op::BORROW,
];

/// Attribute names.
pub mod attr {
    /// Symbol name of a `func`.
    pub const SYM_NAME: &str = "sym_name";
    /// Number of input qubits (`I64Attr`).
    pub const IN_QUBITS: &str = "in_qubits";
    /// Number of output qubits (`I64Attr`).
    pub const OUT_QUBITS: &str = "out_qubits";
    /// Symbolic depth bound, a [`DepthExpr`] S-expression (`DepthExprAttr`).
    pub const DEPTH: &str = "depth";
    /// Clifford classification (`BoolAttr`).
    pub const CLIFFORD: &str = "clifford";
    /// Gate name on a `gate` op.
    pub const GATE_NAME: &str = "gate_name";
    /// Per-gate depth contribution (`I64Attr`).
    pub const DEPTH_CONTRIBUTION: &str = "depth_contribution";
}

/// Registers the `quantum.circ` dialect with `context`.
///
/// See the module docs for why this configures an unregistered dialect rather
/// than loading a C++ one. Idempotent and panic-free.
pub fn register_dialect(context: &Context) {
    context.set_allow_unregistered_dialects(true);
}

/// The MLIR type of a single linear qubit value.
pub fn qubit_type(context: &Context) -> Type<'_> {
    Type::parse(context, QUBIT_TYPE).unwrap_or_else(|| Type::none(context))
}

/// The MLIR type of a circuit value.
pub fn circuit_type(context: &Context) -> Type<'_> {
    Type::parse(context, CIRCUIT_TYPE).unwrap_or_else(|| Type::none(context))
}

fn i64_type(context: &Context) -> Type<'_> {
    IntegerType::new(context, 64).into()
}

/// True if `r#type` is the opaque `!quantum.qubit` type.
///
/// The dialect's single source of truth for qubit identity — verifiers and
/// passes call this rather than re-deriving the printed-form comparison. Per
/// ADR-0004, identity is by printed form because the dialect is unregistered.
pub fn is_qubit_type(r#type: Type) -> bool {
    r#type.to_string() == QUBIT_TYPE
}

/// True if `r#type` is the opaque `!quantum.circ` circuit-value type.
pub fn is_circuit_type(r#type: Type) -> bool {
    r#type.to_string() == CIRCUIT_TYPE
}

// --- Errors ----------------------------------------------------------------

/// A structural problem with a `quantum.circ` op, found by [`verify`].
///
/// These are pure data — they carry no MLIR handle. The dialect driver and the
/// linearity pass fold them into [`crate::diagnostics::Diagnostics`] at the
/// op's location; see [`crate::diagnostics`] for the FFI boundary.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum VerifyError {
    /// A required attribute is absent.
    #[error("{op}: missing required attribute `{attr}`")]
    MissingAttribute {
        /// Op name.
        op: &'static str,
        /// Attribute name.
        attr: &'static str,
    },
    /// An attribute is present but of the wrong kind.
    #[error("{op}: attribute `{attr}` must be of type {expected}")]
    WrongAttributeType {
        /// Op name.
        op: &'static str,
        /// Attribute name.
        attr: &'static str,
        /// Expected attribute kind.
        expected: &'static str,
    },
    /// The op has the wrong number of operands or results.
    #[error("{op}: expected {expected} {role}(s), found {found}")]
    Arity {
        /// Op name.
        op: &'static str,
        /// `"operand"` or `"result"`.
        role: &'static str,
        /// Expected count, as text (may be a bound like `">= 1"`).
        expected: String,
        /// Actual count.
        found: usize,
    },
    /// An operand, result, or block argument has the wrong type.
    #[error("{op}: {role} #{index} must have type {expected}")]
    WrongValueType {
        /// Op name.
        op: &'static str,
        /// Value role, e.g. `"operand"`.
        role: &'static str,
        /// Position.
        index: usize,
        /// Expected type name.
        expected: &'static str,
    },
    /// A qubit count attribute is negative.
    #[error("{op}: `{attr}` ({value}) must be non-negative")]
    NegativeCount {
        /// Op name.
        op: &'static str,
        /// Attribute name.
        attr: &'static str,
        /// Offending value.
        value: i64,
    },
    /// A region-bearing op is missing its region or block.
    #[error("{op}: expected a region with an entry block")]
    MissingRegion {
        /// Op name.
        op: &'static str,
    },
    /// A `func`'s entry block arity does not match `in_qubits`.
    #[error("{op}: entry block has {found} argument(s) but in_qubits is {expected}")]
    ArgCountMismatch {
        /// Op name.
        op: &'static str,
        /// Declared `in_qubits`.
        expected: i64,
        /// Actual block argument count.
        found: usize,
    },
}

/// An error constructing a `quantum.circ` op.
#[derive(Debug, Error)]
pub enum BuildError {
    /// The underlying Melior operation builder failed.
    #[error("failed to build operation: {0}")]
    Mlir(#[from] melior::Error),
    /// The constructed op failed verification.
    #[error("operation failed verification: {0}")]
    Verify(#[from] VerifyError),
}

// --- Verifier --------------------------------------------------------------

/// Verifies a single `quantum.circ` op against its structural invariants.
///
/// Ops outside the dialect are accepted unchanged (`Ok`). This is the explicit
/// stand-in for a C++ op verifier callback under the unregistered-dialect model.
pub fn verify<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    let identifier = operation.name();
    let name_ref = identifier.as_string_ref();
    match name_ref.as_str().unwrap_or("") {
        op::FUNC => verify_func(operation),
        op::GATE => verify_gate(operation),
        op::COMPOSE => verify_binary_circuit(operation, op::COMPOSE),
        op::TENSOR => verify_binary_circuit(operation, op::TENSOR),
        op::ADJOINT => verify_adjoint(operation),
        op::CONTROLLED => verify_controlled(operation),
        op::BORROW => verify_borrow(operation),
        op::RETURN => verify_return(operation),
        _ => Ok(()),
    }
}

fn require_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<Attribute<'c>, VerifyError> {
    operation
        .attribute(attr)
        .map_err(|_| VerifyError::MissingAttribute { op, attr })
}

fn require_string<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<(), VerifyError> {
    let value = require_attr(operation, op, attr)?;
    if value.is_string() {
        Ok(())
    } else {
        Err(VerifyError::WrongAttributeType {
            op,
            attr,
            expected: "string",
        })
    }
}

fn require_integer<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<i64, VerifyError> {
    let value = require_attr(operation, op, attr)?;
    IntegerAttribute::try_from(value)
        .map(|integer| integer.value())
        .map_err(|_| VerifyError::WrongAttributeType {
            op,
            attr,
            expected: "integer",
        })
}

fn require_bool<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<(), VerifyError> {
    let value = require_attr(operation, op, attr)?;
    if value.is_bool() {
        Ok(())
    } else {
        Err(VerifyError::WrongAttributeType {
            op,
            attr,
            expected: "bool",
        })
    }
}

fn require_depth<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
) -> Result<(), VerifyError> {
    // The depth S-expression is carried as a string attribute (ADR-0002).
    require_string(operation, op, attr::DEPTH)
}

fn expect_counts<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    operands: usize,
    results: usize,
) -> Result<(), VerifyError> {
    if operation.operand_count() != operands {
        return Err(VerifyError::Arity {
            op,
            role: "operand",
            expected: operands.to_string(),
            found: operation.operand_count(),
        });
    }
    if operation.result_count() != results {
        return Err(VerifyError::Arity {
            op,
            role: "result",
            expected: results.to_string(),
            found: operation.result_count(),
        });
    }
    Ok(())
}

fn expect_operand_type<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    index: usize,
    predicate: fn(Type) -> bool,
    expected: &'static str,
) -> Result<(), VerifyError> {
    let value = operation.operand(index).map_err(|_| VerifyError::Arity {
        op,
        role: "operand",
        expected: format!("> {index}"),
        found: operation.operand_count(),
    })?;
    if predicate(value.r#type()) {
        Ok(())
    } else {
        Err(VerifyError::WrongValueType {
            op,
            role: "operand",
            index,
            expected,
        })
    }
}

fn expect_result_type<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    index: usize,
    predicate: fn(Type) -> bool,
    expected: &'static str,
) -> Result<(), VerifyError> {
    let value = operation.result(index).map_err(|_| VerifyError::Arity {
        op,
        role: "result",
        expected: format!("> {index}"),
        found: operation.result_count(),
    })?;
    if predicate(value.r#type()) {
        Ok(())
    } else {
        Err(VerifyError::WrongValueType {
            op,
            role: "result",
            index,
            expected,
        })
    }
}

fn verify_func<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    require_string(operation, op::FUNC, attr::SYM_NAME)?;
    let in_qubits = require_integer(operation, op::FUNC, attr::IN_QUBITS)?;
    let out_qubits = require_integer(operation, op::FUNC, attr::OUT_QUBITS)?;
    if in_qubits < 0 {
        return Err(VerifyError::NegativeCount {
            op: op::FUNC,
            attr: attr::IN_QUBITS,
            value: in_qubits,
        });
    }
    if out_qubits < 0 {
        return Err(VerifyError::NegativeCount {
            op: op::FUNC,
            attr: attr::OUT_QUBITS,
            value: out_qubits,
        });
    }
    require_depth(operation, op::FUNC)?;
    require_bool(operation, op::FUNC, attr::CLIFFORD)?;

    let region = operation
        .region(0)
        .map_err(|_| VerifyError::MissingRegion { op: op::FUNC })?;
    let block = region
        .first_block()
        .ok_or(VerifyError::MissingRegion { op: op::FUNC })?;
    let arguments = block.argument_count();
    if arguments as i64 != in_qubits {
        return Err(VerifyError::ArgCountMismatch {
            op: op::FUNC,
            expected: in_qubits,
            found: arguments,
        });
    }
    for index in 0..arguments {
        let argument = block.argument(index).map_err(|_| VerifyError::Arity {
            op: op::FUNC,
            role: "block argument",
            expected: format!("> {index}"),
            found: arguments,
        })?;
        if !is_qubit_type(argument.r#type()) {
            return Err(VerifyError::WrongValueType {
                op: op::FUNC,
                role: "block argument",
                index,
                expected: QUBIT_TYPE,
            });
        }
    }
    Ok(())
}

fn verify_gate<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    require_string(operation, op::GATE, attr::GATE_NAME)?;
    let contribution = require_integer(operation, op::GATE, attr::DEPTH_CONTRIBUTION)?;
    require_bool(operation, op::GATE, attr::CLIFFORD)?;
    if contribution < 0 {
        return Err(VerifyError::NegativeCount {
            op: op::GATE,
            attr: attr::DEPTH_CONTRIBUTION,
            value: contribution,
        });
    }
    let operands = operation.operand_count();
    if operands == 0 {
        return Err(VerifyError::Arity {
            op: op::GATE,
            role: "operand",
            expected: ">= 1".to_string(),
            found: 0,
        });
    }
    if operation.result_count() != operands {
        return Err(VerifyError::Arity {
            op: op::GATE,
            role: "result",
            expected: operands.to_string(),
            found: operation.result_count(),
        });
    }
    for index in 0..operands {
        expect_operand_type(operation, op::GATE, index, is_qubit_type, QUBIT_TYPE)?;
        expect_result_type(operation, op::GATE, index, is_qubit_type, QUBIT_TYPE)?;
    }
    Ok(())
}

fn verify_binary_circuit<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
) -> Result<(), VerifyError> {
    expect_counts(operation, op, 2, 1)?;
    expect_operand_type(operation, op, 0, is_circuit_type, CIRCUIT_TYPE)?;
    expect_operand_type(operation, op, 1, is_circuit_type, CIRCUIT_TYPE)?;
    expect_result_type(operation, op, 0, is_circuit_type, CIRCUIT_TYPE)?;
    require_depth(operation, op)
}

fn verify_adjoint<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    expect_counts(operation, op::ADJOINT, 1, 1)?;
    expect_operand_type(operation, op::ADJOINT, 0, is_circuit_type, CIRCUIT_TYPE)?;
    expect_result_type(operation, op::ADJOINT, 0, is_circuit_type, CIRCUIT_TYPE)?;
    require_depth(operation, op::ADJOINT)
}

fn verify_controlled<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
) -> Result<(), VerifyError> {
    expect_counts(operation, op::CONTROLLED, 2, 1)?;
    expect_operand_type(operation, op::CONTROLLED, 0, is_circuit_type, CIRCUIT_TYPE)?;
    expect_operand_type(operation, op::CONTROLLED, 1, is_qubit_type, QUBIT_TYPE)?;
    expect_result_type(operation, op::CONTROLLED, 0, is_circuit_type, CIRCUIT_TYPE)?;
    require_depth(operation, op::CONTROLLED)
}

fn verify_borrow<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    let results = operation.result_count();
    if results == 0 {
        return Err(VerifyError::Arity {
            op: op::BORROW,
            role: "result",
            expected: ">= 1".to_string(),
            found: 0,
        });
    }
    for index in 0..results {
        expect_result_type(operation, op::BORROW, index, is_qubit_type, QUBIT_TYPE)?;
    }
    operation
        .region(0)
        .map_err(|_| VerifyError::MissingRegion { op: op::BORROW })?
        .first_block()
        .ok_or(VerifyError::MissingRegion { op: op::BORROW })?;
    require_depth(operation, op::BORROW)
}

fn verify_return<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    if operation.result_count() != 0 {
        return Err(VerifyError::Arity {
            op: op::RETURN,
            role: "result",
            expected: "0".to_string(),
            found: operation.result_count(),
        });
    }
    for index in 0..operation.operand_count() {
        expect_operand_type(operation, op::RETURN, index, is_qubit_type, QUBIT_TYPE)?;
    }
    Ok(())
}

// --- Builders --------------------------------------------------------------

/// The `depth` attribute pair carried by every depth-bearing op: a serialized
/// [`DepthExpr`] S-expression (ADR-0002). One definition for all builders.
fn depth_attribute<'c>(context: &'c Context, depth: &DepthExpr) -> (Identifier<'c>, Attribute<'c>) {
    (
        Identifier::new(context, attr::DEPTH),
        StringAttribute::new(context, &depth.to_sexpr()).into(),
    )
}

/// Builds an op and runs the dialect [`verify`]er on it. The single place the
/// build→verify→return tail lives, so a malformed op can never escape a builder.
fn finish(builder: OperationBuilder) -> Result<Operation, BuildError> {
    let operation = builder.build()?;
    verify(&operation)?;
    Ok(operation)
}

/// Builds a `quantum.circ.func` op from a populated region.
#[allow(clippy::too_many_arguments)]
pub fn func<'c>(
    context: &'c Context,
    name: &str,
    in_qubits: i64,
    out_qubits: i64,
    depth: &DepthExpr,
    clifford: bool,
    body: Region<'c>,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    let i64_type = i64_type(context);
    finish(
        OperationBuilder::new(op::FUNC, location)
            .add_attributes(&[
                (
                    Identifier::new(context, attr::SYM_NAME),
                    StringAttribute::new(context, name).into(),
                ),
                (
                    Identifier::new(context, attr::IN_QUBITS),
                    IntegerAttribute::new(i64_type, in_qubits).into(),
                ),
                (
                    Identifier::new(context, attr::OUT_QUBITS),
                    IntegerAttribute::new(i64_type, out_qubits).into(),
                ),
                depth_attribute(context, depth),
                (
                    Identifier::new(context, attr::CLIFFORD),
                    BoolAttribute::new(context, clifford).into(),
                ),
            ])
            .add_regions([body]),
    )
}

/// Builds a `quantum.circ.gate` op threading `qubits` through a named gate.
pub fn gate<'c>(
    context: &'c Context,
    name: &str,
    depth_contribution: i64,
    clifford: bool,
    qubits: &[Value<'c, '_>],
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    let results = vec![qubit_type(context); qubits.len()];
    finish(
        OperationBuilder::new(op::GATE, location)
            .add_operands(qubits)
            .add_results(&results)
            .add_attributes(&[
                (
                    Identifier::new(context, attr::GATE_NAME),
                    StringAttribute::new(context, name).into(),
                ),
                (
                    Identifier::new(context, attr::DEPTH_CONTRIBUTION),
                    IntegerAttribute::new(i64_type(context), depth_contribution).into(),
                ),
                (
                    Identifier::new(context, attr::CLIFFORD),
                    BoolAttribute::new(context, clifford).into(),
                ),
            ]),
    )
}

fn binary_circuit<'c>(
    context: &'c Context,
    op: &str,
    lhs: Value<'c, '_>,
    rhs: Value<'c, '_>,
    depth: &DepthExpr,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op, location)
            .add_operands(&[lhs, rhs])
            .add_results(&[circuit_type(context)])
            .add_attributes(&[depth_attribute(context, depth)]),
    )
}

/// Builds a `quantum.circ.compose` op (sequential composition; depth adds).
pub fn compose<'c>(
    context: &'c Context,
    lhs: Value<'c, '_>,
    rhs: Value<'c, '_>,
    depth: &DepthExpr,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    binary_circuit(context, op::COMPOSE, lhs, rhs, depth, location)
}

/// Builds a `quantum.circ.tensor` op (parallel composition; depth is the max).
pub fn tensor<'c>(
    context: &'c Context,
    lhs: Value<'c, '_>,
    rhs: Value<'c, '_>,
    depth: &DepthExpr,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    binary_circuit(context, op::TENSOR, lhs, rhs, depth, location)
}

/// Builds a `quantum.circ.adjoint` op (circuit inverse; depth preserved).
pub fn adjoint<'c>(
    context: &'c Context,
    circuit: Value<'c, '_>,
    depth: &DepthExpr,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::ADJOINT, location)
            .add_operands(&[circuit])
            .add_results(&[circuit_type(context)])
            .add_attributes(&[depth_attribute(context, depth)]),
    )
}

/// Builds a `quantum.circ.controlled` op (control on a qubit; depth + 1).
pub fn controlled<'c>(
    context: &'c Context,
    circuit: Value<'c, '_>,
    control: Value<'c, '_>,
    depth: &DepthExpr,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::CONTROLLED, location)
            .add_operands(&[circuit, control])
            .add_results(&[circuit_type(context)])
            .add_attributes(&[depth_attribute(context, depth)]),
    )
}

/// Builds a `quantum.circ.borrow` op allocating `count` fresh ancilla qubits.
pub fn borrow<'c>(
    context: &'c Context,
    count: usize,
    depth: &DepthExpr,
    body: Region<'c>,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    let results = vec![qubit_type(context); count];
    finish(
        OperationBuilder::new(op::BORROW, location)
            .add_results(&results)
            .add_attributes(&[depth_attribute(context, depth)])
            .add_regions([body]),
    )
}

/// Builds the `quantum.circ.return` terminator consuming a region's outputs.
pub fn r#return<'c>(
    qubits: &[Value<'c, '_>],
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(OperationBuilder::new(op::RETURN, location).add_operands(qubits))
}
