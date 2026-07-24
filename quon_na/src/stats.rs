//! Neutral-atom compiler statistics artifact (`--emit-na-stats`, issue #307).
//!
//! Per-stage wall times, zoned-search diagnostics, and an effective-
//! configuration echo for one [`crate::pipeline::run_from_graph`] /
//! [`crate::pipeline::run_from_module`] run. Mirrors qmap's per-component
//! `Statistics` JSON (`na/zoned/Compiler.hpp`).
//!
//! # Separation from [`crate::report::ResourceReport`]
//!
//! [`crate::report::ResourceReport`] is evidence about the *compiled
//! circuit* (schedule/QEC metrics, ADR-0020's `analytic` evidence kind).
//! [`NaStats`] is telemetry about the *compile itself* — wall-clock time per
//! stage, search diagnostics, and the configuration that actually ran.
//! Mirroring the ADR-0020 analytic/sampled separation discipline (see
//! `docs/adr/0020-qec-reports-remain-separate.md`), these stay two distinct
//! artifacts (`--emit-resource-report` vs. `--emit-na-stats`); [`NaStats`] is
//! never merged into [`crate::report::ResourceReport`] and vice versa.
//!
//! # Schema evolution
//!
//! Every field is `#[serde(default)]` (scalar) or `Option` with
//! `skip_serializing_if = "Option::is_none"` (stage/mode-conditional
//! fields), so older `NaStats` JSON keeps deserializing as new fields are
//! added — the same tolerant-evolution discipline
//! [`crate::report::ResourceReport`] uses. [`NA_STATS_SCHEMA_VERSION`] only
//! needs bumping for a breaking (non-additive) shape change.

use serde::{Deserialize, Serialize};

use crate::pipeline::{NaBackendKind, NaObjective};
use crate::placement::PlacementStrategy;
use crate::zoned::AgnosticPlacerMechanism;
use crate::zoned::PlacerMode;

/// Wire-format label for [`NaStats`] (parallels
/// [`crate::report::RESOURCE_REPORT_EVIDENCE_KIND`]).
pub const NA_STATS_KIND: &str = "na_compiler_stats";

/// Bump only for a breaking (non-additive) shape change — additive fields
/// don't need a bump since every field tolerates `#[serde(default)]`.
pub const NA_STATS_SCHEMA_VERSION: u32 = 1;

fn default_kind() -> String {
    NA_STATS_KIND.to_string()
}

fn default_schema_version() -> u32 {
    NA_STATS_SCHEMA_VERSION
}

fn default_quon_na_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

/// Per-stage wall-clock timings from one pipeline run (microseconds).
///
/// The stage list differs by backend: `zoned_schedule_us` is `Some` only
/// for [`NaBackendKind::Zoned`]; `placement_us` / `movement_us` are `Some`
/// only for [`NaBackendKind::FlatAod`]. `compaction_us` is `None` when
/// compaction was skipped (`--no-na-compact` or an empty schedule).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StageTimingsUs {
    /// MLIR interaction-graph extraction
    /// ([`crate::pipeline::run_from_module`] only). `None` for the raw-graph
    /// debug entry point ([`crate::pipeline::run_from_graph`]), which has no
    /// MLIR to extract from.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extract_us: Option<u64>,
    /// [`crate::schedule_entry::schedule_from_graph`].
    #[serde(default)]
    pub schedule_from_graph_us: u64,
    /// [`crate::entangling_schedule::schedule_entangling_layers`].
    #[serde(default)]
    pub entangling_layers_us: u64,
    /// [`crate::zoned::schedule_zoned`] — zoned backend only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zoned_schedule_us: Option<u64>,
    /// [`crate::placement::place`] — flat-AOD backend only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement_us: Option<u64>,
    /// [`crate::movement::plan_aod_movement`] — flat-AOD backend only.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub movement_us: Option<u64>,
    /// [`crate::compaction::compact_schedule`]. `None` when compaction was
    /// requested but skipped (empty schedule) or not requested at all — see
    /// [`CompactionConfig::applied`] to distinguish those cases.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compaction_us: Option<u64>,
    /// [`crate::report::build_resource_report`] plus the physical
    /// error-budget attach (ADR-0017), when the target carries one.
    #[serde(default)]
    pub resource_report_us: u64,
    /// Total wall time of the timed portion of the pipeline run (sum of the
    /// stages above; excludes work outside `run_from_graph` /
    /// `run_from_module`, e.g. frontend lowering in `quonc`).
    #[serde(default)]
    pub total_us: u64,
}

