//! Types for the flat AOD movement planner: parameters, results, errors,
//! and the dialect-aligned move / leg payloads.
//!
//! # Invariants
//!
//! - [`MovementParams`] is the sole configuration surface; `validate()` is the
//!   gatekeeper for all numeric constraints.
//! - [`MovementPlanError`] is the single error enum surfaced by
//!   `plan_aod_movement` and helper functions.
//! - [`MoveSpec`] mirrors the MLIR dialect's move payload so verifier tests
//!   can exercise legality without an MLIR context.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::layout::{AtomId, SiteId};
use crate::schedule_entry::GraphScheduleRequest;

/// Extra µm beyond `pair_pitch_um` so placement↔bank edge is dialect-strict `>` min (B14).
pub const BANK_ISOLATION_EPS_UM: f64 = 0.01;

/// Parameters for flat AOD movement (#106).
///
/// Sourced from NeutralAtomTarget JSON by callers; kept crate-local to avoid a
/// backend dependency (same pattern as #105).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementParams {
    pub acceleration_m_s2: f64,
    pub trap_transfer_us: u64,
    pub rydberg_range_um: f64,
    /// R3 isolation: non-partners must be farther than this (generic_rna: 18.75).
    pub min_rydberg_spacing_um: f64,
    pub min_row_col_separation_um: f64,
    pub aod_rows: u32,
    pub aod_cols: u32,
    pub num_aods: u32,
    /// Intra-pair gap (µm); must satisfy `0 < pair_gap_um ≤ rydberg_range_um`.
    pub pair_gap_um: f64,
    /// Center-to-center pitch between distinct interaction pairs.
    /// May equal `min_rydberg_spacing_um`; bank origin adds [`BANK_ISOLATION_EPS_UM`] (B14).
    pub pair_pitch_um: f64,
    /// When true, return bank occupants home after each entangle (Enola-comparable 4-xfer).
    /// When false (default), Quon reuse: 2 transfers per moved atom (load+store); B8/B13
    /// reclaim between layers. This flag alone selects the 2- vs 4-transfer policy (B6).
    pub return_home: bool,
}

impl MovementParams {
    /// Defaults matching `targets/neutral_atom/generic_rna_v0.json` / architecture §8.6.
    pub fn generic_rna_v0() -> Self {
        Self {
            acceleration_m_s2: 2750.0,
            trap_transfer_us: 15,
            rydberg_range_um: 7.5,
            min_rydberg_spacing_um: 18.75,
            min_row_col_separation_um: 2.0,
            aod_rows: 100,
            aod_cols: 100,
            num_aods: 1,
            pair_gap_um: 2.0,
            pair_pitch_um: 18.75,
            return_home: false,
        }
    }

    pub(crate) fn validate(&self) -> Result<(), MovementPlanError> {
        if self.acceleration_m_s2 <= 0.0 {
            return Err(MovementPlanError::InvalidAcceleration(
                self.acceleration_m_s2,
            ));
        }
        if self.rydberg_range_um <= 0.0 {
            return Err(MovementPlanError::InvalidRydbergRange(
                self.rydberg_range_um,
            ));
        }
        if self.min_rydberg_spacing_um <= 0.0 {
            return Err(MovementPlanError::InvalidMinRydbergSpacing(
                self.min_rydberg_spacing_um,
            ));
        }
        if self.min_row_col_separation_um <= 0.0 {
            return Err(MovementPlanError::InvalidMinRydbergSeparation(
                self.min_row_col_separation_um,
            ));
        }
        if self.pair_gap_um <= 0.0 || self.pair_gap_um > self.rydberg_range_um {
            return Err(MovementPlanError::InvalidPairGap {
                gap: self.pair_gap_um,
                rb: self.rydberg_range_um,
            });
        }
        if self.pair_pitch_um < self.min_rydberg_spacing_um {
            return Err(MovementPlanError::InvalidPairPitch {
                pitch: self.pair_pitch_um,
                min: self.min_rydberg_spacing_um,
            });
        }
        if self.pair_pitch_um <= self.pair_gap_um {
            return Err(MovementPlanError::InvalidPairPitch {
                pitch: self.pair_pitch_um,
                min: self.pair_gap_um,
            });
        }
        if self.num_aods == 0 {
            return Err(MovementPlanError::AodCapacity {
                axis: "aods",
                needed: 1,
                limit: 0,
            });
        }
        Ok(())
    }
}

