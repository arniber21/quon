//! `quantum.na` dialect — neutral-atom schedule IR in generic MLIR form.
//!
//! This follows the workspace's unregistered-dialect pattern (ADR-0004 and
//! ADR-0007): ops round-trip as generic MLIR operations, and Rust functions are
//! the verifier callbacks. The dialect is intentionally separate from generic
//! `quantum.physical` annotations because grouped AOD motion and layered
//! Rydberg stages are schedule objects, not scalar op metadata.

use std::collections::{BTreeMap, BTreeSet};

use melior::Context;
use melior::ir::attribute::{AttributeLike, FloatAttribute, IntegerAttribute, StringAttribute};
use melior::ir::operation::{OperationBuilder, OperationLike};
use melior::ir::r#type::IntegerType;
use melior::ir::{
    Attribute, Block, BlockLike, Identifier, Location, Module, Operation, Region, RegionLike,
    RegionRef, Type, Value, ValueLike,
};
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// The dialect namespace.
pub const NAMESPACE: &str = "quantum.na";

/// Opaque MLIR type for a neutral-atom identity token.
pub const ATOM_TYPE: &str = "!quantum.na.atom";

/// Opaque MLIR type for an addressable neutral-atom site token.
pub const SITE_TYPE: &str = "!quantum.na.site";

/// Opaque MLIR type for a classical measurement result.
pub const BIT_TYPE: &str = "!quantum.bit";

/// Operation names.
pub mod op {
    pub const ALLOC_ATOM: &str = "quantum.na.alloc_atom";
    pub const PLACE: &str = "quantum.na.place";
    pub const MOVE: &str = "quantum.na.move";
    pub const TRANSFER: &str = "quantum.na.transfer";
    pub const ENTANGLE: &str = "quantum.na.entangle";
    pub const MEASURE: &str = "quantum.na.measure";
    pub const RESET: &str = "quantum.na.reset";
    pub const WAIT: &str = "quantum.na.wait";
    pub const LAYER: &str = "quantum.na.layer";
    pub const SCHEDULE: &str = "quantum.na.schedule";
}

/// The primary `quantum.na` ops. `transfer` is included because trap transfers
/// are first-class schedule actions in `quon_na`.
pub const OPS: [&str; 10] = [
    op::ALLOC_ATOM,
    op::PLACE,
    op::MOVE,
    op::TRANSFER,
    op::ENTANGLE,
    op::MEASURE,
    op::RESET,
    op::WAIT,
    op::LAYER,
    op::SCHEDULE,
];

/// Attribute names.
pub mod attr {
    pub const ATOM: &str = "atom";
    pub const SITE: &str = "site";
    pub const X_UM: &str = "x_um";
    pub const Y_UM: &str = "y_um";
    pub const TRAP: &str = "trap";
    pub const AOD_ID: &str = "aod_id";
    pub const ROW: &str = "row";
    pub const COL: &str = "col";
    pub const CYCLE: &str = "cycle";
    pub const DURATION_US: &str = "duration_us";
    pub const BASIS: &str = "basis";
    pub const DIRECTION: &str = "direction";
    pub const MOVES: &str = "moves";
    pub const PAIRS: &str = "pairs";
    pub const RYDBERG_RANGE_UM: &str = "rydberg_range_um";
    pub const MIN_RYDBERG_SPACING_UM: &str = "min_rydberg_spacing_um";
    pub const AOD_MIN_SEPARATION_UM: &str = "aod_min_separation_um";
    pub const TARGET_ID: &str = "target_id";
}

/// Registers `quantum.na` with `context`.
///
/// The dialect is intentionally unregistered from MLIR's perspective; enabling
/// unregistered dialects is the complete registration step.
pub fn register_dialect(context: &Context) {
    context.set_allow_unregistered_dialects(true);
}

pub fn atom_type(context: &Context) -> Type<'_> {
    Type::parse(context, ATOM_TYPE).unwrap_or_else(|| Type::none(context))
}

pub fn site_type(context: &Context) -> Type<'_> {
    Type::parse(context, SITE_TYPE).unwrap_or_else(|| Type::none(context))
}

pub fn bit_type(context: &Context) -> Type<'_> {
    Type::parse(context, BIT_TYPE).unwrap_or_else(|| Type::none(context))
}

pub fn is_atom_type(r#type: Type) -> bool {
    r#type.to_string() == ATOM_TYPE
}

pub fn is_site_type(r#type: Type) -> bool {
    r#type.to_string() == SITE_TYPE
}

pub fn is_bit_type(r#type: Type) -> bool {
    r#type.to_string() == BIT_TYPE
}

/// One atom endpoint with explicit stage position.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PositionedAtom {
    pub atom: u32,
    pub x_um: f64,
    pub y_um: f64,
}

/// One desired Rydberg pair in a layer.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntanglePairSpec {
    pub lhs: PositionedAtom,
    pub rhs: PositionedAtom,
}

/// One atom displacement inside a grouped AOD rearrangement step.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MoveSpec {
    pub atom: u32,
    pub from_site: u32,
    pub to_site: u32,
    pub aod_id: u32,
    pub row: u32,
    pub col: u32,
    pub from_x_um: f64,
    pub from_y_um: f64,
    pub to_x_um: f64,
    pub to_y_um: f64,
}

