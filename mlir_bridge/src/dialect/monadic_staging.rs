//! Staging ops for monadic lowering (issue #17).
//!
//! These ops live in `quantum.circ.run` regions and represent desugared `run { }`
//! bind chains before conversion to `quantum.dynamic`. They are erased by
//! [`crate::passes::monadic_lowering`] and never appear in final IR.

use melior::Context;
use melior::ir::attribute::{IntegerAttribute, StringAttribute};
use melior::ir::operation::OperationBuilder;
use melior::ir::r#type::IntegerType;
use melior::ir::{Block, Identifier, Location, Operation, Region, Type, Value};

use super::qec_dynamic::{self, QEC_BLOCK_TYPE};
use super::quantum_circ::{self, QUBIT_TYPE};
use super::quantum_dynamic::BIT_TYPE;

/// Staging operation names (lowered away by the monadic lowering pass).
pub mod op {
    /// Container for a dynamic computation body.
    pub const RUN: &str = "quantum.circ.run";
    /// Allocate `count` fresh qubits in |0⟩.
    pub const QREG: &str = "quantum.circ.qreg";
    /// Apply a `quantum.circ.func` to qubit operands.
    pub const APPLY: &str = "quantum.circ.apply";
    /// Mid-circuit measurement (staging; becomes `quantum.dynamic.measure`).
    pub const MEASURE: &str = "quantum.circ.measure";
    /// Measure-and-reprepare (staging; becomes `quantum.dynamic.reset`).
    pub const RESET: &str = "quantum.circ.reset";
    /// Measure and discard the classical result.
    pub const DISCARD: &str = "quantum.circ.discard";
    /// Classically controlled circuit application.
    pub const COND_APPLY: &str = "quantum.circ.cond_apply";
    /// Terminator for a `run` region.
    pub const YIELD: &str = "quantum.circ.yield";
    /// QEC construct (`repetition_code` / `surface_code` / `surface_code_x`).
    pub const QEC_CONSTRUCT: &str = "quantum.circ.qec_construct";
    /// QEC `memory_round`.
    pub const QEC_MEMORY_ROUND: &str = "quantum.circ.qec_memory_round";
    /// QEC logical measurement.
    pub const QEC_MEASURE_LOGICAL: &str = "quantum.circ.qec_measure_logical";
    /// QEC `logical_cx` (surface-only stub).
    pub const QEC_LOGICAL_CX: &str = "quantum.circ.qec_logical_cx";
}

pub mod attr {
    pub const COUNT: &str = "count";
    pub const CALLEE: &str = "callee";
    pub const THEN_CALLEE: &str = "then_callee";
    pub const ELSE_CALLEE: &str = "else_callee";
    pub const FAMILY: &str = "family";
    pub const DISTANCE: &str = "distance";
    pub const BASIS: &str = "basis";
    pub const LOGICAL_ID: &str = "logical_id";
    pub const CONTROL_ID: &str = "control_id";
    pub const TARGET_ID: &str = "target_id";
}

fn i64_type(context: &Context) -> Type<'_> {
    IntegerType::new(context, 64).into()
}

fn bit_type(context: &Context) -> Type<'_> {
    Type::parse(context, BIT_TYPE).unwrap_or_else(|| Type::none(context))
}

/// Builds a `quantum.circ.run` op with a populated region.
pub fn run<'c>(
    _context: &'c Context,
    operands: &[Value<'c, '_>],
    body: Region<'c>,
    location: Location<'c>,
) -> Operation<'c> {
    OperationBuilder::new(op::RUN, location)
        .add_operands(operands)
        .add_regions([body])
        .build()
        .expect("run op builds")
}

/// Builds a `quantum.circ.qreg` op producing `count` qubits.
pub fn qreg<'c>(context: &'c Context, count: i64, location: Location<'c>) -> Operation<'c> {
    let qubit = quantum_circ::qubit_type(context);
    let results = vec![qubit; count as usize];
    OperationBuilder::new(op::QREG, location)
        .add_results(&results)
        .add_attributes(&[(
            Identifier::new(context, attr::COUNT),
            IntegerAttribute::new(i64_type(context), count).into(),
        )])
        .build()
        .expect("qreg op builds")
}

/// Builds a `quantum.circ.apply` op.
pub fn apply<'c>(
    context: &'c Context,
    callee: &str,
    qubits: &[Value<'c, '_>],
    location: Location<'c>,
) -> Operation<'c> {
    let results = vec![quantum_circ::qubit_type(context); qubits.len()];
    OperationBuilder::new(op::APPLY, location)
        .add_operands(qubits)
        .add_results(&results)
        .add_attributes(&[(
            Identifier::new(context, attr::CALLEE),
            StringAttribute::new(context, callee).into(),
        )])
        .build()
        .expect("apply op builds")
}

/// Builds a staging `quantum.circ.measure` op.
pub fn measure<'c>(
    context: &'c Context,
    qubit: Value<'c, '_>,
    location: Location<'c>,
) -> Operation<'c> {
    OperationBuilder::new(op::MEASURE, location)
        .add_operands(&[qubit])
        .add_results(&[bit_type(context)])
        .build()
        .expect("staging measure op builds")
}

/// Builds a staging `quantum.circ.reset` op.
pub fn reset<'c>(
    context: &'c Context,
    qubit: Value<'c, '_>,
    location: Location<'c>,
) -> Operation<'c> {
    OperationBuilder::new(op::RESET, location)
        .add_operands(&[qubit])
        .add_results(&[quantum_circ::qubit_type(context)])
        .build()
        .expect("staging reset op builds")
}

