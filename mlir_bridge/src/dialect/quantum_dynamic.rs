//! `quantum.dynamic` dialect — registration, types, op builders, and verifiers.
//!
//! `quantum.dynamic` is the dynamic-circuit IR dialect (CONTEXT.md, SPEC §6.3).
//! It adds measurement, reset, feed-forward control, barriers, and embedded
//! `quantum.circ` unitary islands. Qubit linearity and reuse-after-measure are
//! enforced by [`crate::passes::dynamic_linearity_verifier`], not per-op verify.
//!
//! Registration follows ADR-0004 (unregistered dialect), mirroring
//! [`super::quantum_circ`].

use melior::Context;
use melior::ir::attribute::{
    AttributeLike, BoolAttribute, FloatAttribute, IntegerAttribute, StringAttribute,
};
use melior::ir::operation::{OperationBuilder, OperationLike};
use melior::ir::{
    Attribute, BlockLike, Identifier, Location, Operation, Region, RegionLike, RegionRef, Type,
    Value, ValueLike,
};
use quon_core::DepthExpr;
use quon_core::linearity;
use thiserror::Error;

use super::quantum_circ;

/// The dialect namespace.
pub const NAMESPACE: &str = "quantum.dynamic";

/// Opaque MLIR type for a classical measurement result (source-language `Bit`).
pub const BIT_TYPE: &str = "!quantum.bit";

/// The five `quantum.dynamic` ops registered at dialect initialization.
pub const OPS: [&str; 5] = [
    op::MEASURE,
    op::RESET,
    op::UNITARY_REGION,
    op::IF,
    op::BARRIER,
];

/// Operation names.
pub mod op {
    /// Consumes a qubit and produces a classical bit.
    pub const MEASURE: &str = "quantum.dynamic.measure";
    /// Measure-and-reprepare to |0⟩; consume-then-produce on qubit SSA.
    pub const RESET: &str = "quantum.dynamic.reset";
    /// Embedded purely unitary block containing only `quantum.circ` ops.
    pub const UNITARY_REGION: &str = "quantum.dynamic.unitary_region";
    /// Classical branch on a measurement bit with qubit threading.
    pub const IF: &str = "quantum.dynamic.if";
    /// Synchronization point; identity threading on qubits.
    pub const BARRIER: &str = "quantum.dynamic.barrier";
    /// Terminator for `if` branch regions. Auxiliary — not one of the five ops.
    pub const YIELD: &str = "quantum.dynamic.yield";
}

/// Attribute names.
pub mod attr {
    /// Physical qubit index (`I32Attr`), optional until physical lowering.
    pub const PHYS_QUBIT: &str = "phys_qubit";
    /// Whether the op maps to a native gate (`BoolAttr`), optional.
    pub const NATIVE_GATE: &str = "native_gate";
    /// Per-op fidelity from the noise model (`F64Attr`), optional.
    pub const FIDELITY: &str = "fidelity";
    /// Symbolic depth bound on `unitary_region` (`DepthExprAttr` string).
    pub const DEPTH: &str = "depth";
    /// Clifford classification on `unitary_region` (`BoolAttr`).
    pub const CLIFFORD: &str = "clifford";
}

/// Registers the `quantum.dynamic` dialect with `context`. Idempotent and panic-free.
pub fn register_dialect(context: &Context) {
    context.set_allow_unregistered_dialects(true);
}

/// The MLIR type of a classical measurement result.
pub fn bit_type(context: &Context) -> Type<'_> {
    Type::parse(context, BIT_TYPE).unwrap_or_else(|| Type::none(context))
}

/// True if `r#type` is the opaque `!quantum.bit` type.
pub fn is_bit_type(r#type: Type) -> bool {
    r#type.to_string() == BIT_TYPE
}

// --- Errors ----------------------------------------------------------------