/// Trap-transfer payload. Transfers are visible to occupancy and accounting.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransferSpec {
    pub atom: u32,
    pub site: u32,
    pub aod_id: u32,
    pub row: u32,
    pub col: u32,
    pub direction: TransferDirection,
    pub duration_us: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransferDirection {
    SlmToAod,
    AodToSlm,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleSpec {
    pub target_id: String,
    pub rydberg_range_um: f64,
    pub min_rydberg_spacing_um: f64,
    pub aod_min_separation_um: f64,
    pub layers: Vec<LayerSpec>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerSpec {
    pub cycle: u32,
    pub actions: Vec<ActionSpec>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum ActionSpec {
    Move {
        moves: Vec<MoveSpec>,
        duration_us: u64,
    },
    Transfer(TransferSpec),
    Entangle {
        pairs: Vec<EntanglePairSpec>,
        duration_us: u64,
    },
    Measure {
        atom: u32,
        basis: String,
        duration_us: u64,
    },
    Reset {
        atom: u32,
        duration_us: u64,
    },
    Wait {
        duration_us: u64,
    },
}

/// A structural or schedule-legality problem with a `quantum.na` op.
#[derive(Clone, Debug, Error, PartialEq)]
pub enum VerifyError {
    #[error("{op}: missing required attribute `{attr}`")]
    MissingAttribute {
        op: &'static str,
        attr: &'static str,
    },
    #[error("{op}: attribute `{attr}` must be of type {expected}")]
    WrongAttributeType {
        op: &'static str,
        attr: &'static str,
        expected: &'static str,
    },
    #[error("{op}: expected {expected} {role}(s), found {found}")]
    Arity {
        op: &'static str,
        role: &'static str,
        expected: String,
        found: usize,
    },
    #[error("{op}: {role} #{index} must have type {expected}")]
    WrongValueType {
        op: &'static str,
        role: &'static str,
        index: usize,
        expected: &'static str,
    },
    #[error("{op}: `{attr}` ({value}) must be non-negative")]
    NegativeCount {
        op: &'static str,
        attr: &'static str,
        value: i64,
    },
    #[error("{op}: `{attr}` must be positive and finite, got {value}")]
    NonPositiveFloat {
        op: &'static str,
        attr: &'static str,
        value: f64,
    },
    #[error("{op}: expected a region with an entry block")]
    MissingRegion { op: &'static str },
    #[error("{op}: expected {expected} region(s), found {found}")]
    RegionCount {
        op: &'static str,
        expected: usize,
        found: usize,
    },
    #[error("{op}: region contains `{found}`; only quantum.na ops allowed")]
    ForbiddenOp { op: &'static str, found: String },
    #[error("{op}: `{attr}` did not parse as {expected}: {message}")]
    JsonAttribute {
        op: &'static str,
        attr: &'static str,
        expected: &'static str,
        message: String,
    },
    #[error("cycle {cycle}: atom {atom} has multiple occupancy claims")]
    DuplicateOccupancyAtom { cycle: u32, atom: u32 },
    #[error("cycle {cycle}: site {site} has multiple occupancy claims")]
    DuplicateOccupancySite { cycle: u32, site: u32 },
    #[error("cycle {cycle}: atom {atom} participates in multiple entangling gates")]
    DuplicateEntanglingAtom { cycle: u32, atom: u32 },
    #[error(
        "cycle {cycle}: entangling pair ({lhs}, {rhs}) is distance {distance_um} um, beyond Rydberg range {rydberg_range_um} um"
    )]
    EntanglingPairOutOfRange {
        cycle: u32,
        lhs: u32,
        rhs: u32,
        distance_um: f64,
        rydberg_range_um: f64,
    },
    #[error(
        "cycle {cycle}: non-partner atoms ({lhs}, {rhs}) are distance {distance_um} um, within compulsory Rydberg range {rydberg_range_um} um"
    )]
    CompulsoryEntanglement {
        cycle: u32,
        lhs: u32,
        rhs: u32,
        distance_um: f64,
        rydberg_range_um: f64,
    },
    #[error(
        "cycle {cycle}: non-partner atoms ({lhs}, {rhs}) are distance {distance_um} um, below isolation spacing {min_spacing_um} um"
    )]
    RydbergSpacing {
        cycle: u32,
        lhs: u32,
        rhs: u32,
        distance_um: f64,
        min_spacing_um: f64,
    },
    #[error(
        "cycle {cycle}: atom {atom} uses AOD trap ({aod_id}, {row}, {col}) inconsistent with its slm_to_aod load into ({bound_aod_id}, {bound_row}, {bound_col})"
    )]
    AodRefMismatch {
        cycle: u32,
        atom: u32,
        aod_id: u32,
        row: u32,
        col: u32,
        bound_aod_id: u32,
        bound_row: u32,
        bound_col: u32,
    },
    #[error(
        "cycle {cycle}: AOD trap ({aod_id}, {row}, {col}) is claimed by moves from different source positions"
    )]
    AodTrapDoubleClaim {
        cycle: u32,
        aod_id: u32,
        row: u32,
        col: u32,
    },
    #[error("cycle {cycle}: AOD {aod_id} row {row} has inconsistent y displacement")]
    AodRowCoupling { cycle: u32, aod_id: u32, row: u32 },
    #[error("cycle {cycle}: AOD {aod_id} column {col} has inconsistent x displacement")]
    AodColumnCoupling { cycle: u32, aod_id: u32, col: u32 },
    #[error("cycle {cycle}: AOD {aod_id} rows {first} and {second} do not preserve order")]
    AodRowOrder {
        cycle: u32,
        aod_id: u32,
        first: u32,
        second: u32,
    },
    #[error("cycle {cycle}: AOD {aod_id} columns {first} and {second} do not preserve order")]
    AodColumnOrder {
        cycle: u32,
        aod_id: u32,
        first: u32,
        second: u32,
    },
    #[error(
        "cycle {cycle}: AOD {aod_id} rows {first} and {second} are closer than {min_separation_um} um"
    )]
    AodRowSeparation {
        cycle: u32,
        aod_id: u32,
        first: u32,
        second: u32,
        min_separation_um: f64,
    },
    #[error(
        "cycle {cycle}: AOD {aod_id} columns {first} and {second} are closer than {min_separation_um} um"
    )]
    AodColumnSeparation {
        cycle: u32,
        aod_id: u32,
        first: u32,
        second: u32,
        min_separation_um: f64,
    },
    #[error(
        "cycle {cycle}: schedule cycles must be non-decreasing in layer order (previous cycle was {previous_cycle})"
    )]
    NonMonotonicCycles { previous_cycle: u32, cycle: u32 },
    #[error(
        "cycle {after_cycle}: layer after round-barrier Wait at cycle {wait_cycle} must have a strictly later cycle (QEC round dependency)"
    )]
    RoundBarrierCycleOrder { wait_cycle: u32, after_cycle: u32 },
    #[error(
        "cycle {cycle}: atom {atom} is measured and entangled or moved in the same cycle (measurement ordering)"
    )]
    MeasureUseSameCycle { cycle: u32, atom: u32 },
    #[error(
        "cycle {reuse_cycle}: atom {atom} is reused at cycle {reuse_cycle} after measure at cycle {measure_cycle} without an intervening reset (measurement ordering)"
    )]
    MeasureReuseWithoutReset {
        atom: u32,
        measure_cycle: u32,
        reuse_cycle: u32,
    },
    #[error(
        "cycle {cycle}: atom {atom} is reset and entangled or moved in the same cycle (reset ordering)"
    )]
    ResetUseSameCycle { cycle: u32, atom: u32 },
    #[error(
        "cycle {reset_cycle}: atom {atom} is reset before its measure at cycle {measure_cycle} (reset ordering)"
    )]
    ResetBeforeMeasure {
        atom: u32,
        reset_cycle: u32,
        measure_cycle: u32,
    },
    #[error(
        "module: expected a top-level quantum.na.schedule op, found none"
    )]
    MissingSchedule,
    #[error("failed to parse quantum.na MLIR module")]
    ParseFailed,
}