/// Builds a staging `quantum.circ.discard` op.
pub fn discard<'c>(qubit: Value<'c, '_>, location: Location<'c>) -> Operation<'c> {
    OperationBuilder::new(op::DISCARD, location)
        .add_operands(&[qubit])
        .build()
        .expect("staging discard op builds")
}

/// Builds a `quantum.circ.cond_apply` op.
pub fn cond_apply<'c>(
    context: &'c Context,
    condition: Value<'c, '_>,
    then_callee: &str,
    else_callee: &str,
    qubits: &[Value<'c, '_>],
    location: Location<'c>,
) -> Operation<'c> {
    let mut operands = vec![condition];
    operands.extend_from_slice(qubits);
    let results = vec![quantum_circ::qubit_type(context); qubits.len()];
    OperationBuilder::new(op::COND_APPLY, location)
        .add_operands(&operands)
        .add_results(&results)
        .add_attributes(&[
            (
                Identifier::new(context, attr::THEN_CALLEE),
                StringAttribute::new(context, then_callee).into(),
            ),
            (
                Identifier::new(context, attr::ELSE_CALLEE),
                StringAttribute::new(context, else_callee).into(),
            ),
        ])
        .build()
        .expect("cond_apply op builds")
}

/// Builds the `quantum.circ.yield` terminator for a `run` region.
pub fn r#yield<'c>(values: &[Value<'c, '_>], location: Location<'c>) -> Operation<'c> {
    OperationBuilder::new(op::YIELD, location)
        .add_operands(values)
        .build()
        .expect("yield op builds")
}

/// Builds an entry block for a `run` region with the given argument types.
pub fn run_entry_block<'c>(
    arg_types: &[(Type<'c>, Location<'c>)],
    _location: Location<'c>,
) -> Block<'c> {
    Block::new(arg_types)
}

/// Builds a staging `quantum.circ.qec_construct` op.
pub fn qec_construct<'c>(
    context: &'c Context,
    family: &str,
    distance: i64,
    basis: &str,
    logical_id: i64,
    location: Location<'c>,
) -> Operation<'c> {
    OperationBuilder::new(op::QEC_CONSTRUCT, location)
        .add_results(&[qec_dynamic::qec_block_type(context)])
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
        ])
        .build()
        .expect("staging qec_construct builds")
}

/// Builds a staging `quantum.circ.qec_memory_round` op.
pub fn qec_memory_round<'c>(
    context: &'c Context,
    block: Value<'c, '_>,
    logical_id: i64,
    location: Location<'c>,
) -> Operation<'c> {
    OperationBuilder::new(op::QEC_MEMORY_ROUND, location)
        .add_operands(&[block])
        .add_results(&[qec_dynamic::qec_block_type(context)])
        .add_attributes(&[(
            Identifier::new(context, attr::LOGICAL_ID),
            IntegerAttribute::new(i64_type(context), logical_id).into(),
        )])
        .build()
        .expect("staging qec_memory_round builds")
}

/// Builds a staging `quantum.circ.qec_measure_logical` op.
pub fn qec_measure_logical<'c>(
    context: &'c Context,
    block: Value<'c, '_>,
    basis: &str,
    logical_id: i64,
    location: Location<'c>,
) -> Operation<'c> {
    OperationBuilder::new(op::QEC_MEASURE_LOGICAL, location)
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
        ])
        .build()
        .expect("staging qec_measure_logical builds")
}

/// Builds a staging `quantum.circ.qec_logical_cx` op.
pub fn qec_logical_cx<'c>(
    context: &'c Context,
    control: Value<'c, '_>,
    target: Value<'c, '_>,
    control_id: i64,
    target_id: i64,
    location: Location<'c>,
) -> Operation<'c> {
    OperationBuilder::new(op::QEC_LOGICAL_CX, location)
        .add_operands(&[control, target])
        .add_results(&[
            qec_dynamic::qec_block_type(context),
            qec_dynamic::qec_block_type(context),
        ])
        .add_attributes(&[
            (
                Identifier::new(context, attr::CONTROL_ID),
                IntegerAttribute::new(i64_type(context), control_id).into(),
            ),
            (
                Identifier::new(context, attr::TARGET_ID),
                IntegerAttribute::new(i64_type(context), target_id).into(),
            ),
        ])
        .build()
        .expect("staging qec_logical_cx builds")
}

/// True when `name` is a staging op consumed by monadic lowering.
pub fn is_staging_op(name: &str) -> bool {
    matches!(
        name,
        op::RUN
            | op::QREG
            | op::APPLY
            | op::MEASURE
            | op::RESET
            | op::DISCARD
            | op::COND_APPLY
            | op::YIELD
            | op::QEC_CONSTRUCT
            | op::QEC_MEMORY_ROUND
            | op::QEC_MEASURE_LOGICAL
            | op::QEC_LOGICAL_CX
    )
}

/// True when `name` is a staging QEC op.
pub fn is_qec_staging_op(name: &str) -> bool {
    matches!(
        name,
        op::QEC_CONSTRUCT | op::QEC_MEMORY_ROUND | op::QEC_MEASURE_LOGICAL | op::QEC_LOGICAL_CX
    )
}

/// Printed type name for diagnostics.
pub fn qec_block_type_name() -> &'static str {
    QEC_BLOCK_TYPE
}

/// Returns the printed type name for a qubit value (for diagnostics).
pub fn qubit_type_name() -> &'static str {
    QUBIT_TYPE
}
