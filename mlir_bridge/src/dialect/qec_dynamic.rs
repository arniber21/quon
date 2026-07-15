//! QEC ops in the `quantum.dynamic` dialect (issue #251).
//!
//! These ops carry encoded-block SSA values (`!quantum.qec_block`) with family /
//! distance / logical-id attributes. They are collected into MLIR-free
//! [`quon_qec::QecWorkload`] structures; schedule expansion is #248.

use melior::Context;
use melior::ir::attribute::{IntegerAttribute, StringAttribute};
use melior::ir::operation::{OperationBuilder, OperationLike};
use melior::ir::r#type::IntegerType;
use melior::ir::{Attribute, Identifier, Location, Operation, Type, Value, ValueLike};
use thiserror::Error;

use super::quantum_dynamic::{BIT_TYPE, BuildError, VerifyError};

/// Opaque MLIR type for a source `QecBlock` SSA value.
pub const QEC_BLOCK_TYPE: &str = "!quantum.qec_block";

/// QEC operation names under `quantum.dynamic`.
pub mod op {
    pub const CONSTRUCT: &str = "quantum.dynamic.qec_construct";
    pub const MEMORY_ROUND: &str = "quantum.dynamic.qec_memory_round";
    pub const MEASURE_LOGICAL: &str = "quantum.dynamic.qec_measure_logical";
    pub const LOGICAL_CX: &str = "quantum.dynamic.qec_logical_cx";
}

/// Attribute names shared by QEC dynamic ops.
pub mod attr {
    pub const FAMILY: &str = "family";
    pub const DISTANCE: &str = "distance";
    pub const BASIS: &str = "basis";
    pub const LOGICAL_ID: &str = "logical_id";
    pub const CONTROL_ID: &str = "control_id";
    pub const TARGET_ID: &str = "target_id";
}

/// The MLIR type of a QEC block SSA value.
pub fn qec_block_type(context: &Context) -> Type<'_> {
    Type::parse(context, QEC_BLOCK_TYPE).unwrap_or_else(|| Type::none(context))
}

/// True if `r#type` is `!quantum.qec_block`.
pub fn is_qec_block_type(r#type: Type) -> bool {
    r#type.to_string() == QEC_BLOCK_TYPE
}

fn i64_type(context: &Context) -> Type<'_> {
    IntegerType::new(context, 64).into()
}

fn bit_type(context: &Context) -> Type<'_> {
    Type::parse(context, BIT_TYPE).unwrap_or_else(|| Type::none(context))
}

fn finish(builder: OperationBuilder) -> Result<Operation, BuildError> {
    let operation = builder.build()?;
    verify(&operation)?;
    Ok(operation)
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

fn require_string_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<String, VerifyError> {
    let value = require_attr(operation, op, attr)?;
    StringAttribute::try_from(value)
        .map(|s| s.value().to_string())
        .map_err(|_| VerifyError::WrongAttributeType {
            op,
            attr,
            expected: "string",
        })
}

fn require_i64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<i64, VerifyError> {
    let value = require_attr(operation, op, attr)?;
    IntegerAttribute::try_from(value)
        .map(|i| i.value())
        .map_err(|_| VerifyError::WrongAttributeType {
            op,
            attr,
            expected: "i64",
        })
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

fn expect_operand_qec<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    index: usize,
) -> Result<(), VerifyError> {
    let operand = operation
        .operand(index)
        .map_err(|_| VerifyError::Arity {
            op,
            role: "operand",
            expected: (index + 1).to_string(),
            found: operation.operand_count(),
        })?;
    if !is_qec_block_type(operand.r#type()) {
        return Err(VerifyError::WrongValueType {
            op,
            role: "operand",
            index,
            expected: QEC_BLOCK_TYPE,
        });
    }
    Ok(())
}

fn expect_result_qec<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    index: usize,
) -> Result<(), VerifyError> {
    let result = operation.result(index).map_err(|_| VerifyError::Arity {
        op,
        role: "result",
        expected: (index + 1).to_string(),
        found: operation.result_count(),
    })?;
    if !is_qec_block_type(result.r#type()) {
        return Err(VerifyError::WrongValueType {
            op,
            role: "result",
            index,
            expected: QEC_BLOCK_TYPE,
        });
    }
    Ok(())
}