#[derive(Debug, Error)]
pub enum BuildError {
    #[error("failed to build operation: {0}")]
    Mlir(#[from] melior::Error),
    #[error("operation failed verification: {0}")]
    Verify(#[from] VerifyError),
    #[error("operation builder produced no result #{index} for {op}")]
    MissingResult { op: &'static str, index: usize },
}

pub fn verify<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    let identifier = operation.name();
    let name_ref = identifier.as_string_ref();
    match name_ref.as_str().unwrap_or("") {
        op::ALLOC_ATOM => verify_alloc_atom(operation),
        op::PLACE => verify_place(operation),
        op::MOVE => verify_move(operation),
        op::TRANSFER => verify_transfer(operation),
        op::ENTANGLE => verify_entangle(operation),
        op::MEASURE => verify_measure(operation),
        op::RESET => verify_reset(operation),
        op::WAIT => verify_wait(operation),
        op::LAYER => verify_layer(operation, None, None).map(|_| ()),
        op::SCHEDULE => verify_schedule(operation),
        _ => Ok(()),
    }
}

/// Verify every top-level `quantum.na.schedule` in a parsed MLIR module.
pub fn verify_module(module: &Module<'_>) -> Result<(), VerifyError> {
    let Some(body) = module.as_operation().region(0).ok().and_then(|r| r.first_block()) else {
        return Err(VerifyError::MissingSchedule);
    };
    let mut op = body.first_operation();
    let mut found = false;
    while let Some(current) = op {
        let name = current
            .name()
            .as_string_ref()
            .as_str()
            .unwrap_or("")
            .to_owned();
        if name == op::SCHEDULE {
            found = true;
            verify_schedule(&current)?;
        }
        op = current.next_in_block();
    }
    if found {
        Ok(())
    } else {
        Err(VerifyError::MissingSchedule)
    }
}

/// Parse generic-form `quantum.na` MLIR text and run [`verify_module`].
pub fn verify_mlir_text(text: &str) -> Result<(), VerifyError> {
    let context = Context::new();
    register_dialect(&context);
    let module = Module::parse(&context, text).ok_or(VerifyError::ParseFailed)?;
    verify_module(&module)
}

/// Build a schedule module from [`ScheduleSpec`] and verify it.
pub fn verify_schedule_spec(spec: &ScheduleSpec) -> Result<(), VerifyError> {
    let context = Context::new();
    let module = match schedule_module(&context, spec) {
        Ok(module) => module,
        Err(BuildError::Verify(error)) => return Err(error),
        Err(_) => return Err(VerifyError::ParseFailed),
    };
    verify_module(&module)
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

fn require_i64<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
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

fn require_non_negative_i64<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<i64, VerifyError> {
    let value = require_i64(operation, op, attr)?;
    if value < 0 {
        Err(VerifyError::NegativeCount { op, attr, value })
    } else {
        Ok(value)
    }
}

fn require_f64<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<f64, VerifyError> {
    let value = require_attr(operation, op, attr)?;
    let wrong = || VerifyError::WrongAttributeType {
        op,
        attr,
        expected: "f64",
    };
    let float = FloatAttribute::try_from(value).map_err(|_| wrong())?;
    if float.r#type().to_string() != "f64" {
        return Err(wrong());
    }
    Ok(float.value())
}

fn require_positive_f64<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<f64, VerifyError> {
    let value = require_f64(operation, op, attr)?;
    if value.is_finite() && value > 0.0 {
        Ok(value)
    } else {
        Err(VerifyError::NonPositiveFloat { op, attr, value })
    }
}

fn require_string<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
) -> Result<String, VerifyError> {
    let value = require_attr(operation, op, attr)?;
    StringAttribute::try_from(value)
        .map(|string| string.value().to_owned())
        .map_err(|_| VerifyError::WrongAttributeType {
            op,
            attr,
            expected: "string",
        })
}

fn parse_json_attr<'c: 'a, 'a, O, T>(
    operation: &O,
    op: &'static str,
    attr: &'static str,
    expected: &'static str,
) -> Result<T, VerifyError>
where
    O: OperationLike<'c, 'a>,
    T: for<'de> Deserialize<'de>,
{
    let text = require_string(operation, op, attr)?;
    serde_json::from_str(&text).map_err(|error| VerifyError::JsonAttribute {
        op,
        attr,
        expected,
        message: error.to_string(),
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

fn verify_alloc_atom<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
) -> Result<(), VerifyError> {
    expect_counts(operation, op::ALLOC_ATOM, 0, 1)?;
    require_non_negative_i64(operation, op::ALLOC_ATOM, attr::ATOM)?;
    expect_result_type(operation, op::ALLOC_ATOM, 0, is_atom_type, ATOM_TYPE)
}

fn verify_place<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    expect_counts(operation, op::PLACE, 1, 1)?;
    expect_operand_type(operation, op::PLACE, 0, is_atom_type, ATOM_TYPE)?;
    expect_result_type(operation, op::PLACE, 0, is_atom_type, ATOM_TYPE)?;
    require_non_negative_i64(operation, op::PLACE, attr::ATOM)?;
    require_non_negative_i64(operation, op::PLACE, attr::SITE)?;
    require_f64(operation, op::PLACE, attr::X_UM)?;
    require_f64(operation, op::PLACE, attr::Y_UM)?;
    let trap = require_string(operation, op::PLACE, attr::TRAP)?;
    match trap.as_str() {
        "slm" => Ok(()),
        "aod" => {
            require_non_negative_i64(operation, op::PLACE, attr::AOD_ID)?;
            require_non_negative_i64(operation, op::PLACE, attr::ROW)?;
            require_non_negative_i64(operation, op::PLACE, attr::COL)?;
            Ok(())
        }
        _ => Err(VerifyError::WrongAttributeType {
            op: op::PLACE,
            attr: attr::TRAP,
            expected: "\"slm\" or \"aod\"",
        }),
    }
}

fn verify_move<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    expect_counts(operation, op::MOVE, 0, 0)?;
    require_non_negative_i64(operation, op::MOVE, attr::DURATION_US)?;
    let moves: Vec<MoveSpec> = parse_json_attr(operation, op::MOVE, attr::MOVES, "Vec<MoveSpec>")?;
    if moves.is_empty() {
        return Err(VerifyError::Arity {
            op: op::MOVE,
            role: "move",
            expected: ">= 1".to_string(),
            found: 0,
        });
    }
    Ok(())
}

fn verify_transfer<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    expect_counts(operation, op::TRANSFER, 0, 0)?;
    require_non_negative_i64(operation, op::TRANSFER, attr::ATOM)?;
    require_non_negative_i64(operation, op::TRANSFER, attr::SITE)?;
    require_non_negative_i64(operation, op::TRANSFER, attr::AOD_ID)?;
    require_non_negative_i64(operation, op::TRANSFER, attr::ROW)?;
    require_non_negative_i64(operation, op::TRANSFER, attr::COL)?;
    require_non_negative_i64(operation, op::TRANSFER, attr::DURATION_US)?;
    match require_string(operation, op::TRANSFER, attr::DIRECTION)?.as_str() {
        "slm_to_aod" | "aod_to_slm" => Ok(()),
        _ => Err(VerifyError::WrongAttributeType {
            op: op::TRANSFER,
            attr: attr::DIRECTION,
            expected: "\"slm_to_aod\" or \"aod_to_slm\"",
        }),
    }
}