/// A structural problem with a `quantum.dynamic` op, found by [`verify`].
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
    /// A region-bearing op is missing its region or block.
    #[error("{op}: expected a region with an entry block")]
    MissingRegion {
        /// Op name.
        op: &'static str,
    },
    /// Entry block arity does not match the enclosing op's qubit operands.
    #[error("{op}: entry block has {found} argument(s) but expected {expected}")]
    ArgCountMismatch {
        /// Op name.
        op: &'static str,
        /// Expected block argument count.
        expected: usize,
        /// Actual block argument count.
        found: usize,
    },
    /// A `unitary_region` contains a non-`quantum.circ` op.
    #[error("{op}: region contains `{found}`; only quantum.circ ops allowed (SPEC §6.3)")]
    ForbiddenOpInUnitaryRegion {
        /// Op name.
        op: &'static str,
        /// Offending op name.
        found: String,
    },
    /// Operand/result counts disagree across a region boundary.
    #[error("{op}: {role} count {outer} does not match inner region {inner}")]
    UnitaryRegionBoundaryMismatch {
        /// Op name.
        op: &'static str,
        /// `"operand"` or `"result"`.
        role: &'static str,
        /// Outer count.
        outer: usize,
        /// Inner count.
        inner: usize,
    },
    /// An `if` branch region lacks the required `yield` terminator.
    #[error("{op}: region #{region_index} must terminate with quantum.dynamic.yield")]
    MissingYieldTerminator {
        /// Op name.
        op: &'static str,
        /// Region index (0 = then, 1 = else).
        region_index: usize,
    },
    /// A `yield` operand count does not match the enclosing `if` results.
    #[error("{op}: yield has {found} operand(s) but if has {expected} result(s)")]
    YieldCountMismatch {
        /// Op name.
        op: &'static str,
        /// Expected operand count.
        expected: usize,
        /// Actual operand count.
        found: usize,
    },
    /// An `if` op has the wrong number of regions.
    #[error("{op}: expected {expected} region(s), found {found}")]
    RegionCount {
        /// Op name.
        op: &'static str,
        /// Expected region count.
        expected: usize,
        /// Actual region count.
        found: usize,
    },
}

/// An error constructing a `quantum.dynamic` op.
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