/// Verifies a QEC dynamic op. Non-QEC ops return `Ok` unchanged.
pub fn verify<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    let identifier = operation.name();
    let name = identifier.as_string_ref().as_str().unwrap_or("");
    match name {
        op::CONSTRUCT => verify_construct(operation),
        op::MEMORY_ROUND => verify_memory_round(operation),
        op::MEASURE_LOGICAL => verify_measure_logical(operation),
        op::LOGICAL_CX => verify_logical_cx(operation),
        _ => Ok(()),
    }
}

fn verify_construct<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
) -> Result<(), VerifyError> {
    expect_counts(operation, op::CONSTRUCT, 0, 1)?;
    expect_result_qec(operation, op::CONSTRUCT, 0)?;
    let family = require_string_attr(operation, op::CONSTRUCT, attr::FAMILY)?;
    if quon_qec::SourceFamily::parse(&family).is_none() {
        return Err(VerifyError::WrongAttributeType {
            op: op::CONSTRUCT,
            attr: attr::FAMILY,
            expected: "\"repetition\" or \"surface\"",
        });
    }
    let basis = require_string_attr(operation, op::CONSTRUCT, attr::BASIS)?;
    if quon_qec::LogicalBasis::parse(&basis).is_none() {
        return Err(VerifyError::WrongAttributeType {
            op: op::CONSTRUCT,
            attr: attr::BASIS,
            expected: "\"x\" or \"z\"",
        });
    }
    let _ = require_i64_attr(operation, op::CONSTRUCT, attr::DISTANCE)?;
    let _ = require_i64_attr(operation, op::CONSTRUCT, attr::LOGICAL_ID)?;
    Ok(())
}

fn verify_memory_round<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
) -> Result<(), VerifyError> {
    expect_counts(operation, op::MEMORY_ROUND, 1, 1)?;
    expect_operand_qec(operation, op::MEMORY_ROUND, 0)?;
    expect_result_qec(operation, op::MEMORY_ROUND, 0)?;
    let _ = require_i64_attr(operation, op::MEMORY_ROUND, attr::LOGICAL_ID)?;
    Ok(())
}

fn verify_measure_logical<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
) -> Result<(), VerifyError> {
    expect_counts(operation, op::MEASURE_LOGICAL, 1, 1)?;
    expect_operand_qec(operation, op::MEASURE_LOGICAL, 0)?;
    let result = operation.result(0).map_err(|_| VerifyError::Arity {
        op: op::MEASURE_LOGICAL,
        role: "result",
        expected: "1".into(),
        found: operation.result_count(),
    })?;
    if result.r#type().to_string() != BIT_TYPE {
        return Err(VerifyError::WrongValueType {
            op: op::MEASURE_LOGICAL,
            role: "result",
            index: 0,
            expected: BIT_TYPE,
        });
    }
    let basis = require_string_attr(operation, op::MEASURE_LOGICAL, attr::BASIS)?;
    if quon_qec::LogicalBasis::parse(&basis).is_none() {
        return Err(VerifyError::WrongAttributeType {
            op: op::MEASURE_LOGICAL,
            attr: attr::BASIS,
            expected: "\"x\" or \"z\"",
        });
    }
    let _ = require_i64_attr(operation, op::MEASURE_LOGICAL, attr::LOGICAL_ID)?;
    Ok(())
}

fn verify_logical_cx<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
) -> Result<(), VerifyError> {
    expect_counts(operation, op::LOGICAL_CX, 2, 2)?;
    expect_operand_qec(operation, op::LOGICAL_CX, 0)?;
    expect_operand_qec(operation, op::LOGICAL_CX, 1)?;
    expect_result_qec(operation, op::LOGICAL_CX, 0)?;
    expect_result_qec(operation, op::LOGICAL_CX, 1)?;
    let _ = require_i64_attr(operation, op::LOGICAL_CX, attr::CONTROL_ID)?;
    let _ = require_i64_attr(operation, op::LOGICAL_CX, attr::TARGET_ID)?;
    Ok(())
}