fn verify_entangle<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    expect_counts(operation, op::ENTANGLE, 0, 0)?;
    require_non_negative_i64(operation, op::ENTANGLE, attr::DURATION_US)?;
    let pairs: Vec<EntanglePairSpec> = parse_json_attr(
        operation,
        op::ENTANGLE,
        attr::PAIRS,
        "Vec<EntanglePairSpec>",
    )?;
    if pairs.is_empty() {
        return Err(VerifyError::Arity {
            op: op::ENTANGLE,
            role: "pair",
            expected: ">= 1".to_string(),
            found: 0,
        });
    }
    Ok(())
}

fn verify_measure<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    expect_counts(operation, op::MEASURE, 1, 1)?;
    expect_operand_type(operation, op::MEASURE, 0, is_atom_type, ATOM_TYPE)?;
    expect_result_type(operation, op::MEASURE, 0, is_bit_type, BIT_TYPE)?;
    require_non_negative_i64(operation, op::MEASURE, attr::ATOM)?;
    require_non_negative_i64(operation, op::MEASURE, attr::DURATION_US)?;
    match require_string(operation, op::MEASURE, attr::BASIS)?.as_str() {
        "x" | "y" | "z" => Ok(()),
        _ => Err(VerifyError::WrongAttributeType {
            op: op::MEASURE,
            attr: attr::BASIS,
            expected: "\"x\", \"y\", or \"z\"",
        }),
    }
}

fn verify_reset<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    expect_counts(operation, op::RESET, 1, 1)?;
    expect_operand_type(operation, op::RESET, 0, is_atom_type, ATOM_TYPE)?;
    expect_result_type(operation, op::RESET, 0, is_atom_type, ATOM_TYPE)?;
    require_non_negative_i64(operation, op::RESET, attr::ATOM)?;
    require_non_negative_i64(operation, op::RESET, attr::DURATION_US)?;
    Ok(())
}

fn verify_wait<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    expect_counts(operation, op::WAIT, 0, 0)?;
    require_non_negative_i64(operation, op::WAIT, attr::DURATION_US)?;
    Ok(())
}

#[derive(Clone, Copy)]
struct ScheduleLimits {
    rydberg_range_um: f64,
    min_rydberg_spacing_um: f64,
    aod_min_separation_um: f64,
}

fn verify_schedule<'c: 'a, 'a, O: OperationLike<'c, 'a>>(operation: &O) -> Result<(), VerifyError> {
    expect_counts(operation, op::SCHEDULE, 0, 0)?;
    require_string(operation, op::SCHEDULE, attr::TARGET_ID)?;
    let limits = ScheduleLimits {
        rydberg_range_um: require_positive_f64(operation, op::SCHEDULE, attr::RYDBERG_RANGE_UM)?,
        min_rydberg_spacing_um: require_positive_f64(
            operation,
            op::SCHEDULE,
            attr::MIN_RYDBERG_SPACING_UM,
        )?,
        aod_min_separation_um: require_positive_f64(
            operation,
            op::SCHEDULE,
            attr::AOD_MIN_SEPARATION_UM,
        )?,
    };

    if operation.region_count() != 1 {
        return Err(VerifyError::RegionCount {
            op: op::SCHEDULE,
            expected: 1,
            found: operation.region_count(),
        });
    }
    let region = operation
        .region(0)
        .map_err(|_| VerifyError::MissingRegion { op: op::SCHEDULE })?;
    let block = region
        .first_block()
        .ok_or(VerifyError::MissingRegion { op: op::SCHEDULE })?;
    let mut inner = block.first_operation();
    let mut aod_bindings: BTreeMap<u32, AodTrapKey> = BTreeMap::new();
    let mut layer_facts = Vec::new();
    while let Some(operation) = inner {
        let identifier = operation.name();
        let name = identifier.as_string_ref().as_str().unwrap_or("").to_owned();
        if name != op::LAYER {
            return Err(VerifyError::ForbiddenOp {
                op: op::SCHEDULE,
                found: name,
            });
        }
        let facts = verify_layer(&operation, Some(limits), Some(&mut aod_bindings))?;
        layer_facts.push(facts);
        inner = operation.next_in_block();
    }
    verify_schedule_ordering(&layer_facts)
}

/// `(aod_id, row, col)` of an AOD trap, as carried by transfer and move ops.
type AodTrapKey = (u32, u32, u32);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AtomEvent {
    Measure(u32),
    Reset(u32),
    Use(u32),
}

#[derive(Clone, Debug, Default)]
struct LayerFacts {
    cycle: u32,
    has_wait: bool,
    events: Vec<AtomEvent>,
}

fn verify_layer<'c: 'a, 'a, O: OperationLike<'c, 'a>>(
    operation: &O,
    limits: Option<ScheduleLimits>,
    aod_bindings: Option<&mut BTreeMap<u32, AodTrapKey>>,
) -> Result<LayerFacts, VerifyError> {
    expect_counts(operation, op::LAYER, 0, 0)?;
    let cycle = require_non_negative_i64(operation, op::LAYER, attr::CYCLE)? as u32;
    if operation.region_count() != 1 {
        return Err(VerifyError::RegionCount {
            op: op::LAYER,
            expected: 1,
            found: operation.region_count(),
        });
    }
    let region = operation
        .region(0)
        .map_err(|_| VerifyError::MissingRegion { op: op::LAYER })?;
    verify_layer_region(region, cycle, limits, aod_bindings)
}