/// Result of [`super::plan_aod_movement`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MovementPlanResult {
    pub request: GraphScheduleRequest,
    pub rearrangement_steps: u64,
    pub rearrangement_time_us: u64,
    pub trap_transfers: u64,
    pub transfer_time_us: u64,
    pub skipped_already_adjacent: u64,
}

/// Errors from flat AOD movement planning.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum MovementPlanError {
    #[error("layout is required for AOD movement planning")]
    MissingLayout,
    #[error("no entangling layers to plan movement for")]
    EmptySchedule,
    #[error("acceleration_m_s2 must be positive, got {0}")]
    InvalidAcceleration(f64),
    #[error("rydberg_range_um must be positive, got {0}")]
    InvalidRydbergRange(f64),
    #[error("min_rydberg_spacing_um must be positive, got {0}")]
    InvalidMinRydbergSpacing(f64),
    #[error("min_row_col_separation_um must be positive, got {0}")]
    InvalidMinRydbergSeparation(f64),
    #[error("pair_gap_um ({gap}) must be in (0, rydberg_range_um={rb}]")]
    InvalidPairGap { gap: f64, rb: f64 },
    #[error("pair_pitch_um ({pitch}) must be >= min_rydberg_spacing_um ({min})")]
    InvalidPairPitch { pitch: f64, min: f64 },
    #[error("missing site or binding for atom {0:?}")]
    MissingAtom(AtomId),
    #[error("collision: site {site:?} claimed by multiple atoms in cycle {cycle}")]
    Collision { cycle: u32, site: SiteId },
    #[error("transfer into occupied site {site:?} at cycle {cycle}")]
    TransferIntoOccupied { cycle: u32, site: SiteId },
    #[error("AOD capacity exceeded: need {needed} {axis} but target allows {limit}")]
    AodCapacity {
        axis: &'static str,
        needed: u32,
        limit: u32,
    },
    #[error("unsatisfiable move set under AOD conflicts for layer cycle {0}")]
    Unsatisfiable(u32),
    #[error("no free interaction pair for gate ({lhs:?}, {rhs:?}) at cycle {cycle}")]
    NoInteractionPair {
        cycle: u32,
        lhs: AtomId,
        rhs: AtomId,
    },
    #[error(
        "entangling geometry violation (R1–R3) at cycle {cycle}: {detail}; the flat AOD planner checks all occupied atoms, idle ones included (B11, fail-closed) — a placement grid denser than the target's Rydberg limits cannot entangle legally; use the zoned backend (--na-backend zoned) for such targets"
    )]
    EntanglingGeometry { cycle: u32, detail: String },
    #[error("multi-qubit entangle (k={k}) unsupported by flat movement planner at cycle {cycle}")]
    UnsupportedEntangleArity { cycle: u32, k: usize },
    #[error("schedule layer conflict: {0}")]
    Conflict(String),
    #[error("missing site {0:?} in layout")]
    MissingSite(SiteId),
}

/// One reserved interaction pair: two empty sites with gap ≤ r_b.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InteractionPair {
    pub left: SiteId,
    pub right: SiteId,
}

/// Dialect-aligned move payload for verifier tests (MLIR-free).
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

/// One planned leg with coordinates and AOD indices (planner-internal).
#[derive(Clone, Debug, PartialEq)]
pub struct CandidateLeg {
    pub atom: AtomId,
    pub from: SiteId,
    pub to: SiteId,
    pub from_pos: crate::layout::Position,
    pub to_pos: crate::layout::Position,
    pub aod_id: u32,
    pub row: u32,
    pub col: u32,
    pub dual_id: u64,
    pub distance_um: f64,
}