/// Verifies a single `quantum.dynamic` op against its structural invariants.
///
/// Ops outside the dialect are accepted unchanged (`Ok`). Qubit reuse is
/// checked by [`crate::passes::dynamic_linearity_verifier`].
pub fn verify<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    let identifier = operation.name();
    let name_ref = identifier.as_string_ref();
    match name_ref.as_str().unwrap_or("") {
        op::MEASURE => verify_measure(operation),
        op::RESET => verify_reset(operation),
        op::UNITARY_REGION => verify_unitary_region(operation),
        op::IF => verify_if(operation),
        op::BARRIER => verify_barrier(operation),
        op::YIELD => verify_yield(operation),
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

fn verify_optional_i32<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<(), VerifyError> {
    if let Ok(value) = operation.attribute(attr) {
        IntegerAttribute::try_from(value).map_err(|_| VerifyError::WrongAttributeType {
            op,
            attr,
            expected: "i32",
        })?;
    }
    Ok(())
}

fn verify_optional_bool<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<(), VerifyError> {
    if let Ok(value) = operation.attribute(attr)
        && !value.is_bool()
    {
        return Err(VerifyError::WrongAttributeType {
            op,
            attr,
            expected: "bool",
        });
    }
    Ok(())
}

fn verify_optional_f64<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<(), VerifyError> {
    if let Ok(value) = operation.attribute(attr) {
        FloatAttribute::try_from(value).map_err(|_| VerifyError::WrongAttributeType {
            op,
            attr,
            expected: "f64",
        })?;
    }
    Ok(())
}

fn verify_optional_physical_attrs<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
) -> Result<(), VerifyError> {
    verify_optional_i32(operation, op, attr::PHYS_QUBIT)?;
    verify_optional_bool(operation, op, attr::NATIVE_GATE)?;
    verify_optional_f64(operation, op, attr::FIDELITY)?;
    Ok(())
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

fn qubit_operand_count<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> usize {
    operation
        .operands()
        .filter(|operand| quantum_circ::is_qubit_type(operand.r#type()))
        .count()
}

fn verify_measure<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    expect_counts(operation, op::MEASURE, 1, 1)?;
    expect_operand_type(
        operation,
        op::MEASURE,
        0,
        quantum_circ::is_qubit_type,
        quantum_circ::QUBIT_TYPE,
    )?;
    expect_result_type(operation, op::MEASURE, 0, is_bit_type, BIT_TYPE)?;
    verify_optional_physical_attrs(operation, op::MEASURE)
}

fn verify_reset<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    expect_counts(operation, op::RESET, 1, 1)?;
    expect_operand_type(
        operation,
        op::RESET,
        0,
        quantum_circ::is_qubit_type,
        quantum_circ::QUBIT_TYPE,
    )?;
    expect_result_type(
        operation,
        op::RESET,
        0,
        quantum_circ::is_qubit_type,
        quantum_circ::QUBIT_TYPE,
    )?;
    verify_optional_physical_attrs(operation, op::RESET)
}

fn verify_barrier<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    let operands = operation.operand_count();
    let results = operation.result_count();
    if !linearity::barrier_identity_ok(operands, results) {
        if operands == 0 {
            return Err(VerifyError::Arity {
                op: op::BARRIER,
                role: "operand",
                expected: ">= 1".to_string(),
                found: 0,
            });
        }
        return Err(VerifyError::Arity {
            op: op::BARRIER,
            role: "result",
            expected: operands.to_string(),
            found: results,
        });
    }
    for index in 0..operands {
        expect_operand_type(
            operation,
            op::BARRIER,
            index,
            quantum_circ::is_qubit_type,
            quantum_circ::QUBIT_TYPE,
        )?;
        expect_result_type(
            operation,
            op::BARRIER,
            index,
            quantum_circ::is_qubit_type,
            quantum_circ::QUBIT_TYPE,
        )?;
    }
    verify_optional_physical_attrs(operation, op::BARRIER)
}

fn is_quantum_circ_op_name(name: &str) -> bool {
    name.starts_with("quantum.circ.")
}