fn verify_layer_region(
    region: RegionRef<'_, '_>,
    cycle: u32,
    limits: Option<ScheduleLimits>,
    aod_bindings: Option<&mut BTreeMap<u32, AodTrapKey>>,
) -> Result<LayerFacts, VerifyError> {
    let block = region
        .first_block()
        .ok_or(VerifyError::MissingRegion { op: op::LAYER })?;
    let mut context = LayerContext::default();
    // Transfers take effect for later layers (an atom never transfers and
    // moves in one layer), so binding updates are buffered until the walk
    // finishes.
    let mut binding_updates: Vec<(u32, TransferDirection, AodTrapKey)> = Vec::new();
    let mut facts = LayerFacts {
        cycle,
        has_wait: false,
        events: Vec::new(),
    };
    let mut inner = block.first_operation();
    while let Some(operation) = inner {
        let identifier = operation.name();
        let name = identifier.as_string_ref().as_str().unwrap_or("").to_owned();
        match name.as_str() {
            op::MOVE => {
                verify_move(&operation)?;
                let moves: Vec<MoveSpec> =
                    parse_json_attr(&operation, op::MOVE, attr::MOVES, "Vec<MoveSpec>")?;
                if let Some(bindings) = aod_bindings.as_deref() {
                    for spec in &moves {
                        check_bound_trap(
                            cycle,
                            spec.atom,
                            (spec.aod_id, spec.row, spec.col),
                            bindings,
                        )?;
                    }
                }
                for spec in &moves {
                    facts.events.push(AtomEvent::Use(spec.atom));
                }
                context.record_moves(cycle, &moves)?;
            }
            op::TRANSFER => {
                verify_transfer(&operation)?;
                let atom = require_non_negative_i64(&operation, op::TRANSFER, attr::ATOM)? as u32;
                let site = require_non_negative_i64(&operation, op::TRANSFER, attr::SITE)? as u32;
                context.claim_occupancy(cycle, atom, site)?;
                facts.events.push(AtomEvent::Use(atom));
                if aod_bindings.is_some() {
                    let trap = (
                        require_non_negative_i64(&operation, op::TRANSFER, attr::AOD_ID)? as u32,
                        require_non_negative_i64(&operation, op::TRANSFER, attr::ROW)? as u32,
                        require_non_negative_i64(&operation, op::TRANSFER, attr::COL)? as u32,
                    );
                    let direction =
                        match require_string(&operation, op::TRANSFER, attr::DIRECTION)?.as_str() {
                            "slm_to_aod" => TransferDirection::SlmToAod,
                            _ => TransferDirection::AodToSlm,
                        };
                    if direction == TransferDirection::AodToSlm
                        && let Some(bindings) = aod_bindings.as_deref()
                    {
                        // A store references the trap the atom rode in.
                        check_bound_trap(cycle, atom, trap, bindings)?;
                    }
                    binding_updates.push((atom, direction, trap));
                }
            }
            op::ENTANGLE => {
                verify_entangle(&operation)?;
                let pairs: Vec<EntanglePairSpec> = parse_json_attr(
                    &operation,
                    op::ENTANGLE,
                    attr::PAIRS,
                    "Vec<EntanglePairSpec>",
                )?;
                for pair in &pairs {
                    facts.events.push(AtomEvent::Use(pair.lhs.atom));
                    facts.events.push(AtomEvent::Use(pair.rhs.atom));
                }
                context.record_entangle(cycle, &pairs)?;
            }
            op::MEASURE => {
                verify_measure(&operation)?;
                let atom = require_non_negative_i64(&operation, op::MEASURE, attr::ATOM)? as u32;
                facts.events.push(AtomEvent::Measure(atom));
            }
            op::RESET => {
                verify_reset(&operation)?;
                let atom = require_non_negative_i64(&operation, op::RESET, attr::ATOM)? as u32;
                facts.events.push(AtomEvent::Reset(atom));
            }
            op::WAIT => {
                verify_wait(&operation)?;
                facts.has_wait = true;
            }
            op::ALLOC_ATOM => {
                verify_alloc_atom(&operation)?;
            }
            op::PLACE => {
                verify_place(&operation)?;
            }
            _ => {
                return Err(VerifyError::ForbiddenOp {
                    op: op::LAYER,
                    found: name,
                });
            }
        }
        inner = operation.next_in_block();
    }

    if let Some(limits) = limits {
        context.verify_entangling_geometry(cycle, limits)?;
        context.verify_aod_legality(cycle, limits.aod_min_separation_um)?;
    }
    if let Some(bindings) = aod_bindings {
        for (atom, direction, trap) in binding_updates {
            match direction {
                TransferDirection::SlmToAod => {
                    bindings.insert(atom, trap);
                }
                TransferDirection::AodToSlm => {
                    bindings.remove(&atom);
                }
            }
        }
    }
    Ok(facts)
}

/// Cross-layer measurement / reset / Wait round-barrier ordering (ADR-0021).
fn verify_schedule_ordering(layers: &[LayerFacts]) -> Result<(), VerifyError> {
    let mut previous_cycle: Option<u32> = None;
    for layer in layers {
        if let Some(prev) = previous_cycle
            && layer.cycle < prev
        {
            return Err(VerifyError::NonMonotonicCycles {
                previous_cycle: prev,
                cycle: layer.cycle,
            });
        }
        previous_cycle = Some(layer.cycle);
    }

    for (i, layer) in layers.iter().enumerate() {
        if !layer.has_wait {
            continue;
        }
        for later in layers.iter().skip(i + 1) {
            if later.cycle <= layer.cycle {
                return Err(VerifyError::RoundBarrierCycleOrder {
                    wait_cycle: layer.cycle,
                    after_cycle: later.cycle,
                });
            }
        }
    }

    #[derive(Clone, Copy)]
    enum AtomPhase {
        /// Measured and not yet reset.
        Measured { cycle: u32 },
        /// Reset completed; atom may be reused in a later cycle.
        Reset { cycle: u32 },
    }

    let mut phase: BTreeMap<u32, AtomPhase> = BTreeMap::new();
    let mut measured_this_cycle: BTreeSet<u32> = BTreeSet::new();
    let mut reset_this_cycle: BTreeSet<u32> = BTreeSet::new();
    let mut last_cycle: Option<u32> = None;

    for layer in layers {
        if last_cycle != Some(layer.cycle) {
            measured_this_cycle.clear();
            reset_this_cycle.clear();
            last_cycle = Some(layer.cycle);
        }

        for event in &layer.events {
            match *event {
                AtomEvent::Measure(atom) => {
                    if let Some(AtomPhase::Reset {
                        cycle: reset_cycle,
                    }) = phase.get(&atom).copied()
                        && layer.cycle == reset_cycle
                    {
                        return Err(VerifyError::ResetBeforeMeasure {
                            atom,
                            reset_cycle,
                            measure_cycle: layer.cycle,
                        });
                    }
                    phase.insert(atom, AtomPhase::Measured { cycle: layer.cycle });
                    measured_this_cycle.insert(atom);
                }
                AtomEvent::Reset(atom) => {
                    if let Some(AtomPhase::Measured {
                        cycle: measure_cycle,
                    }) = phase.get(&atom).copied()
                        && layer.cycle < measure_cycle
                    {
                        return Err(VerifyError::ResetBeforeMeasure {
                            atom,
                            reset_cycle: layer.cycle,
                            measure_cycle,
                        });
                    }
                    phase.insert(atom, AtomPhase::Reset { cycle: layer.cycle });
                    reset_this_cycle.insert(atom);
                    measured_this_cycle.remove(&atom);
                }
                AtomEvent::Use(atom) => {
                    if measured_this_cycle.contains(&atom) {
                        return Err(VerifyError::MeasureUseSameCycle {
                            cycle: layer.cycle,
                            atom,
                        });
                    }
                    if reset_this_cycle.contains(&atom) {
                        return Err(VerifyError::ResetUseSameCycle {
                            cycle: layer.cycle,
                            atom,
                        });
                    }
                    match phase.get(&atom).copied() {
                        Some(AtomPhase::Measured {
                            cycle: measure_cycle,
                        }) => {
                            return Err(VerifyError::MeasureReuseWithoutReset {
                                atom,
                                measure_cycle,
                                reuse_cycle: layer.cycle,
                            });
                        }
                        Some(AtomPhase::Reset { cycle: reset_cycle }) => {
                            if layer.cycle <= reset_cycle {
                                return Err(VerifyError::ResetUseSameCycle {
                                    cycle: layer.cycle,
                                    atom,
                                });
                            }
                            phase.remove(&atom);
                        }
                        None => {}
                    }
                }
            }
        }
    }

    Ok(())
}

