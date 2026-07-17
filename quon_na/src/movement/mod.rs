//! Flat (non-zoned) AOD-constrained movement planner (issue #106).
//!
//! # Attribution (B9)
//!
//! This module implements a **Quon** pipeline:
//! - interaction-pair bank destinations,
//! - both-atom dual orientations onto a free pair,
//! - B7 second packing pass that serializes dual legs,
//! - B8/B13 multi-layer reuse / eviction / partial-overlap reclaim.
//!
//! [Enola] Sec. 5 is cited **only** for: (1) three per-axis move conflict types
//! and (2) the greedy longest-first maximal independent-set *idea* (sortIS
//! spirit — not KaMIS). Enola Sec. 5 duals are one-atom "move either endpoint"
//! candidates; **this planner does not implement those duals**.
//!
//! # Not in scope
//!
//! - RAP zoned joint placement-routing ([RAP] / issue #107) — use
//!   [`crate::zoned::schedule_zoned`] instead.
//! - Atomique's flat 300 µs stage cost — duration uses shared √-law helpers
//!   [`crate::geometry::movement_duration_us`] / [`crate::geometry::euclidean_um`].
//!
//! # Transfer policy (B6)
//!
//! | Mode | Params | Transfers / moved atom |
//! | --- | --- | --- |
//! | **Quon reuse (default)** | `return_home=false` | SLM→AOD load + AOD→SLM store onto pair (2 xfers); atoms stay; B8/B13 between layers |
//! | **Enola-comparable** | `return_home=true` | load/store to pair, then load/store home each layer (4 xfers) |
//!
//! Only [`types::MovementParams::return_home`] selects the 2- vs 4-transfer policy.
//! Do **not** document the default as Enola Sec. 2/6.1.
//!
//! # Geometry / legality
//!
//! - Bank origin (B14): `x0 = max_placement_x + pair_pitch_um + BANK_ISOLATION_EPS_UM`.
//! - Geometry-gated skip (B10): skip moves only when partners ≤ `rydberg_range_um`
//!   **and** full R1–R3 over all occupied atoms would pass; otherwise relocate gate
//!   atoms onto the pair bank (or fail closed).
//! - R1–R3 (B11): dialect-identical `≤` reject predicates; scope = **all occupied**
//!   atoms (including dense `#104` idles). No spectator-parking workaround — dense
//!   multi-atom grids with idle–idle R2/R3 violations fail [`types::MovementPlanError::EntanglingGeometry`].
//! - Conflict oracle (B12): Enola three types + M3 dest separation
//!   (`|dst_i − dst_j| < min_row_col_separation_um`).
//! - AOD indices (B4): dense row/col overlay from unique site coordinates;
//!   MoveSpec row/col follow the **source** site for the move duration.
//!
//! # Submodule layout
//!
//! | Submodule | Responsibility |
//! | --- | --- |
//! | [`types`] | Params, results, errors, leg/pair/spec payloads |
//! | [`geometry`] | R1–R3 predicates, AOD legality, conflict oracle |
//! | [`bank`] | Interaction-pair bank creation / detection |
//! | [`duals`] | Dual generation, sortIS selection, greedy packing |
//! | [`emit`] | EmitCtx: load→move→store stages, reclaim/evict, layer helpers |
//! | [`plan`] | `plan_aod_movement` orchestrator |

mod bank;
mod duals;
mod emit;
mod geometry;
mod plan;
mod types;

pub use bank::ensure_interaction_pairs;
pub use emit::{atom_moves_to_move_specs, try_transfer_into_occupied};
pub use geometry::{
    check_entangling_geometry, legs_conflict, verify_aod_legality,
    verify_entangling_geometry_predicates,
};
pub use plan::plan_aod_movement;
pub use types::{
    BANK_ISOLATION_EPS_UM, CandidateLeg, InteractionPair, MoveSpec, MovementParams,
    MovementPlanError, MovementPlanResult,
};