fn verify_unitary_region<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
) -> Result<(), VerifyError> {
    let operands = operation.operand_count();
    if operation.result_count() != operands {
        return Err(VerifyError::Arity {
            op: op::UNITARY_REGION,
            role: "result",
            expected: operands.to_string(),
            found: operation.result_count(),
        });
    }
    for index in 0..operands {
        expect_operand_type(
            operation,
            op::UNITARY_REGION,
            index,
            quantum_circ::is_qubit_type,
            quantum_circ::QUBIT_TYPE,
        )?;
        expect_result_type(
            operation,
            op::UNITARY_REGION,
            index,
            quantum_circ::is_qubit_type,
            quantum_circ::QUBIT_TYPE,
        )?;
    }

    require_string(operation, op::UNITARY_REGION, attr::DEPTH)?;
    require_bool(operation, op::UNITARY_REGION, attr::CLIFFORD)?;

    let region = operation
        .region(0)
        .map_err(|_| VerifyError::MissingRegion {
            op: op::UNITARY_REGION,
        })?;
    let block = region.first_block().ok_or(VerifyError::MissingRegion {
        op: op::UNITARY_REGION,
    })?;

    let arguments = block.argument_count();
    if arguments != operands {
        return Err(VerifyError::ArgCountMismatch {
            op: op::UNITARY_REGION,
            expected: operands,
            found: arguments,
        });
    }
    for index in 0..arguments {
        let argument = block.argument(index).map_err(|_| VerifyError::Arity {
            op: op::UNITARY_REGION,
            role: "block argument",
            expected: format!("> {index}"),
            found: arguments,
        })?;
        if !quantum_circ::is_qubit_type(argument.r#type()) {
            return Err(VerifyError::WrongValueType {
                op: op::UNITARY_REGION,
                role: "block argument",
                index,
                expected: quantum_circ::QUBIT_TYPE,
            });
        }
    }

    let mut return_operands = None;
    let mut operation_ref = block.first_operation();
    while let Some(inner) = operation_ref {
        let name = inner
            .name()
            .as_string_ref()
            .as_str()
            .unwrap_or("")
            .to_string();
        if !is_quantum_circ_op_name(&name) {
            return Err(VerifyError::ForbiddenOpInUnitaryRegion {
                op: op::UNITARY_REGION,
                found: name,
            });
        }
        if name == quantum_circ::op::FUNC {
            return Err(VerifyError::ForbiddenOpInUnitaryRegion {
                op: op::UNITARY_REGION,
                found: name,
            });
        }
        quantum_circ::verify(&inner).map_err(|error| match error {
            quantum_circ::VerifyError::MissingAttribute { op, attr } => {
                VerifyError::MissingAttribute { op, attr }
            }
            quantum_circ::VerifyError::WrongAttributeType { op, attr, expected } => {
                VerifyError::WrongAttributeType { op, attr, expected }
            }
            quantum_circ::VerifyError::Arity {
                op,
                role,
                expected,
                found,
            } => VerifyError::Arity {
                op,
                role,
                expected,
                found,
            },
            quantum_circ::VerifyError::WrongValueType {
                op,
                role,
                index,
                expected,
            } => VerifyError::WrongValueType {
                op,
                role,
                index,
                expected,
            },
            quantum_circ::VerifyError::NegativeCount { op, attr, value: _ } => {
                VerifyError::WrongAttributeType {
                    op,
                    attr,
                    expected: "non-negative integer",
                }
            }
            quantum_circ::VerifyError::MissingRegion { op } => VerifyError::MissingRegion { op },
            quantum_circ::VerifyError::ArgCountMismatch {
                op,
                expected,
                found,
            } => VerifyError::ArgCountMismatch {
                op,
                expected: expected as usize,
                found,
            },
        })?;

        if name == quantum_circ::op::RETURN {
            return_operands = Some(inner.operand_count());
        }

        for nested in inner.regions() {
            verify_circ_only_region(nested)?;
        }

        operation_ref = inner.next_in_block();
    }

    let return_count = return_operands.ok_or(VerifyError::MissingRegion {
        op: op::UNITARY_REGION,
    })?;
    if !linearity::unitary_region_boundary_ok(operands, arguments, return_count, operands) {
        return Err(VerifyError::UnitaryRegionBoundaryMismatch {
            op: op::UNITARY_REGION,
            role: "result",
            outer: operands,
            inner: return_count,
        });
    }

    Ok(())
}