/// A move or store must use the trap its atom's `slm_to_aod` load
/// established. Atoms with no recorded load (hand-built move-only specs)
/// are exempt.
fn check_bound_trap(
    cycle: u32,
    atom: u32,
    trap: AodTrapKey,
    bindings: &BTreeMap<u32, AodTrapKey>,
) -> Result<(), VerifyError> {
    match bindings.get(&atom) {
        Some(&bound) if bound != trap => Err(VerifyError::AodRefMismatch {
            cycle,
            atom,
            aod_id: trap.0,
            row: trap.1,
            col: trap.2,
            bound_aod_id: bound.0,
            bound_row: bound.1,
            bound_col: bound.2,
        }),
        _ => Ok(()),
    }
}

#[derive(Default)]
struct LayerContext {
    claimed_atoms: BTreeSet<u32>,
    claimed_sites: BTreeSet<u32>,
    entangling_atoms: BTreeSet<u32>,
    entangle_pairs: Vec<EntanglePairSpec>,
    moves: Vec<MoveSpec>,
    /// Source position each AOD trap was claimed from this cycle.
    trap_claims: BTreeMap<AodTrapKey, (f64, f64)>,
}

impl LayerContext {
    fn claim_occupancy(&mut self, cycle: u32, atom: u32, site: u32) -> Result<(), VerifyError> {
        if !self.claimed_atoms.insert(atom) {
            return Err(VerifyError::DuplicateOccupancyAtom { cycle, atom });
        }
        if !self.claimed_sites.insert(site) {
            return Err(VerifyError::DuplicateOccupancySite { cycle, site });
        }
        Ok(())
    }

    fn record_moves(&mut self, cycle: u32, moves: &[MoveSpec]) -> Result<(), VerifyError> {
        for atom_move in moves {
            self.claim_occupancy(cycle, atom_move.atom, atom_move.to_site)?;
            // One AOD trap can hold one atom: a second same-cycle claim is
            // only tolerable as an exact duplicate (same source position).
            let trap = (atom_move.aod_id, atom_move.row, atom_move.col);
            let source = (atom_move.from_x_um, atom_move.from_y_um);
            if let Some(&(x, y)) = self.trap_claims.get(&trap) {
                if x.total_cmp(&source.0).is_ne() || y.total_cmp(&source.1).is_ne() {
                    return Err(VerifyError::AodTrapDoubleClaim {
                        cycle,
                        aod_id: trap.0,
                        row: trap.1,
                        col: trap.2,
                    });
                }
            } else {
                self.trap_claims.insert(trap, source);
            }
            self.moves.push(*atom_move);
        }
        Ok(())
    }

    fn record_entangle(
        &mut self,
        cycle: u32,
        pairs: &[EntanglePairSpec],
    ) -> Result<(), VerifyError> {
        for pair in pairs {
            for atom in [pair.lhs.atom, pair.rhs.atom] {
                if !self.entangling_atoms.insert(atom) {
                    return Err(VerifyError::DuplicateEntanglingAtom { cycle, atom });
                }
            }
            self.entangle_pairs.push(*pair);
        }
        Ok(())
    }

    fn verify_entangling_geometry(
        &self,
        cycle: u32,
        limits: ScheduleLimits,
    ) -> Result<(), VerifyError> {
        let mut partner_pairs = BTreeSet::new();
        let mut atoms = Vec::new();
        for pair in &self.entangle_pairs {
            let distance = distance_um(pair.lhs, pair.rhs);
            if distance > limits.rydberg_range_um {
                return Err(VerifyError::EntanglingPairOutOfRange {
                    cycle,
                    lhs: pair.lhs.atom,
                    rhs: pair.rhs.atom,
                    distance_um: distance,
                    rydberg_range_um: limits.rydberg_range_um,
                });
            }
            partner_pairs.insert(atom_pair_key(pair.lhs.atom, pair.rhs.atom));
            atoms.push(pair.lhs);
            atoms.push(pair.rhs);
        }

        for i in 0..atoms.len() {
            for rhs in atoms.iter().skip(i + 1) {
                let lhs = atoms[i];
                if partner_pairs.contains(&atom_pair_key(lhs.atom, rhs.atom)) {
                    continue;
                }
                let distance = distance_um(lhs, *rhs);
                if distance <= limits.rydberg_range_um {
                    return Err(VerifyError::CompulsoryEntanglement {
                        cycle,
                        lhs: lhs.atom,
                        rhs: rhs.atom,
                        distance_um: distance,
                        rydberg_range_um: limits.rydberg_range_um,
                    });
                }
                if distance <= limits.min_rydberg_spacing_um {
                    return Err(VerifyError::RydbergSpacing {
                        cycle,
                        lhs: lhs.atom,
                        rhs: rhs.atom,
                        distance_um: distance,
                        min_spacing_um: limits.min_rydberg_spacing_um,
                    });
                }
            }
        }
        Ok(())
    }

    fn verify_aod_legality(&self, cycle: u32, min_separation_um: f64) -> Result<(), VerifyError> {
        verify_coupled_motion(cycle, &self.moves)?;
        verify_axis_order_and_separation(cycle, &self.moves, Axis::Row, min_separation_um)?;
        verify_axis_order_and_separation(cycle, &self.moves, Axis::Column, min_separation_um)
    }
}