/// Builds `quantum.dynamic.qec_construct`.
pub fn qec_construct<'c>(
    context: &'c Context,
    family: &str,
    distance: i64,
    basis: &str,
    logical_id: i64,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::CONSTRUCT, location)
            .add_results(&[qec_block_type(context)])
            .add_attributes(&[
                (
                    Identifier::new(context, attr::FAMILY),
                    StringAttribute::new(context, family).into(),
                ),
                (
                    Identifier::new(context, attr::DISTANCE),
                    IntegerAttribute::new(i64_type(context), distance).into(),
                ),
                (
                    Identifier::new(context, attr::BASIS),
                    StringAttribute::new(context, basis).into(),
                ),
                (
                    Identifier::new(context, attr::LOGICAL_ID),
                    IntegerAttribute::new(i64_type(context), logical_id).into(),
                ),
            ]),
    )
}

/// Builds `quantum.dynamic.qec_memory_round`.
pub fn qec_memory_round<'c>(
    context: &'c Context,
    block: Value<'c, '_>,
    logical_id: i64,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::MEMORY_ROUND, location)
            .add_operands(&[block])
            .add_results(&[qec_block_type(context)])
            .add_attributes(&[(
                Identifier::new(context, attr::LOGICAL_ID),
                IntegerAttribute::new(i64_type(context), logical_id).into(),
            )]),
    )
}

/// Builds `quantum.dynamic.qec_measure_logical`.
pub fn qec_measure_logical<'c>(
    context: &'c Context,
    block: Value<'c, '_>,
    basis: &str,
    logical_id: i64,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::MEASURE_LOGICAL, location)
            .add_operands(&[block])
            .add_results(&[bit_type(context)])
            .add_attributes(&[
                (
                    Identifier::new(context, attr::BASIS),
                    StringAttribute::new(context, basis).into(),
                ),
                (
                    Identifier::new(context, attr::LOGICAL_ID),
                    IntegerAttribute::new(i64_type(context), logical_id).into(),
                ),
            ]),
    )
}

/// Builds `quantum.dynamic.qec_logical_cx` (stub until #248 expands it).
pub fn qec_logical_cx<'c>(
    context: &'c Context,
    control: Value<'c, '_>,
    target: Value<'c, '_>,
    control_id: i64,
    target_id: i64,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::LOGICAL_CX, location)
            .add_operands(&[control, target])
            .add_results(&[qec_block_type(context), qec_block_type(context)])
            .add_attributes(&[
                (
                    Identifier::new(context, attr::CONTROL_ID),
                    IntegerAttribute::new(i64_type(context), control_id).into(),
                ),
                (
                    Identifier::new(context, attr::TARGET_ID),
                    IntegerAttribute::new(i64_type(context), target_id).into(),
                ),
            ]),
    )
}

/// True when `name` is a QEC dynamic op.
pub fn is_qec_op(name: &str) -> bool {
    matches!(
        name,
        op::CONSTRUCT | op::MEMORY_ROUND | op::MEASURE_LOGICAL | op::LOGICAL_CX
    )
}

/// Errors reading QEC attributes during collection (not structural verify).
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum QecAttrError {
    #[error("missing attribute `{0}`")]
    Missing(&'static str),
    #[error("invalid attribute `{attr}`: {detail}")]
    Invalid { attr: &'static str, detail: String },
}

pub fn read_string_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    attr: &'static str,
) -> Result<String, QecAttrError> {
    let value = operation
        .attribute(attr)
        .map_err(|_| QecAttrError::Missing(attr))?;
    StringAttribute::try_from(value)
        .map(|s| s.value().to_string())
        .map_err(|_| QecAttrError::Invalid {
            attr,
            detail: "expected string".into(),
        })
}

pub fn read_i64_attr<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    attr: &'static str,
) -> Result<i64, QecAttrError> {
    let value = operation
        .attribute(attr)
        .map_err(|_| QecAttrError::Missing(attr))?;
    IntegerAttribute::try_from(value)
        .map(|i| i.value())
        .map_err(|_| QecAttrError::Invalid {
            attr,
            detail: "expected i64".into(),
        })
}