fn verify_circ_only_region<'c>(region: RegionRef<'c, '_>) -> Result<(), VerifyError> {
    let mut block = region.first_block();
    while let Some(current) = block {
        let mut operation = current.first_operation();
        while let Some(inner) = operation {
            let name = inner
                .name()
                .as_string_ref()
                .as_str()
                .unwrap_or("")
                .to_string();
            if !is_quantum_circ_op_name(&name) {
                return Err(VerifyError::ForbiddenOpInUnitaryRegion {
                    op: op::UNITARY_REGION,
                    found: name,
                });
            }
            quantum_circ::verify(&inner).map_err(|error| match error {
                quantum_circ::VerifyError::MissingAttribute { op, attr } => {
                    VerifyError::MissingAttribute { op, attr }
                }
                quantum_circ::VerifyError::WrongAttributeType { op, attr, expected } => {
                    VerifyError::WrongAttributeType { op, attr, expected }
                }
                quantum_circ::VerifyError::Arity {
                    op,
                    role,
                    expected,
                    found,
                } => VerifyError::Arity {
                    op,
                    role,
                    expected,
                    found,
                },
                quantum_circ::VerifyError::WrongValueType {
                    op,
                    role,
                    index,
                    expected,
                } => VerifyError::WrongValueType {
                    op,
                    role,
                    index,
                    expected,
                },
                quantum_circ::VerifyError::NegativeCount { op, attr, value: _ } => {
                    VerifyError::WrongAttributeType {
                        op,
                        attr,
                        expected: "non-negative integer",
                    }
                }
                quantum_circ::VerifyError::MissingRegion { op } => {
                    VerifyError::MissingRegion { op }
                }
                quantum_circ::VerifyError::ArgCountMismatch {
                    op,
                    expected,
                    found,
                } => VerifyError::ArgCountMismatch {
                    op,
                    expected: expected as usize,
                    found,
                },
            })?;
            for nested in inner.regions() {
                verify_circ_only_region(nested)?;
            }
            operation = inner.next_in_block();
        }
        block = current.next_in_region();
    }
    Ok(())
}

fn verify_if<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    if operation.operand_count() == 0 {
        return Err(VerifyError::Arity {
            op: op::IF,
            role: "operand",
            expected: ">= 1".to_string(),
            found: 0,
        });
    }
    expect_operand_type(operation, op::IF, 0, is_bit_type, BIT_TYPE)?;

    let qubit_operands = qubit_operand_count(operation);
    if !linearity::if_qubit_threading_ok(qubit_operands, operation.result_count()) {
        return Err(VerifyError::Arity {
            op: op::IF,
            role: "result",
            expected: qubit_operands.to_string(),
            found: operation.result_count(),
        });
    }
    for index in 0..qubit_operands {
        let operand_index = index + 1;
        expect_operand_type(
            operation,
            op::IF,
            operand_index,
            quantum_circ::is_qubit_type,
            quantum_circ::QUBIT_TYPE,
        )?;
        expect_result_type(
            operation,
            op::IF,
            index,
            quantum_circ::is_qubit_type,
            quantum_circ::QUBIT_TYPE,
        )?;
    }

    if operation.region_count() != 2 {
        return Err(VerifyError::RegionCount {
            op: op::IF,
            expected: 2,
            found: operation.region_count(),
        });
    }

    for region_index in 0..2 {
        let region = operation
            .region(region_index)
            .map_err(|_| VerifyError::MissingRegion { op: op::IF })?;
        let block = region
            .first_block()
            .ok_or(VerifyError::MissingRegion { op: op::IF })?;
        if block.argument_count() != qubit_operands {
            return Err(VerifyError::ArgCountMismatch {
                op: op::IF,
                expected: qubit_operands,
                found: block.argument_count(),
            });
        }
        for index in 0..block.argument_count() {
            let argument = block.argument(index).map_err(|_| VerifyError::Arity {
                op: op::IF,
                role: "block argument",
                expected: format!("> {index}"),
                found: block.argument_count(),
            })?;
            if !quantum_circ::is_qubit_type(argument.r#type()) {
                return Err(VerifyError::WrongValueType {
                    op: op::IF,
                    role: "block argument",
                    index,
                    expected: quantum_circ::QUBIT_TYPE,
                });
            }
        }

        let mut terminator = None;
        let mut operation_ref = block.first_operation();
        while let Some(inner) = operation_ref {
            terminator = Some(inner);
            operation_ref = inner.next_in_block();
        }
        let terminator = terminator.ok_or(VerifyError::MissingYieldTerminator {
            op: op::IF,
            region_index,
        })?;
        let terminator_name = {
            let name = terminator.name();
            name.as_string_ref().as_str().unwrap_or("").to_string()
        };
        if terminator_name != op::YIELD {
            return Err(VerifyError::MissingYieldTerminator {
                op: op::IF,
                region_index,
            });
        }
        if terminator.operand_count() != qubit_operands {
            return Err(VerifyError::YieldCountMismatch {
                op: op::IF,
                expected: qubit_operands,
                found: terminator.operand_count(),
            });
        }
        for index in 0..terminator.operand_count() {
            expect_operand_type(
                &terminator,
                op::IF,
                index,
                quantum_circ::is_qubit_type,
                quantum_circ::QUBIT_TYPE,
            )?;
        }
    }

    Ok(())
}