fn verify_coupled_motion(cycle: u32, moves: &[MoveSpec]) -> Result<(), VerifyError> {
    let mut row_deltas: BTreeMap<(u32, u32), f64> = BTreeMap::new();
    let mut col_deltas: BTreeMap<(u32, u32), f64> = BTreeMap::new();

    for atom_move in moves {
        let row_key = (atom_move.aod_id, atom_move.row);
        let row_delta = atom_move.to_y_um - atom_move.from_y_um;
        if let Some(existing) = row_deltas.insert(row_key, row_delta)
            && !same_coordinate(existing, row_delta)
        {
            return Err(VerifyError::AodRowCoupling {
                cycle,
                aod_id: atom_move.aod_id,
                row: atom_move.row,
            });
        }

        let col_key = (atom_move.aod_id, atom_move.col);
        let col_delta = atom_move.to_x_um - atom_move.from_x_um;
        if let Some(existing) = col_deltas.insert(col_key, col_delta)
            && !same_coordinate(existing, col_delta)
        {
            return Err(VerifyError::AodColumnCoupling {
                cycle,
                aod_id: atom_move.aod_id,
                col: atom_move.col,
            });
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum Axis {
    Row,
    Column,
}

fn verify_axis_order_and_separation(
    cycle: u32,
    moves: &[MoveSpec],
    axis: Axis,
    min_separation_um: f64,
) -> Result<(), VerifyError> {
    let mut representatives: BTreeMap<(u32, u32), (f64, f64)> = BTreeMap::new();
    for atom_move in moves {
        let key = match axis {
            Axis::Row => (atom_move.aod_id, atom_move.row),
            Axis::Column => (atom_move.aod_id, atom_move.col),
        };
        let coordinates = match axis {
            Axis::Row => (atom_move.from_y_um, atom_move.to_y_um),
            Axis::Column => (atom_move.from_x_um, atom_move.to_x_um),
        };
        representatives.entry(key).or_insert(coordinates);
    }

    let items: Vec<_> = representatives.into_iter().collect();
    for i in 0..items.len() {
        for ((rhs_aod, rhs_index), (rhs_from, rhs_to)) in items.iter().skip(i + 1) {
            let ((lhs_aod, lhs_index), (lhs_from, lhs_to)) = items[i];
            if lhs_aod != *rhs_aod {
                continue;
            }

            let source_order = lhs_from.total_cmp(rhs_from);
            let dest_order = lhs_to.total_cmp(rhs_to);
            if source_order != dest_order {
                return match axis {
                    Axis::Row => Err(VerifyError::AodRowOrder {
                        cycle,
                        aod_id: lhs_aod,
                        first: lhs_index,
                        second: *rhs_index,
                    }),
                    Axis::Column => Err(VerifyError::AodColumnOrder {
                        cycle,
                        aod_id: lhs_aod,
                        first: lhs_index,
                        second: *rhs_index,
                    }),
                };
            }

            if (lhs_to - rhs_to).abs() < min_separation_um {
                return match axis {
                    Axis::Row => Err(VerifyError::AodRowSeparation {
                        cycle,
                        aod_id: lhs_aod,
                        first: lhs_index,
                        second: *rhs_index,
                        min_separation_um,
                    }),
                    Axis::Column => Err(VerifyError::AodColumnSeparation {
                        cycle,
                        aod_id: lhs_aod,
                        first: lhs_index,
                        second: *rhs_index,
                        min_separation_um,
                    }),
                };
            }
        }
    }
    Ok(())
}

fn same_coordinate(lhs: f64, rhs: f64) -> bool {
    (lhs - rhs).abs() <= f64::EPSILON
}

fn distance_um(lhs: PositionedAtom, rhs: PositionedAtom) -> f64 {
    let dx = lhs.x_um - rhs.x_um;
    let dy = lhs.y_um - rhs.y_um;
    (dx * dx + dy * dy).sqrt()
}

fn atom_pair_key(lhs: u32, rhs: u32) -> (u32, u32) {
    if lhs <= rhs { (lhs, rhs) } else { (rhs, lhs) }
}

fn i64_attr(context: &Context, value: i64) -> Attribute<'_> {
    IntegerAttribute::new(IntegerType::new(context, 64).into(), value).into()
}

fn f64_attr(context: &Context, value: f64) -> Attribute<'_> {
    let float_type = Type::parse(context, "f64").unwrap_or_else(|| Type::none(context));
    FloatAttribute::new(context, float_type, value).into()
}

fn string_attr<'c>(context: &'c Context, value: &str) -> Attribute<'c> {
    StringAttribute::new(context, value).into()
}

fn json_attr<'c, T: Serialize>(
    context: &'c Context,
    value: &T,
) -> Result<Attribute<'c>, BuildError> {
    let text = serde_json::to_string(value).map_err(|error| {
        BuildError::Verify(VerifyError::JsonAttribute {
            op: NAMESPACE,
            attr: "json",
            expected: "serializable JSON",
            message: error.to_string(),
        })
    })?;
    Ok(string_attr(context, &text))
}

fn named_attr<'c>(
    context: &'c Context,
    name: &str,
    value: Attribute<'c>,
) -> (Identifier<'c>, Attribute<'c>) {
    (Identifier::new(context, name), value)
}

fn finish(builder: OperationBuilder) -> Result<Operation, BuildError> {
    let operation = builder.build()?;
    verify(&operation)?;
    Ok(operation)
}

pub fn alloc_atom<'c>(
    context: &'c Context,
    atom: u32,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::ALLOC_ATOM, location)
            .add_results(&[atom_type(context)])
            .add_attributes(&[named_attr(
                context,
                attr::ATOM,
                i64_attr(context, i64::from(atom)),
            )]),
    )
}

pub fn place<'c>(
    context: &'c Context,
    atom_value: Value<'c, '_>,
    placement: MoveSpec,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::PLACE, location)
            .add_operands(&[atom_value])
            .add_results(&[atom_type(context)])
            .add_attributes(&[
                named_attr(
                    context,
                    attr::ATOM,
                    i64_attr(context, i64::from(placement.atom)),
                ),
                named_attr(
                    context,
                    attr::SITE,
                    i64_attr(context, i64::from(placement.from_site)),
                ),
                named_attr(context, attr::X_UM, f64_attr(context, placement.from_x_um)),
                named_attr(context, attr::Y_UM, f64_attr(context, placement.from_y_um)),
                named_attr(context, attr::TRAP, string_attr(context, "aod")),
                named_attr(
                    context,
                    attr::AOD_ID,
                    i64_attr(context, i64::from(placement.aod_id)),
                ),
                named_attr(
                    context,
                    attr::ROW,
                    i64_attr(context, i64::from(placement.row)),
                ),
                named_attr(
                    context,
                    attr::COL,
                    i64_attr(context, i64::from(placement.col)),
                ),
            ]),
    )
}

pub fn move_group<'c>(
    context: &'c Context,
    moves: &[MoveSpec],
    duration_us: u64,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(OperationBuilder::new(op::MOVE, location).add_attributes(&[
        named_attr(context, attr::MOVES, json_attr(context, &moves)?),
        named_attr(
            context,
            attr::DURATION_US,
            i64_attr(context, duration_us as i64),
        ),
    ]))
}