/// Zoned routing-aware search diagnostics (issue #111 / #307).
///
/// All fields are `None` for the flat-AOD backend, where the aware-search
/// concept does not apply. Under the zoned backend they are always `Some`,
/// including `Some(0)` under [`PlacerMode::RoutingAgnostic`] (the search
/// never runs, so every counter is legitimately zero).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchDiagnostics {
    /// Per-layer gate-assignment calls where the A* search found a full
    /// assignment within budget (issue #297: the search's best find, not
    /// necessarily a proven joint optimum — see
    /// [`crate::zoned::AwareSearchOutcome::Completed`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aware_search_completed_layers: Option<u64>,
    /// Per-layer calls that exhausted the expansion budget and fell back to
    /// the greedy assignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aware_search_budget_exceeded_layers: Option<u64>,
    /// Per-layer calls that exhausted the entire search space (no legal full
    /// assignment exists) and fell back to the greedy assignment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aware_search_no_legal_assignment_layers: Option<u64>,
    /// Sum of best-first search node expansions across every per-layer
    /// gate-assignment call this schedule made
    /// ([`crate::zoned::ZonedScheduleResult::aware_search_node_expansions`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aware_search_node_expansions: Option<u64>,
    /// The expansion budget each per-layer call was allowed
    /// ([`crate::zoned::AwareSearchParams::node_budget`]) — constant across
    /// layers, echoed here so `aware_search_node_expansions` can be read as a
    /// fraction of budget without hardcoding the constant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aware_search_node_budget: Option<u64>,
    /// δ used ([`crate::zoned::AwareSearchParams::deepening_factor`], [RAP]
    /// Eq. (4)) — issue #297.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aware_search_deepening_factor: Option<f64>,
    /// β used ([`crate::zoned::AwareSearchParams::deepening_value`], [RAP]
    /// Eq. (4)) — issue #297.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aware_search_deepening_value: Option<f64>,
    /// [RAP] Sec. V-D pruning window used
    /// ([`crate::zoned::AwareSearchParams::pruning_window`]) — issue #297.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aware_search_pruning_window: Option<u64>,
    /// Which routing-agnostic placement mechanism produced the schedule
    /// (issue #300): [`AgnosticPlacerMechanism::Matching`] (min-weight
    /// bipartite matching, the default for normal-size layers) or
    /// [`AgnosticPlacerMechanism::GreedyFallback`] (very-large layers or
    /// matching's conflict-repair failure). `None` for the flat-AOD backend
    /// and for [`PlacerMode::RoutingAware`] (the agnostic concept does not
    /// apply); under [`PlacerMode::RoutingAgnostic`] it is always `Some`,
    /// matching the `aware_search_*` "always `Some` under zoned" convention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agnostic_placer_mechanism: Option<AgnosticPlacerMechanism>,
}

/// Compaction knobs actually exercised. Requested and applied can diverge:
/// `--no-na-compact` requests skipping it, and an empty schedule skips it
/// even when requested.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactionConfig {
    /// [`crate::pipeline::NaScheduleOptions::compact`] as requested.
    #[serde(default)]
    pub requested: bool,
    /// Whether [`crate::compaction::compact_schedule`] actually ran.
    #[serde(default)]
    pub applied: bool,
    /// [`crate::compaction::CompactionOptions::greedy`] on the call that
    /// ran (`false`/meaningless when `applied` is `false`).
    #[serde(default)]
    pub greedy: bool,
    /// Whether the compaction call carried architecture/legality limits
    /// ([`crate::compaction::CompactionOptions::arch`] /
    /// `legality`). The pipeline's post-movement compaction call always
    /// leaves these unset — static R2/R3 legality is not meaningful once
    /// movement has bound final atom positions — so this is currently
    /// always `false` when `applied` is `true`, echoed rather than assumed
    /// in case that changes.
    #[serde(default)]
    pub legality_checked: bool,
}

/// Effective (as-run, not merely as-requested) pipeline configuration.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveConfig {
    #[serde(default)]
    pub backend: NaBackendKind,
    /// Zoned backend only — [`PlacerMode::RoutingAgnostic`] vs.
    /// [`PlacerMode::RoutingAware`] (the routing-mode knob for that
    /// backend). `None` for flat AOD, which has no placer-mode concept.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placer_mode: Option<PlacerMode>,
    /// Flat-AOD backend only. `None` for zoned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placement_strategy: Option<PlacementStrategy>,
    #[serde(default)]
    pub compaction: CompactionConfig,
    /// Placement/routing objective (issue #309): `Time` (default) or
    /// `ErrorBudget`. Mirrors the `--na-objective` CLI knob.
    #[serde(default)]
    pub objective: NaObjective,
}

/// Tool/target identifiers so a stats file is self-describing without a
/// companion command line (issue #307 / qmap `Statistics` parity).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionInfo {
    /// `quon_na` crate version (`CARGO_PKG_VERSION`) — the pipeline library
    /// that produced every timed stage. `quonc` overlays its own binary
    /// version separately (`quonc/src/main.rs`); this field intentionally
    /// stays library-scoped so `NaStats` built by non-`quonc` callers
    /// (tests, other tools embedding `quon_na`) is still meaningful.
    #[serde(default = "default_quon_na_version")]
    pub quon_na_version: String,
    /// Loaded backend target descriptor id (e.g.
    /// `generic_reconfigurable_neutral_atom_v0`). `quon_na::pipeline` does
    /// not know its own id ([`backend::NeutralAtomTarget`] carries no id
    /// field — only the wrapping `BackendTarget` does), so callers that
    /// have the id (`quonc`) fill this in; `None` otherwise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_id: Option<String>,
}

impl Default for VersionInfo {
    fn default() -> Self {
        Self {
            quon_na_version: default_quon_na_version(),
            target_id: None,
        }
    }
}

/// Per-compile neutral-atom compiler statistics (issue #307).
///
/// A genuinely separate artifact from [`crate::report::ResourceReport`] —
/// see module docs. `--emit-na-stats` in `quonc` is the only writer;
/// nothing in the pipeline reads this back.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NaStats {
    /// Always [`NA_STATS_KIND`] — labels the artifact type for machine
    /// readers (parallels `ResourceReport::evidence_kind`).
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    #[serde(default)]
    pub version: VersionInfo,
    #[serde(default)]
    pub config: EffectiveConfig,
    #[serde(default)]
    pub stage_timings_us: StageTimingsUs,
    #[serde(default)]
    pub search: SearchDiagnostics,
}

/// Serialize [`NaStats`] to pretty JSON (mirrors
/// [`crate::report::resource_report_to_json`]).
pub fn na_stats_to_json(stats: &NaStats) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(stats)
}