fn verify_yield<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    if operation.result_count() != 0 {
        return Err(VerifyError::Arity {
            op: op::YIELD,
            role: "result",
            expected: "0".to_string(),
            found: operation.result_count(),
        });
    }
    for index in 0..operation.operand_count() {
        expect_operand_type(
            operation,
            op::YIELD,
            index,
            quantum_circ::is_qubit_type,
            quantum_circ::QUBIT_TYPE,
        )?;
    }
    Ok(())
}

// --- Builders --------------------------------------------------------------

fn depth_attribute<'c>(context: &'c Context, depth: &DepthExpr) -> (Identifier<'c>, Attribute<'c>) {
    (
        Identifier::new(context, attr::DEPTH),
        StringAttribute::new(context, &depth.to_sexpr()).into(),
    )
}

fn finish(builder: OperationBuilder) -> Result<Operation, BuildError> {
    let operation = builder.build()?;
    verify(&operation)?;
    Ok(operation)
}

/// Builds a `quantum.dynamic.measure` op.
pub fn measure<'c>(
    context: &'c Context,
    qubit: Value<'c, '_>,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::MEASURE, location)
            .add_operands(&[qubit])
            .add_results(&[bit_type(context)]),
    )
}

/// Builds a `quantum.dynamic.reset` op.
pub fn reset<'c>(
    context: &'c Context,
    qubit: Value<'c, '_>,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::RESET, location)
            .add_operands(&[qubit])
            .add_results(&[quantum_circ::qubit_type(context)]),
    )
}

/// Builds a `quantum.dynamic.barrier` op (identity threading).
pub fn barrier<'c>(
    context: &'c Context,
    qubits: &[Value<'c, '_>],
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    let results = vec![quantum_circ::qubit_type(context); qubits.len()];
    finish(
        OperationBuilder::new(op::BARRIER, location)
            .add_operands(qubits)
            .add_results(&results),
    )
}

/// Builds a `quantum.dynamic.unitary_region` op from a populated region.
pub fn unitary_region<'c>(
    context: &'c Context,
    qubits: &[Value<'c, '_>],
    depth: &DepthExpr,
    clifford: bool,
    body: Region<'c>,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    let results = vec![quantum_circ::qubit_type(context); qubits.len()];
    finish(
        OperationBuilder::new(op::UNITARY_REGION, location)
            .add_operands(qubits)
            .add_results(&results)
            .add_attributes(&[
                depth_attribute(context, depth),
                (
                    Identifier::new(context, attr::CLIFFORD),
                    BoolAttribute::new(context, clifford).into(),
                ),
            ])
            .add_regions([body]),
    )
}

/// Builds a `quantum.dynamic.if` op from populated branch regions.
pub fn r#if<'c>(
    context: &'c Context,
    condition: Value<'c, '_>,
    qubits: &[Value<'c, '_>],
    then_region: Region<'c>,
    else_region: Region<'c>,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    let mut operands = vec![condition];
    operands.extend_from_slice(qubits);
    let results = vec![quantum_circ::qubit_type(context); qubits.len()];
    finish(
        OperationBuilder::new(op::IF, location)
            .add_operands(&operands)
            .add_results(&results)
            .add_regions([then_region, else_region]),
    )
}

/// Builds the `quantum.dynamic.yield` terminator for an `if` branch.
pub fn r#yield<'c>(
    qubits: &[Value<'c, '_>],
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(OperationBuilder::new(op::YIELD, location).add_operands(qubits))
}