pub fn transfer<'c>(
    context: &'c Context,
    transfer: TransferSpec,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    let direction = match transfer.direction {
        TransferDirection::SlmToAod => "slm_to_aod",
        TransferDirection::AodToSlm => "aod_to_slm",
    };
    finish(
        OperationBuilder::new(op::TRANSFER, location).add_attributes(&[
            named_attr(
                context,
                attr::ATOM,
                i64_attr(context, i64::from(transfer.atom)),
            ),
            named_attr(
                context,
                attr::SITE,
                i64_attr(context, i64::from(transfer.site)),
            ),
            named_attr(
                context,
                attr::AOD_ID,
                i64_attr(context, i64::from(transfer.aod_id)),
            ),
            named_attr(
                context,
                attr::ROW,
                i64_attr(context, i64::from(transfer.row)),
            ),
            named_attr(
                context,
                attr::COL,
                i64_attr(context, i64::from(transfer.col)),
            ),
            named_attr(context, attr::DIRECTION, string_attr(context, direction)),
            named_attr(
                context,
                attr::DURATION_US,
                i64_attr(context, transfer.duration_us as i64),
            ),
        ]),
    )
}

pub fn entangle<'c>(
    context: &'c Context,
    pairs: &[EntanglePairSpec],
    duration_us: u64,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::ENTANGLE, location).add_attributes(&[
            named_attr(context, attr::PAIRS, json_attr(context, &pairs)?),
            named_attr(
                context,
                attr::DURATION_US,
                i64_attr(context, duration_us as i64),
            ),
        ]),
    )
}

pub fn measure<'c>(
    context: &'c Context,
    atom_value: Value<'c, '_>,
    atom: u32,
    basis: &str,
    duration_us: u64,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::MEASURE, location)
            .add_operands(&[atom_value])
            .add_results(&[bit_type(context)])
            .add_attributes(&[
                named_attr(context, attr::ATOM, i64_attr(context, i64::from(atom))),
                named_attr(context, attr::BASIS, string_attr(context, basis)),
                named_attr(
                    context,
                    attr::DURATION_US,
                    i64_attr(context, duration_us as i64),
                ),
            ]),
    )
}

pub fn reset<'c>(
    context: &'c Context,
    atom_value: Value<'c, '_>,
    atom: u32,
    duration_us: u64,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::RESET, location)
            .add_operands(&[atom_value])
            .add_results(&[atom_type(context)])
            .add_attributes(&[
                named_attr(context, attr::ATOM, i64_attr(context, i64::from(atom))),
                named_attr(
                    context,
                    attr::DURATION_US,
                    i64_attr(context, duration_us as i64),
                ),
            ]),
    )
}

pub fn wait<'c>(
    context: &'c Context,
    duration_us: u64,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::WAIT, location).add_attributes(&[named_attr(
            context,
            attr::DURATION_US,
            i64_attr(context, duration_us as i64),
        )]),
    )
}

pub fn layer<'c>(
    context: &'c Context,
    cycle: u32,
    body: Region<'c>,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::LAYER, location)
            .add_regions([body])
            .add_attributes(&[named_attr(
                context,
                attr::CYCLE,
                i64_attr(context, i64::from(cycle)),
            )]),
    )
}

pub fn schedule<'c>(
    context: &'c Context,
    spec: &ScheduleSpec,
    body: Region<'c>,
    location: Location<'c>,
) -> Result<Operation<'c>, BuildError> {
    finish(
        OperationBuilder::new(op::SCHEDULE, location)
            .add_regions([body])
            .add_attributes(&[
                named_attr(
                    context,
                    attr::TARGET_ID,
                    string_attr(context, &spec.target_id),
                ),
                named_attr(
                    context,
                    attr::RYDBERG_RANGE_UM,
                    f64_attr(context, spec.rydberg_range_um),
                ),
                named_attr(
                    context,
                    attr::MIN_RYDBERG_SPACING_UM,
                    f64_attr(context, spec.min_rydberg_spacing_um),
                ),
                named_attr(
                    context,
                    attr::AOD_MIN_SEPARATION_UM,
                    f64_attr(context, spec.aod_min_separation_um),
                ),
            ]),
    )
}

pub fn schedule_module<'c>(
    context: &'c Context,
    spec: &ScheduleSpec,
) -> Result<Module<'c>, BuildError> {
    register_dialect(context);
    let location = Location::unknown(context);
    let schedule_region = Region::new();
    let schedule_block = Block::new(&[]);

    for layer_spec in &spec.layers {
        let layer_region = Region::new();
        let layer_block = Block::new(&[]);
        for action in &layer_spec.actions {
            append_action(context, &layer_block, action, location)?;
        }
        layer_region.append_block(layer_block);
        schedule_block.append_operation(layer(context, layer_spec.cycle, layer_region, location)?);
    }

    schedule_region.append_block(schedule_block);
    let module = Module::new(location);
    module
        .body()
        .append_operation(schedule(context, spec, schedule_region, location)?);
    Ok(module)
}

/// Builds and prints a generic-form MLIR module for the neutral-atom schedule.
///
/// This is the Rust-level `--dump-na` path for issue #102; the CLI can call it
/// in a later slice after lowering produces a [`ScheduleSpec`].
pub fn dump_schedule_text(spec: &ScheduleSpec) -> Result<String, BuildError> {
    let context = Context::new();
    let module = schedule_module(&context, spec)?;
    Ok(module.as_operation().to_string())
}

fn append_action<'c, B: BlockLike<'c, 'c>>(
    context: &'c Context,
    block: &B,
    action: &ActionSpec,
    location: Location<'c>,
) -> Result<(), BuildError> {
    match action {
        ActionSpec::Move { moves, duration_us } => {
            block.append_operation(move_group(context, moves, *duration_us, location)?);
        }
        ActionSpec::Transfer(transfer_spec) => {
            block.append_operation(transfer(context, *transfer_spec, location)?);
        }
        ActionSpec::Entangle { pairs, duration_us } => {
            block.append_operation(entangle(context, pairs, *duration_us, location)?);
        }
        ActionSpec::Measure {
            atom,
            basis,
            duration_us,
        } => {
            let atom_ref = append_synthetic_atom(context, block, *atom, location)?;
            block.append_operation(measure(
                context,
                atom_ref,
                *atom,
                basis,
                *duration_us,
                location,
            )?);
        }
        ActionSpec::Reset { atom, duration_us } => {
            let atom_ref = append_synthetic_atom(context, block, *atom, location)?;
            block.append_operation(reset(context, atom_ref, *atom, *duration_us, location)?);
        }
        ActionSpec::Wait { duration_us } => {
            block.append_operation(wait(context, *duration_us, location)?);
        }
    }
    Ok(())
}

fn append_synthetic_atom<'c, B: BlockLike<'c, 'c>>(
    context: &'c Context,
    block: &B,
    atom: u32,
    location: Location<'c>,
) -> Result<Value<'c, 'c>, BuildError> {
    let alloc = block.append_operation(alloc_atom(context, atom, location)?);
    let result = alloc.result(0).map_err(|_| BuildError::MissingResult {
        op: op::ALLOC_ATOM,
        index: 0,
    })?;
    Ok(Value::from(result))
}
