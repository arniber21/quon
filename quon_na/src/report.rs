//! Neutral-atom resource reports: schedule aggregation, QEC sizing, and emitters.
//!
//! Field names align with TUM RAP Table I / Enola headline metrics. See
//! `docs/neutral_atom/architecture_model.md` §11.
//!
//! Physical error-budget contributions (`error_budget`) use the target's
//! optional [`NeutralAtomErrorModel`] (ADR-0017): `rate × schedule count` only,
//! never logical error rates or `1 - fidelity`.
//!
//! End-to-end fidelity estimates (`gate_fidelity_product` / `estimated_fidelity`,
//! issue #305) use the target's [`NeutralAtomFidelity`] instead: Enola Eq. (1),
//! product of per-action fidelities times per-atom idle decay. Both estimates
//! are **analytic** (ADR-0020) and distinct from `error_budget` — see
//! [`ResourceReport::with_fidelity_estimate`].

use backend::{BackendError, NeutralAtomErrorModel, NeutralAtomFidelity};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::qec::{CodeBlock, CodeFamily, QecError, atoms_per_logical};
use crate::schedule::{LocalGateKind, NeutralAtomAction, ScheduleLayer};
use crate::zoned::AgnosticPlacerMechanism;

#[cfg(feature = "flux")]
use flux_rs::attrs::*;

/// Dominant schedule cost category for a [`ResourceReport`].
///
/// Classification uses max of rydberg stages / rearrangement µs / transfer µs /
/// measurement rounds (ties → [`Mixed`](BottleneckKind::Mixed); all-zero →
/// [`None`](BottleneckKind::None)). See architecture_model.md §11.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BottleneckKind {
    /// Default / empty schedule / all-zero time components.
    #[default]
    None,
    Rydberg,
    Rearrangement,
    Transfer,
    Measurement,
    /// Two or more categories tie for the maximum score.
    Mixed,
}

impl BottleneckKind {
    /// Snake_case wire / Markdown cell text matching JSON serde.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Rydberg => "rydberg",
            Self::Rearrangement => "rearrangement",
            Self::Transfer => "transfer",
            Self::Measurement => "measurement",
            Self::Mixed => "mixed",
        }
    }
}

/// Whether a schedule was produced by the exact SMT solver or a heuristic
/// (issue #302). Stored on [`ResourceReport`] so every emitted report
/// labels the schedule provenance.
///
/// Defined here (not in the `solver`-gated `exact` module) so the field is
/// available without linking z3 — the default build reports `Heuristic`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleOptimality {
    /// Exact SMT solver produced the schedule (proven optimal within the
    /// solver's time budget).
    #[default]
    Exact,
    /// Heuristic placer / scheduler produced the schedule (either the
    /// default path or a fallback after an exact-solver timeout).
    Heuristic,
}

impl ScheduleOptimality {
    /// Wire label for the resource report Markdown / JSON.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Exact => "exact",
            Self::Heuristic => "heuristic",
        }
    }
}

/// Temporal atom-pressure metrics for a neutral-atom schedule (issue #282).
///
/// These make peak-atom pressure and qubit-lifecycle reuse visible so a
/// mid-circuit-measurement / reset / reuse workload (e.g. QEC ancilla reused
/// across rounds) can be compared against a no-reuse variant.
///
/// # Allocation model
///
/// An atom is "allocated" from the first cycle it appears in any action and
/// stays allocated for the rest of the schedule (there is no free event within
/// a schedule — a physical trap holds its atom). Qubit **reuse** shows up as
/// *fewer distinct atom ids*: a schedule that measures→resets→reuses the same
/// ancilla across rounds allocates fewer atoms than a no-reuse variant that
/// consumes a fresh ancilla each round. `allocated_atoms_series[c]` is the
/// count of distinct atoms seen in cycles `0..=c` (monotonic non-decreasing);
/// `peak_atoms` is its maximum (the last value).
///
/// `measurement_count` / `reset_count` are **per-op** tallies (distinct from
/// [`ResourceReport::measurement_rounds`] / `reset_rounds`, which count layers).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemporalAtomMetrics {
    /// Maximum number of simultaneously allocated atoms over the schedule.
    pub peak_atoms: u64,
    /// Distinct allocated-atom count at the end of each cycle (monotonic).
    pub allocated_atoms_series: Vec<u64>,
    /// Total `Measure` actions (per-op, not per-layer rounds).
    pub measurement_count: u64,
    /// Total `Reset` actions (per-op, not per-layer rounds).
    pub reset_count: u64,
    /// Total `Reuse` actions (explicit ancilla reclaim events).
    pub reuse_count: u64,
    /// Distinct atoms that were reclaimed by at least one `Reuse` event.
    pub reused_ancilla_count: u64,
}

impl TemporalAtomMetrics {
    /// Aggregate temporal atom metrics from schedule layers.
    pub fn from_layers(layers: &[ScheduleLayer]) -> Self {
        use crate::layout::AtomId;
        use std::collections::BTreeSet;

        let mut seen: BTreeSet<AtomId> = BTreeSet::new();
        let mut reused: BTreeSet<AtomId> = BTreeSet::new();
        let mut series = Vec::with_capacity(layers.len());
        let mut measurement_count = 0u64;
        let mut reset_count = 0u64;
        let mut reuse_count = 0u64;

        for layer in layers {
            for action in &layer.actions {
                accumulate_action_atoms(action, &mut seen);
                match action {
                    NeutralAtomAction::Measure { .. } => measurement_count += 1,
                    NeutralAtomAction::Reset { .. } => reset_count += 1,
                    NeutralAtomAction::Reuse { atom, .. } => {
                        reuse_count += 1;
                        reused.insert(*atom);
                    }
                    _ => {}
                }
            }
            series.push(seen.len() as u64);
        }

        let peak_atoms = series.iter().copied().max().unwrap_or(0);
        Self {
            peak_atoms,
            allocated_atoms_series: series,
            measurement_count,
            reset_count,
            reuse_count,
            reused_ancilla_count: reused.len() as u64,
        }
    }
}

/// Insert every atom id an action touches into `seen`.
fn accumulate_action_atoms(
    action: &NeutralAtomAction,
    seen: &mut std::collections::BTreeSet<crate::layout::AtomId>,
) {
    match action {
        NeutralAtomAction::Move(group) => {
            for m in &group.moves {
                seen.insert(m.atom);
            }
        }
        NeutralAtomAction::Transfer(t) => {
            seen.insert(t.atom);
        }
        NeutralAtomAction::Entangle2 { atoms, .. } => {
            seen.insert(atoms[0]);
            seen.insert(atoms[1]);
        }
        NeutralAtomAction::EntangleN { atoms, .. } => {
            for &a in atoms {
                seen.insert(a);
            }
        }
        NeutralAtomAction::LocalGate { atom, .. }
        | NeutralAtomAction::Measure { atom, .. }
        | NeutralAtomAction::Reset { atom, .. }
        | NeutralAtomAction::Reuse { atom, .. } => {
            seen.insert(*atom);
        }
        // No discrete atom list (issue #298): a global raster neither
        // allocates nor frees an atom, so it doesn't change the
        // allocated-atom census tracked here.
        NeutralAtomAction::GlobalRy { .. } => {}
        NeutralAtomAction::Wait { .. } => {}
    }
}

/// Wire value for [`ResourceReport::evidence_kind`] (ADR-0020).
pub const RESOURCE_REPORT_EVIDENCE_KIND: &str = "analytic";

/// Short ADR-0020 disclaimer embedded in JSON for machine readers.
pub const RESOURCE_REPORT_EVIDENCE_DISCLAIMER: &str = "Compiler analytic metrics only — not fused with Python/Sinter sampled CSV; neither artifact is a threshold claim (ADR-0020).";

fn default_evidence_kind() -> String {
    RESOURCE_REPORT_EVIDENCE_KIND.to_string()
}

fn default_evidence_disclaimer() -> String {
    RESOURCE_REPORT_EVIDENCE_DISCLAIMER.to_string()
}

/// Aggregated resource metrics for a neutral-atom schedule.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResourceReport {
    /// Always [`RESOURCE_REPORT_EVIDENCE_KIND`] — labels analytic vs Sinter CSV (ADR-0020).
    #[serde(default = "default_evidence_kind")]
    pub evidence_kind: String,
    /// Human/machine disclaimer: analytic ≠ sampled; not a threshold claim.
    #[serde(default = "default_evidence_disclaimer")]
    pub evidence_disclaimer: String,

    pub rydberg_stages: u64,
    pub rearrangement_steps: u64,
    pub rearrangement_time_us: u64,
    pub trap_transfers: u64,
    pub transfer_time_us: u64,
    pub entangle2_count: u64,
    pub entangle_n_count: u64,
    pub measurement_rounds: u64,
    pub reset_rounds: u64,
    pub wait_time_us: u64,
    pub total_time_us: u64,

    /// Logical qubit count (0 until a sizing builder is applied).
    #[serde(default)]
    pub logical_qubits: u64,
    /// Physical atom count (0 until a sizing builder is applied).
    #[serde(default)]
    pub physical_atoms: u64,
    /// Atoms per logical when a single code family is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atoms_per_logical: Option<u64>,
    /// Stable code-family label when a single family is set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code_family: Option<String>,
    /// Code distance when a single homogeneous QEC family is set (#248).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distance: Option<u64>,
    /// Number of syndrome / memory rounds from the QEC workload (#248).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory_rounds: Option<u64>,

    /// Logical T gate count (magic-state-consuming, issue #283).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub t_count: Option<u64>,
    /// Logical T† gate count (issue #283).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tdag_count: Option<u64>,
    /// Logical CCZ gate count (issue #283).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ccz_count: Option<u64>,
    /// Total magic-state demand (T + Tdag + CCZ, issue #283).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub magic_state_demand: Option<u64>,

    /// Number of schedule layers (`layers.len()`).
    #[serde(default)]
    pub estimated_cycles: u64,
    /// Max of rydberg / rearrangement / transfer / measurement scores.
    #[serde(default)]
    pub bottleneck: BottleneckKind,

    /// Zoned NA backend only (issue #111 review finding): number of
    /// per-layer [`PlacerMode::RoutingAware`](crate::zoned::PlacerMode)
    /// gate-assignment calls whose best-first search found a full legal
    /// assignment within budget (the true joint-optimal for that layer).
    /// `None` for non-zoned compiles; `Some(0)` under
    /// `PlacerMode::RoutingAgnostic` (the concept doesn't apply there).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aware_search_completed_layers: Option<u64>,
    /// Companion to `aware_search_completed_layers`: number of layers where
    /// the aware search instead exhausted its budget or search space and
    /// silently fell back to the routing-agnostic greedy assignment. A
    /// nonzero value here on a `routing-aware` compile means any
    /// aware == agnostic schedule match is **not** evidence of "no routing
    /// contention" — it may just be the fallback reproducing the greedy
    /// schedule. See `docs/neutral_atom/rap_table_i_methodology.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aware_search_fell_back_layers: Option<u64>,

    /// Zoned NA backend only (issue #300): which routing-agnostic placement
    /// mechanism produced the schedule —
    /// [`AgnosticPlacerMechanism::Matching`] (the #300 min-weight bipartite
    /// matching default for normal-size layers) or
    /// [`AgnosticPlacerMechanism::GreedyFallback`] (very-large layers above the
    /// [`crate::zoned::MATCHING_FALLBACK_GATE_PAIR_PRODUCT`] threshold, or
    /// matching's conflict-repair failure). `None` for non-zoned compiles and
    /// for `routing-aware` (the agnostic concept does not apply); under
    /// `routing-agnostic` it is always `Some`. Mirrors the
    /// `aware_search_completed_layers` schema-evolution pattern.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agnostic_placer_mechanism: Option<AgnosticPlacerMechanism>,

    /// Whether the schedule was produced by the exact SMT solver or a
    /// heuristic (issue #302). `None` for reports built without calling
    /// [`ResourceReport::with_schedule_optimality`] (library use, most unit
    /// tests); `Some` on every production report from the pipeline. When
    /// the exact solver timed out and fell back, this is `Heuristic` — the
    /// fallback is never silent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule_optimality: Option<ScheduleOptimality>,

    /// Analytic physical error-budget contributions (rate × schedule counts).
    /// Never logical error rates or thresholds (ADR-0017 / ADR-0020).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_budget: Option<ErrorBudgetContributions>,

    /// Temporal atom-pressure metrics: peak atoms, allocated-over-time series,
    /// per-op measurement/reset counts, and reused ancilla count (issue #282).
    #[serde(default)]
    pub temporal_atom_metrics: TemporalAtomMetrics,

    // --- Single-qubit gate counts / time (issue #298) ---
    //
    // `Option` following the `aware_search_completed_layers` schema-evolution
    // pattern: `None` only for a `ResourceReport` value that predates #298
    // (or was hand-built without calling `from_layers`); `from_layers` always
    // sets `Some`, `Some(0)` included, since single-qubit gate accounting
    // applies to every NA schedule. Per-gate timing uses the target's
    // `timing.single_qubit_us` (0.625 us in `generic_rna_v0.json`) — this is
    // the same per-gate model documented (and deliberately *not* qmap's
    // ~52 us global-raster batching model) in
    // `docs/neutral_atom/architecture_model.md` §8.6.
    /// Count of local `h` gate actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_h_count: Option<u64>,
    /// Count of local `rz(theta)` gate actions. Includes the Hahn-echo
    /// `Rz(pi)`/`Rz(-pi)` refocusing pulses `GlobalRy` bystander isolation
    /// inserts (issue #298 review finding) alongside "real" `rz` gates from
    /// the source program — both are genuine `LocalGate { gate: Rz(..) }`
    /// schedule actions, so this count does not distinguish them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_rz_count: Option<u64>,
    /// Count of local `u3(theta, phi, lambda)` escape-hatch gate actions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_u3_count: Option<u64>,
    /// Cumulative local single-qubit gate op time (us): sum of `LocalGate`
    /// (`h`/`rz`/`u3`) durations, independent of layer wall-clock overlap —
    /// same accounting convention as `rearrangement_time_us`/`transfer_time_us`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local_gate_time_us: Option<u64>,
    /// Count of global `ry(theta)` whole-plane raster actions (one action =
    /// one raster covering every trapped atom, not per-atom). A raster with
    /// bystander atoms present (issue #298 review finding) is split into two
    /// `theta / 2` half-pulses, so a single source-program `ry` gate that
    /// isn't alone in the trap array contributes 2 to this count, not 1 —
    /// see [`crate::schedule::NeutralAtomAction::GlobalRy`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_ry_count: Option<u64>,
    /// Cumulative global `ry` raster op time (us).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub global_ry_time_us: Option<u64>,

    // --- End-to-end fidelity estimate (Enola Eq. (1), issue #305) ---
    //
    // `None` until [`ResourceReport::with_fidelity_estimate`] is called with a
    // target's [`NeutralAtomFidelity`] — the same "None until overlaid"
    // convention `error_budget` uses (ADR-0017 §11.4), not a per-field schema
    // migration marker. In production, `quon_na::pipeline::finish_pipeline`
    // and `quon_na::qec_schedule::schedule_expanded` always call the overlay
    // when a `NeutralAtomTarget` is available (its `fidelity` field is
    // mandatory, unlike the optional `error_model`), so both fields are
    // `Some` on every `--emit-resource-report` output; only reports built
    // directly from `ResourceReport::from_layers` / `build_resource_report`
    // without that overlay (library use, most unit tests) leave them unset.
    //
    // **Analytic estimate, not a logical error rate or threshold claim**
    // (ADR-0020) — same discipline as `error_budget`, kept as a visibly
    // separate field pair so the two analytic estimates are never conflated.
    /// Raw product of per-action fidelities over the compiled schedule, with
    /// **no** idle decay: `fidelity.cz ^ (entangling actions) ×
    /// fidelity.single_qubit ^ (local + global-ry actions) ×
    /// fidelity.atom_transfer ^ (trap transfers)`.
    ///
    /// "Actions" here are real compiled-schedule
    /// [`crate::schedule::NeutralAtomAction`]s, not source-program gate
    /// counts: a `GlobalRy` raster with bystander atoms present splits into
    /// two half-pulses plus a `Rz(pi)`/`Rz(-pi)` echo pair per bystander
    /// (issue #298's `push_global_ry_with_refocus`), and every one of those
    /// echo pulses is a genuine `LocalGate` action already counted in
    /// `local_rz_count` — so this product counts them for free by reusing
    /// the schedule-scan aggregates (`entangle2_count` + `entangle_n_count`,
    /// `local_h_count` + `local_rz_count` + `local_u3_count` +
    /// `global_ry_count`, `trap_transfers`) rather than re-deriving a
    /// separate "logical gate count".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gate_fidelity_product: Option<f64>,
    /// `gate_fidelity_product` times per-atom idle decay:
    /// `∏_atoms max(0, 1 - t_idle(atom) / fidelity.coherence_time_us)` —
    /// architecture_model.md §9/§11's **linear** approximation of [Enola]
    /// Eq. (1)'s decoherence factor (not `exp(-t_idle/T)`). `t_idle(atom)`
    /// is `total_time_us` minus the sum of layer max-durations
    /// (`simultaneous_layer_time`) over layers in which the atom appears in
    /// any action — the same per-layer wall-clock-proxy granularity
    /// `total_time_us` itself uses, not a finer intra-layer accounting.
    /// Atoms that never appear in any schedule action contribute no idle
    /// term (out of scope for a per-action fidelity model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_fidelity: Option<f64>,
}

impl Default for ResourceReport {
    fn default() -> Self {
        Self {
            evidence_kind: default_evidence_kind(),
            evidence_disclaimer: default_evidence_disclaimer(),
            rydberg_stages: 0,
            rearrangement_steps: 0,
            rearrangement_time_us: 0,
            trap_transfers: 0,
            transfer_time_us: 0,
            entangle2_count: 0,
            entangle_n_count: 0,
            measurement_rounds: 0,
            reset_rounds: 0,
            wait_time_us: 0,
            total_time_us: 0,
            logical_qubits: 0,
            physical_atoms: 0,
            atoms_per_logical: None,
            code_family: None,
            distance: None,
            memory_rounds: None,
            t_count: None,
            tdag_count: None,
            ccz_count: None,
            magic_state_demand: None,
            estimated_cycles: 0,
            bottleneck: BottleneckKind::None,
            aware_search_completed_layers: None,
            aware_search_fell_back_layers: None,
            agnostic_placer_mechanism: None,
            schedule_optimality: None,
            error_budget: None,
            temporal_atom_metrics: TemporalAtomMetrics::default(),
            local_h_count: None,
            local_rz_count: None,
            local_u3_count: None,
            local_gate_time_us: None,
            global_ry_count: None,
            global_ry_time_us: None,
            gate_fidelity_product: None,
            estimated_fidelity: None,
        }
    }
}

/// Per-category physical error-budget contributions: `rate × schedule count`.
///
/// These are **not** logical failure probabilities or threshold estimates.
///
/// Multipliers (schedule aggregates × [`NeutralAtomErrorModel`] rates):
/// - `rydberg` ← `error_model.rydberg × rydberg_stages` (per Rydberg
///   illumination **stage**, not per Entangle2/CZ gate — Enola stage exposure)
/// - `measurement` ← `error_model.measurement × measurement_rounds`
/// - `reset` ← `error_model.reset × reset_rounds`
/// - `movement` ← `error_model.movement × rearrangement_steps`
/// - `transfer` ← `error_model.transfer × trap_transfers`
/// - `idle` ← `error_model.idle_per_us × wait_time_us`
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ErrorBudgetContributions {
    pub rydberg: f64,
    pub measurement: f64,
    pub reset: f64,
    pub movement: f64,
    pub transfer: f64,
    pub idle: f64,
}

/// Collapse binary float dust from `rate × count` products for stable JSON/MD emit.
fn clean_contribution(v: f64) -> f64 {
    if !v.is_finite() || v == 0.0 {
        return if v == 0.0 { 0.0 } else { v };
    }
    // Absolute 15 decimal places keeps idle (~1e-9) while fixing 0.003×3 dust.
    let scale = 1e15;
    (v * scale).round() / scale
}

impl ErrorBudgetContributions {
    /// `rate × count` only — uses report schedule aggregates, never fidelities.
    ///
    /// See struct-level docs for the exact multiplier for each field.
    pub fn from_schedule_and_model(report: &ResourceReport, model: &NeutralAtomErrorModel) -> Self {
        Self {
            rydberg: clean_contribution(model.rydberg * report.rydberg_stages as f64),
            measurement: clean_contribution(model.measurement * report.measurement_rounds as f64),
            reset: clean_contribution(model.reset * report.reset_rounds as f64),
            movement: clean_contribution(model.movement * report.rearrangement_steps as f64),
            transfer: clean_contribution(model.transfer * report.trap_transfers as f64),
            idle: clean_contribution(model.idle_per_us * report.wait_time_us as f64),
        }
    }
}

/// Failures from building a sized [`ResourceReport`].
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ReportError {
    #[error("qec code-block list was empty; pass None for non-QEC sizing")]
    EmptyCodeBlocks,
    /// QEC error-budget reporting was requested but the target has no `error_model`.
    ///
    /// Message aligned with [`BackendError::MissingErrorModel`] (ADR-0017).
    #[error(
        "neutral-atom target is missing error_model required for QEC error reporting \
         (--emit-resource-report) or --emit-qec-experiment; set error_model on the \
         target (do not derive from fidelity)"
    )]
    MissingErrorModel,
    #[error(transparent)]
    Qec(#[from] QecError),
}

/// Simultaneous actions make a layer's elapsed time the maximum action duration.
#[cfg_attr(
    feature = "flux",
    spec(fn(current: u64, next: u64) -> u64{v: current <= v && next <= v && (v == current || v == next)})
)]
pub fn simultaneous_layer_time(current: u64, next: u64) -> u64 {
    if current >= next { current } else { next }
}

fn classify_bottleneck(report: &ResourceReport) -> BottleneckKind {
    let scores = [
        (BottleneckKind::Rydberg, report.rydberg_stages),
        (BottleneckKind::Rearrangement, report.rearrangement_time_us),
        (BottleneckKind::Transfer, report.transfer_time_us),
        (BottleneckKind::Measurement, report.measurement_rounds),
    ];

    let max = scores.iter().map(|(_, s)| *s).fold(0u64, |a, b| a.max(b));
    if max == 0 {
        return BottleneckKind::None;
    }

    let winners: Vec<BottleneckKind> = scores
        .iter()
        .filter(|(_, s)| *s == max)
        .map(|(k, _)| *k)
        .collect();

    match winners.as_slice() {
        [only] => *only,
        _ => BottleneckKind::Mixed,
    }
}

fn code_family_label(family: &CodeFamily) -> &'static str {
    family.as_report_str()
}

fn same_code_family(a: &CodeFamily, b: &CodeFamily) -> bool {
    match (a, b) {
        (
            CodeFamily::SurfaceCodeLike { distance: d1 },
            CodeFamily::SurfaceCodeLike { distance: d2 },
        ) => d1 == d2,
        (
            CodeFamily::RepetitionCodeToy { distance: d1 },
            CodeFamily::RepetitionCodeToy { distance: d2 },
        ) => d1 == d2,
        (
            CodeFamily::HighRateQldpcLike { net_rate: r1 },
            CodeFamily::HighRateQldpcLike { net_rate: r2 },
        ) => r1 == r2,
        (
            CodeFamily::AbstractBlockCode {
                n: n1,
                k: k1,
                d: d1,
            },
            CodeFamily::AbstractBlockCode {
                n: n2,
                k: k2,
                d: d2,
            },
        ) => n1 == n2 && k1 == k2 && d1 == d2,
        _ => false,
    }
}

/// Per-atom idle-decay product for Enola Eq. (1) (issue #305):
/// `∏_atoms max(0, 1 - t_idle(atom) / coherence_time_us)`, the linear
/// approximation architecture_model.md §9/§11 commits to (not
/// `exp(-t_idle/T)`).
///
/// Idle time is tracked at the same per-layer wall-clock-proxy granularity
/// `total_time_us` itself uses (`simultaneous_layer_time`'s
/// layer-max-duration, not per-action durations): an atom is "active" for a
/// layer's full max-duration if it appears in *any* action in that layer
/// (via [`accumulate_action_atoms`]).
///
/// **`GlobalRy` activity (issue #328):** `accumulate_action_atoms` inserts
/// no atoms for `GlobalRy` (it has no discrete atom list), which is correct
/// for the [`TemporalAtomMetrics`] allocation census but wrong for idle
/// decay — a global raster physically hits *every* trapped atom, so every
/// atom in the schedule's atom universe is active during a `GlobalRy` layer.
/// This function builds the atom universe from all atom-addressed actions
/// across the schedule (via [`accumulate_action_atoms`], same as the
/// allocation census) and, for any layer containing a `GlobalRy` action,
/// extends that layer's active set with the full universe. Atoms that appear
/// only via `GlobalRy` (no atom-addressed action anywhere) stay outside the
/// universe and contribute no idle term (out of scope for a per-action
/// fidelity model — nothing here attributes them the full schedule duration
/// as idle).
fn idle_decay_product(layers: &[ScheduleLayer], total_time_us: u64, coherence_time_us: f64) -> f64 {
    use crate::layout::AtomId;
    use std::collections::{BTreeMap, BTreeSet};

    // Build the fidelity atom universe from all atom-addressed actions across
    // the schedule. GlobalRy alone does not contribute atoms here (issue #328:
    // an atom known only through GlobalRy is outside the report's universe).
    let universe: BTreeSet<AtomId> = {
        let mut u = BTreeSet::new();
        for layer in layers {
            for action in &layer.actions {
                accumulate_action_atoms(action, &mut u);
            }
        }
        u
    };

    let mut active_us: BTreeMap<AtomId, u64> = BTreeMap::new();
    for layer in layers {
        let max_duration = layer
            .actions
            .iter()
            .map(NeutralAtomAction::duration_us)
            .fold(0u64, simultaneous_layer_time);

        let mut seen: BTreeSet<AtomId> = BTreeSet::new();
        let mut has_global_ry = false;
        for action in &layer.actions {
            accumulate_action_atoms(action, &mut seen);
            if matches!(action, NeutralAtomAction::GlobalRy { .. }) {
                has_global_ry = true;
            }
        }
        // A GlobalRy raster hits every atom in the universe, so mark them all
        // active for this layer's max-duration (issue #328).
        if has_global_ry {
            seen.extend(universe.iter().copied());
        }
        for atom in seen {
            *active_us.entry(atom).or_insert(0) += max_duration;
        }
    }

    active_us
        .values()
        .map(|&active| {
            let idle_us = total_time_us.saturating_sub(active);
            (1.0 - idle_us as f64 / coherence_time_us).max(0.0)
        })
        .product()
}

impl ResourceReport {
    /// Aggregate schedule metrics from layers.
    ///
    /// Sets `estimated_cycles = layers.len()` and `bottleneck` from scores.
    /// Leaves sizing fields at zero / `None` until a builder overlay is applied.
    pub fn from_layers(layers: &[ScheduleLayer]) -> Self {
        let mut report = ResourceReport::default();
        let mut local_h = 0u64;
        let mut local_rz = 0u64;
        let mut local_u3 = 0u64;
        let mut local_gate_time_us = 0u64;
        let mut global_ry = 0u64;
        let mut global_ry_time_us = 0u64;

        for layer in layers {
            let mut layer_has_rydberg = false;
            let mut layer_has_measurement = false;
            let mut layer_has_reset = false;
            let mut max_duration_us = 0;

            for action in &layer.actions {
                let duration_us = action.duration_us();
                max_duration_us = simultaneous_layer_time(max_duration_us, duration_us);

                match action {
                    NeutralAtomAction::Move(_) => {
                        report.rearrangement_steps += 1;
                        report.rearrangement_time_us += duration_us;
                    }
                    NeutralAtomAction::Transfer(_) => {
                        report.trap_transfers += 1;
                        report.transfer_time_us += duration_us;
                    }
                    NeutralAtomAction::Entangle2 { .. } => {
                        layer_has_rydberg = true;
                        report.entangle2_count += 1;
                    }
                    NeutralAtomAction::EntangleN { .. } => {
                        layer_has_rydberg = true;
                        report.entangle_n_count += 1;
                    }
                    NeutralAtomAction::LocalGate { gate, .. } => {
                        // Locals contribute wall-clock via max layer duration
                        // only (same convention as `Reuse` below); counts and
                        // cumulative op time are tallied separately (#298).
                        match gate {
                            LocalGateKind::H => local_h += 1,
                            LocalGateKind::Rz(_) => local_rz += 1,
                            LocalGateKind::U3 { .. } => local_u3 += 1,
                        }
                        local_gate_time_us += duration_us;
                    }
                    NeutralAtomAction::GlobalRy { .. } => {
                        // Whole-plane raster: wall-clock via max layer
                        // duration only, same as other local ops (#298).
                        global_ry += 1;
                        global_ry_time_us += duration_us;
                    }
                    NeutralAtomAction::Measure { .. } => {
                        layer_has_measurement = true;
                    }
                    NeutralAtomAction::Reset { .. } => {
                        layer_has_reset = true;
                    }
                    NeutralAtomAction::Reuse { .. } => {
                        // Reuse is a bookkeeping reclaim; it contributes wall
                        // clock via the layer max only, and is tallied in
                        // `temporal_atom_metrics` below.
                    }
                    NeutralAtomAction::Wait { .. } => {
                        report.wait_time_us += duration_us;
                    }
                }
            }

            if layer_has_rydberg {
                report.rydberg_stages += 1;
            }
            if layer_has_measurement {
                report.measurement_rounds += 1;
            }
            if layer_has_reset {
                report.reset_rounds += 1;
            }

            report.total_time_us += max_duration_us;
        }

        report.estimated_cycles = layers.len() as u64;
        report.bottleneck = classify_bottleneck(&report);
        report.temporal_atom_metrics = TemporalAtomMetrics::from_layers(layers);
        report.local_h_count = Some(local_h);
        report.local_rz_count = Some(local_rz);
        report.local_u3_count = Some(local_u3);
        report.local_gate_time_us = Some(local_gate_time_us);
        report.global_ry_count = Some(global_ry);
        report.global_ry_time_us = Some(global_ry_time_us);
        report
    }

    /// Overlay sizing from an explicit physical atom count (non-QEC, 1:1).
    pub fn with_physical_atoms(mut self, n: u64) -> Self {
        self.physical_atoms = n;
        self.logical_qubits = n;
        self.atoms_per_logical = None;
        self.code_family = None;
        self.distance = None;
        self.memory_rounds = None;
        self
    }

    /// Overlay logical/physical counts from expanded code blocks.
    pub fn with_code_blocks(mut self, blocks: &[CodeBlock]) -> Result<Self, QecError> {
        let mut logical = 0u64;
        let mut physical = 0u64;

        for block in blocks {
            logical = logical
                .checked_add(block.logical_qubits.len() as u64)
                .ok_or(QecError::AtomCountOverflow)?;
            physical = physical
                .checked_add(block.atoms.len() as u64)
                .ok_or(QecError::AtomCountOverflow)?;
        }

        self.logical_qubits = logical;
        self.physical_atoms = physical;

        // Single shared family → set optional QEC detail rows; mixed → counts only.
        let first_family = blocks.first().map(|b| &b.family);
        let homogeneous = match first_family {
            Some(first) => blocks.iter().all(|b| same_code_family(&b.family, first)),
            None => false,
        };

        if homogeneous {
            if let Some(family) = first_family {
                let per = atoms_per_logical(family)?;
                self.atoms_per_logical = Some(u64::from(per));
                self.code_family = Some(code_family_label(family).to_string());
                self.distance = family.distance().map(u64::from);
            }
        } else {
            self.atoms_per_logical = None;
            self.code_family = None;
            self.distance = None;
        }

        Ok(self)
    }

    /// Overlay analytic physical error-budget contributions (rate × counts).
    pub fn with_error_budget(mut self, model: &NeutralAtomErrorModel) -> Self {
        self.error_budget = Some(ErrorBudgetContributions::from_schedule_and_model(
            &self, model,
        ));
        self
    }

    /// Overlay an analytic end-to-end fidelity estimate (Enola Eq. (1),
    /// issue #305): `gate_fidelity_product` (raw per-action product) and
    /// `estimated_fidelity` (additionally multiplying in per-atom idle
    /// decay). See the field docs above for the exact formulas.
    ///
    /// `layers` must be the same compiled schedule this report's counts came
    /// from (`from_layers`) — the per-action counts already on `self`
    /// (`entangle2_count`, `local_h_count`, …) are sufficient for
    /// `gate_fidelity_product`, but per-atom idle time isn't tracked by any
    /// existing aggregate, so idle decay re-scans `layers` directly.
    pub fn with_fidelity_estimate(
        mut self,
        layers: &[ScheduleLayer],
        fidelity: &NeutralAtomFidelity,
    ) -> Self {
        let entangling_actions = self.entangle2_count + self.entangle_n_count;
        let single_qubit_actions = self.local_h_count.unwrap_or(0)
            + self.local_rz_count.unwrap_or(0)
            + self.local_u3_count.unwrap_or(0)
            + self.global_ry_count.unwrap_or(0);
        let transfer_actions = self.trap_transfers;

        let gate_fidelity_product = fidelity.cz.powf(entangling_actions as f64)
            * fidelity.single_qubit.powf(single_qubit_actions as f64)
            * fidelity.atom_transfer.powf(transfer_actions as f64);

        let idle_decay = idle_decay_product(layers, self.total_time_us, fidelity.coherence_time_us);

        self.gate_fidelity_product = Some(gate_fidelity_product);
        self.estimated_fidelity = Some(gate_fidelity_product * idle_decay);
        self
    }

    /// Overlay zoned-backend routing-aware search diagnostics (issue #111
    /// review finding). `completed_layers` / `fell_back_layers` come from
    /// [`crate::zoned::ZonedScheduleResult`]; see that type's field docs for
    /// exactly what each counts.
    pub fn with_aware_search_status(
        mut self,
        completed_layers: u64,
        fell_back_layers: u64,
    ) -> Self {
        self.aware_search_completed_layers = Some(completed_layers);
        self.aware_search_fell_back_layers = Some(fell_back_layers);
        self
    }

    /// Overlay which routing-agnostic placement mechanism produced the
    /// zoned schedule (issue #300). `mechanism` comes from
    /// [`crate::zoned::ZonedScheduleResult`] (derived there from its
    /// `agnostic_matching_layers` / `agnostic_greedy_fallback_layers`
    /// counters); see [`ResourceReport::agnostic_placer_mechanism`] for what
    /// each variant means. `None` (non-zoned / routing-aware) leaves the field
    /// at its `from_layers` default of `None`.
    pub fn with_agnostic_placer_mechanism(
        mut self,
        mechanism: Option<AgnosticPlacerMechanism>,
    ) -> Self {
        self.agnostic_placer_mechanism = mechanism;
        self
    }

    /// Overlay whether the schedule was produced by the exact SMT solver or
    /// a heuristic (issue #302). `None` leaves the field at its `from_layers`
    /// default of `None` (no optimality claim); `Some(Exact)` labels the
    /// schedule as solver-proven; `Some(Heuristic)` labels it as heuristic
    /// (including the timeout-fallback case).
    pub fn with_schedule_optimality(mut self, optimality: ScheduleOptimality) -> Self {
        self.schedule_optimality = Some(optimality);
        self
    }
}

/// Build a report from layers with optional QEC or physical-atom sizing.
///
/// Preference: `from_layers` → QEC blocks (if `Some`) → else physical hint → else zeros.
/// `qec: Some(&[])` returns [`ReportError::EmptyCodeBlocks`].
pub fn build_resource_report(
    layers: &[ScheduleLayer],
    qec: Option<&[CodeBlock]>,
    physical_atoms_hint: Option<u64>,
) -> Result<ResourceReport, ReportError> {
    let mut report = ResourceReport::from_layers(layers);

    match qec {
        Some([]) => return Err(ReportError::EmptyCodeBlocks),
        Some(blocks) => {
            report = report.with_code_blocks(blocks)?;
        }
        None => {
            if let Some(n) = physical_atoms_hint {
                report = report.with_physical_atoms(n);
            }
        }
    }

    Ok(report)
}

/// Attach QEC analytic error-budget contributions when requested.
///
/// Hard-fails with [`ReportError::MissingErrorModel`] if `error_model` is
/// absent. Used by `--emit-resource-report` (always attaches budget) and by
/// `--emit-qec-experiment` (#255) when that path requests rates (ADR-0017).
/// Never invents defaults or converts from fidelity.
pub fn attach_qec_error_budget(
    report: ResourceReport,
    error_model: Option<&NeutralAtomErrorModel>,
) -> Result<ResourceReport, ReportError> {
    let model = error_model.ok_or(ReportError::MissingErrorModel)?;
    Ok(report.with_error_budget(model))
}

/// Resolve a target's error model for QEC error artifacts.
///
/// Maps [`BackendError::MissingErrorModel`] only — call sites match that
/// variant explicitly so other [`BackendError`]s are never laundered into
/// [`ReportError::MissingErrorModel`]. Prefer this (or
/// [`backend::NeutralAtomTarget::require_error_model`]) from CLI paths that
/// request QEC error reporting (`--emit-resource-report`) or
/// `--emit-qec-experiment`.
pub fn require_target_error_model(
    target: &backend::NeutralAtomTarget,
) -> Result<&NeutralAtomErrorModel, ReportError> {
    match target.require_error_model() {
        Ok(model) => Ok(model),
        Err(BackendError::MissingErrorModel) => Err(ReportError::MissingErrorModel),
        Err(other) => unreachable!(
            "NeutralAtomTarget::require_error_model only returns MissingErrorModel; got {other}"
        ),
    }
}

/// Pretty-printed JSON for a resource report (stable struct field order).
pub fn resource_report_to_json(report: &ResourceReport) -> Result<String, serde_json::Error> {
    serde_json::to_string_pretty(report)
}

/// Format an error-budget contribution for Markdown tables.
///
/// Rule (architecture_model.md §11.1): use lowercase scientific notation when
/// `|v|` is nonzero and `< 1e-4` (e.g. `8e-9`); otherwise a short decimal
/// (`0.004`, `0.0005`, `0.009`) — never binary dust like `0.009000000000000001`.
fn format_contribution(v: f64) -> String {
    let v = clean_contribution(v);
    let abs = v.abs();
    if v != 0.0 && abs < 1e-4 {
        let exp = abs.log10().floor() as i32;
        let mant = v / 10f64.powi(exp);
        let mant = clean_contribution(mant);
        if (mant - mant.round()).abs() < 1e-12 {
            format!("{}e{exp}", mant.round() as i64)
        } else {
            format!("{mant}e{exp}")
        }
    } else if v == 0.0 {
        "0".to_string()
    } else {
        // Trim trailing zeros from a fixed-precision decimal.
        let mut s = format!("{v:.12}");
        if s.contains('.') {
            while s.ends_with('0') {
                s.pop();
            }
            if s.ends_with('.') {
                s.pop();
            }
        }
        s
    }
}

/// Deterministic Markdown matching architecture_model.md §11.
///
/// Non-QEC reports omit atoms-per-logical and code-family rows (never `N/A`).
pub fn resource_report_to_markdown(report: &ResourceReport) -> String {
    let mut out = String::new();
    out.push_str("# Neutral-atom analytic resource report\n\n");
    out.push_str("## Qubit resources\n");
    out.push_str("| Metric | Value |\n");
    out.push_str("| --- | ---: |\n");
    out.push_str(&format!("| Logical qubits | {} |\n", report.logical_qubits));
    out.push_str(&format!("| Physical atoms | {} |\n", report.physical_atoms));
    if let Some(apl) = report.atoms_per_logical {
        out.push_str(&format!("| Atoms per logical | {apl} |\n"));
    }
    if let Some(ref family) = report.code_family {
        out.push_str(&format!("| Code family | {family} |\n"));
    }
    if let Some(distance) = report.distance {
        out.push_str(&format!("| Distance | {distance} |\n"));
    }
    if let Some(rounds) = report.memory_rounds {
        out.push_str(&format!("| Memory rounds | {rounds} |\n"));
    }
    out.push('\n');

    out.push_str("## Schedule metrics\n");
    out.push_str("| Metric | Value |\n");
    out.push_str("| --- | ---: |\n");
    out.push_str(&format!(
        "| Estimated cycles | {} |\n",
        report.estimated_cycles
    ));
    out.push_str(&format!(
        "| Bottleneck | {} |\n",
        report.bottleneck.as_str()
    ));
    out.push_str(&format!("| Rydberg stages | {} |\n", report.rydberg_stages));
    out.push_str(&format!(
        "| Rearrangement steps | {} |\n",
        report.rearrangement_steps
    ));
    out.push_str(&format!(
        "| Rearrangement time (µs) | {} |\n",
        report.rearrangement_time_us
    ));
    out.push_str(&format!("| Trap transfers | {} |\n", report.trap_transfers));
    out.push_str(&format!(
        "| Transfer time (µs) | {} |\n",
        report.transfer_time_us
    ));
    out.push_str(&format!(
        "| Entangle2 count | {} |\n",
        report.entangle2_count
    ));
    out.push_str(&format!(
        "| EntangleN count | {} |\n",
        report.entangle_n_count
    ));
    out.push_str(&format!(
        "| Measurement rounds | {} |\n",
        report.measurement_rounds
    ));
    out.push_str(&format!("| Reset rounds | {} |\n", report.reset_rounds));
    out.push_str(&format!("| Wait time (µs) | {} |\n", report.wait_time_us));
    out.push_str(&format!("| Total time (µs) | {} |\n", report.total_time_us));
    if let Some(completed) = report.aware_search_completed_layers {
        let fell_back = report.aware_search_fell_back_layers.unwrap_or(0);
        out.push_str(&format!(
            "| Routing-aware search completed layers | {completed} |\n"
        ));
        out.push_str(&format!(
            "| Routing-aware search fell back to greedy (layers) | {fell_back} |\n"
        ));
    }
    if let Some(mechanism) = report.agnostic_placer_mechanism {
        out.push_str(&format!(
            "| Routing-agnostic placement mechanism | {} |\n",
            mechanism.as_str()
        ));
    }
    if let Some(optimality) = report.schedule_optimality {
        out.push_str(&format!(
            "| Schedule optimality | {} |\n",
            optimality.as_str()
        ));
    }
    out.push('\n');

    let temporal = &report.temporal_atom_metrics;
    out.push_str("## Atom pressure & reuse\n");
    out.push_str("| Metric | Value |\n");
    out.push_str("| --- | ---: |\n");
    out.push_str(&format!("| Peak atoms | {} |\n", temporal.peak_atoms));
    out.push_str(&format!(
        "| Measurement count (ops) | {} |\n",
        temporal.measurement_count
    ));
    out.push_str(&format!(
        "| Reset count (ops) | {} |\n",
        temporal.reset_count
    ));
    out.push_str(&format!(
        "| Reuse count (ops) | {} |\n",
        temporal.reuse_count
    ));
    out.push_str(&format!(
        "| Reused ancilla count | {} |\n",
        temporal.reused_ancilla_count
    ));
    out.push('\n');

    if let Some(budget) = &report.error_budget {
        out.push_str("## Physical error budget\n");
        out.push_str("| Category | Contribution (rate × count) |\n");
        out.push_str("| --- | ---: |\n");
        out.push_str(&format!(
            "| Rydberg | {} |\n",
            format_contribution(budget.rydberg)
        ));
        out.push_str(&format!(
            "| Measurement | {} |\n",
            format_contribution(budget.measurement)
        ));
        out.push_str(&format!(
            "| Reset | {} |\n",
            format_contribution(budget.reset)
        ));
        out.push_str(&format!(
            "| Movement | {} |\n",
            format_contribution(budget.movement)
        ));
        out.push_str(&format!(
            "| Transfer | {} |\n",
            format_contribution(budget.transfer)
        ));
        out.push_str(&format!(
            "| Idle | {} |\n",
            format_contribution(budget.idle)
        ));
        out.push('\n');
    }

    if let Some(gate_fidelity_product) = report.gate_fidelity_product {
        out.push_str("## Fidelity estimate (Enola Eq. (1))\n");
        out.push_str("| Metric | Value |\n");
        out.push_str("| --- | ---: |\n");
        out.push_str(&format!(
            "| Gate fidelity product | {} |\n",
            format_contribution(gate_fidelity_product)
        ));
        if let Some(estimated_fidelity) = report.estimated_fidelity {
            out.push_str(&format!(
                "| Estimated fidelity (with idle decay) | {} |\n",
                format_contribution(estimated_fidelity)
            ));
        }
        out.push('\n');
    }

    out.push_str("## Notes\n");
    out.push_str(
        "- Compiler analytic metrics only — not fused with Python/Sinter sampled CSV; neither artifact is a threshold claim (ADR-0020).\n",
    );
    out.push_str("- Field names align with TUM RAP Table I / Enola headline metrics.\n");
    out.push_str(
        "- `estimated_cycles` is `layers.len()`; `bottleneck` is the max of rydberg stages / rearrangement time / transfer time / measurement rounds (ties → mixed; all-zero → none).\n",
    );
    out.push_str("- Non-QEC reports omit atoms-per-logical and code-family rows.\n");
    out.push_str(
        "- `peak_atoms` counts distinct simultaneously-allocated atoms; qubit reuse (measure → reset → reuse of the same ancilla) lowers it versus a fresh-ancilla-per-round variant. `measurement_count` / `reset_count` are per-op tallies, distinct from the per-layer `measurement_rounds` / `reset_rounds`.\n",
    );
    if report.aware_search_fell_back_layers.is_some_and(|n| n > 0) {
        out.push_str(
            "- Routing-aware search fell back to the greedy assignment on at least one layer \
             (budget exhaustion or no legal full assignment) — a byte-identical or \
             near-identical routing-aware/agnostic schedule here is not evidence of \
             \"no routing contention\"; see `docs/neutral_atom/rap_table_i_methodology.md`.\n",
        );
    }
    if report.error_budget.is_some() {
        out.push_str(
            "- Physical error budget lines are analytic schedule-count × rate contributions only — not sampled logical failure rates (Sinter) or threshold claims.\n",
        );
    }
    if report.gate_fidelity_product.is_some() {
        out.push_str(
            "- Fidelity estimate is an analytic Enola Eq. (1) product over the compiled schedule \
             (`fidelity.cz` for entangling actions, `fidelity.single_qubit` for local/global-ry \
             actions including Hahn-echo bystander pulses, `fidelity.atom_transfer` for trap \
             transfers, `fidelity.coherence_time_us` for idle decay) — not a logical error rate, \
             not sampled (Sinter), and not a threshold claim; distinct from the analytic physical \
             error budget (ADR-0017, `error_budget`, when present) — a `rate × schedule-count` \
             sum, not a fidelity product.\n",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use backend::{BackendError, NeutralAtomErrorModel, NeutralAtomFidelity};
    use serde_json::json;

    use super::*;
    use crate::graph::LogicalQubitId;
    use crate::layout::{AodTrapRef, AtomId, SiteId};
    use crate::qec::{CodeBlockId, NetRate, code_blocks_from_expanded};
    use crate::schedule::{
        AtomMove, MeasurementBasis, MovementGroup, NeutralAtomAction, ScheduleLayer,
        TransferDirection, TrapTransfer,
    };
    use quon_qec::{LogicalBasis, SourceFamily, WorkloadBuilder, expand_workload};

    fn atom(id: u32) -> AtomId {
        AtomId(id)
    }

    fn site(id: u32) -> SiteId {
        SiteId(id)
    }

    fn aod() -> AodTrapRef {
        AodTrapRef {
            aod_id: 0,
            row: 1,
            col: 2,
        }
    }
    /// Build report [`CodeBlock`]s from the production expansion IR
    /// (ADR-0015 / ADR-0030): `WorkloadBuilder → expand_workload →
    /// code_blocks_from_expanded`. Replaces the retired `expand_code_block`
    /// toy expander for distance-bearing families (`Repetition` / `Surface`).
    fn repetition_code_blocks(distances: &[u32]) -> Vec<CodeBlock> {
        let mut builder = WorkloadBuilder::new();
        for (i, &distance) in distances.iter().enumerate() {
            builder
                .construct(
                    SourceFamily::Repetition,
                    distance,
                    LogicalBasis::Z,
                    LogicalQubitId(i as u32),
                )
                .expect("construct repetition block");
        }
        let expanded = expand_workload(&builder.finish()).expect("expand workload");
        code_blocks_from_expanded(&expanded)
    }

    /// Build a sizing-only [`CodeBlock`] for a family with no physical round
    /// expansion (qLDPC / abstract) directly from the `quon_qec` sizing
    /// formula (`atoms_per_logical`). This is the report-sizing path for
    /// families without an expansion IR (ADR-0030); there is no toy expander.
    fn sizing_only_block(family: CodeFamily, n_logicals: usize) -> CodeBlock {
        let per = atoms_per_logical(&family).expect("atoms_per_logical");
        let total = usize::try_from(per).expect("atoms per logical fits usize") * n_logicals;
        let logicals: Vec<LogicalQubitId> =
            (0..n_logicals).map(|i| LogicalQubitId(i as u32)).collect();
        CodeBlock {
            id: CodeBlockId(0),
            family,
            logical_qubits: logicals,
            atoms: (0..total).map(|i| AtomId(i as u32)).collect(),
        }
    }

    fn toy_layers() -> Vec<ScheduleLayer> {
        vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![
                    NeutralAtomAction::Move(MovementGroup {
                        duration_us: 10,
                        moves: vec![
                            AtomMove {
                                atom: atom(0),
                                from: site(0),
                                to: site(1),
                            },
                            AtomMove {
                                atom: atom(1),
                                from: site(2),
                                to: site(3),
                            },
                        ],
                    }),
                    NeutralAtomAction::Wait { duration_us: 4 },
                ],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![
                    NeutralAtomAction::Transfer(TrapTransfer {
                        atom: atom(0),
                        direction: TransferDirection::SlmToAod,
                        site: site(1),
                        aod: aod(),
                        duration_us: 6,
                    }),
                    NeutralAtomAction::Entangle2 {
                        atoms: [atom(0), atom(1)],
                        duration_us: 12,
                    },
                ],
            },
            ScheduleLayer {
                cycle: 2,
                actions: vec![
                    NeutralAtomAction::EntangleN {
                        atoms: vec![atom(0), atom(1), atom(2)],
                        duration_us: 8,
                    },
                    NeutralAtomAction::Measure {
                        atom: atom(0),
                        basis: MeasurementBasis::Z,
                        duration_us: 5,
                    },
                    NeutralAtomAction::Reset {
                        atom: atom(1),
                        duration_us: 7,
                    },
                ],
            },
        ]
    }

    #[test]
    fn resource_report_counts_grouped_movement_and_layer_time() {
        let layers = toy_layers();
        let report = ResourceReport::from_layers(&layers);

        assert_eq!(report.rearrangement_steps, 1);
        assert_eq!(report.rearrangement_time_us, 10);
        assert_eq!(report.trap_transfers, 1);
        assert_eq!(report.transfer_time_us, 6);
        assert_eq!(report.rydberg_stages, 2);
        assert_eq!(report.entangle2_count, 1);
        assert_eq!(report.entangle_n_count, 1);
        assert_eq!(report.measurement_rounds, 1);
        assert_eq!(report.reset_rounds, 1);
        assert_eq!(report.wait_time_us, 4);
        assert_eq!(report.total_time_us, 30);
        assert_eq!(report.estimated_cycles, 3);
        assert_eq!(report.bottleneck, BottleneckKind::Rearrangement);
        assert_eq!(report.logical_qubits, 0);
        assert_eq!(report.physical_atoms, 0);
        assert_eq!(report.atoms_per_logical, None);
        assert_eq!(report.code_family, None);
    }

    #[test]
    fn empty_layers_have_zero_resource_usage() {
        let empty = ResourceReport::from_layers(&[]);
        assert_eq!(empty.estimated_cycles, 0);
        assert_eq!(empty.bottleneck, BottleneckKind::None);
        // Unlike `default()` (where the #298 single-qubit-gate fields are
        // `None`, meaning "never computed"), `from_layers` always computes
        // them — `Some(0)` for an empty schedule, not `None` — the same
        // "applies to every report" semantics as e.g. `entangle2_count`,
        // just kept `Option` for `deny_unknown_fields` schema evolution.
        assert_eq!(
            empty,
            ResourceReport {
                local_h_count: Some(0),
                local_rz_count: Some(0),
                local_u3_count: Some(0),
                local_gate_time_us: Some(0),
                global_ry_count: Some(0),
                global_ry_time_us: Some(0),
                ..ResourceReport::default()
            }
        );

        let blank_layer = ResourceReport::from_layers(&[ScheduleLayer {
            cycle: 0,
            actions: Vec::new(),
        }]);
        assert_eq!(blank_layer.estimated_cycles, 1);
        assert_eq!(blank_layer.bottleneck, BottleneckKind::None);
    }

    #[test]
    fn estimated_cycles_equals_layer_count_not_max_cycle() {
        let layers = [
            ScheduleLayer {
                cycle: 0,
                actions: vec![NeutralAtomAction::Wait { duration_us: 1 }],
            },
            ScheduleLayer {
                cycle: 99,
                actions: vec![NeutralAtomAction::Wait { duration_us: 1 }],
            },
        ];
        let report = ResourceReport::from_layers(&layers);
        assert_eq!(report.estimated_cycles, 2);
    }

    #[test]
    fn bottleneck_tie_is_mixed() {
        let layers = [ScheduleLayer {
            cycle: 0,
            actions: vec![
                NeutralAtomAction::Move(MovementGroup {
                    duration_us: 5,
                    moves: vec![AtomMove {
                        atom: atom(0),
                        from: site(0),
                        to: site(1),
                    }],
                }),
                NeutralAtomAction::Transfer(TrapTransfer {
                    atom: atom(0),
                    direction: TransferDirection::SlmToAod,
                    site: site(1),
                    aod: aod(),
                    duration_us: 5,
                }),
            ],
        }];
        let report = ResourceReport::from_layers(&layers);
        assert_eq!(report.rearrangement_time_us, 5);
        assert_eq!(report.transfer_time_us, 5);
        assert_eq!(report.bottleneck, BottleneckKind::Mixed);
    }

    #[test]
    fn bottleneck_rydberg_when_stages_dominate() {
        let layers = [
            ScheduleLayer {
                cycle: 0,
                actions: vec![NeutralAtomAction::Entangle2 {
                    atoms: [atom(0), atom(1)],
                    duration_us: 1,
                }],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![NeutralAtomAction::Entangle2 {
                    atoms: [atom(0), atom(1)],
                    duration_us: 1,
                }],
            },
            ScheduleLayer {
                cycle: 2,
                actions: vec![NeutralAtomAction::Move(MovementGroup {
                    duration_us: 1,
                    moves: vec![AtomMove {
                        atom: atom(0),
                        from: site(0),
                        to: site(1),
                    }],
                })],
            },
        ];
        let report = ResourceReport::from_layers(&layers);
        assert_eq!(report.rydberg_stages, 2);
        assert_eq!(report.rearrangement_time_us, 1);
        assert_eq!(report.bottleneck, BottleneckKind::Rydberg);
    }

    #[test]
    fn measurement_and_reset_rounds_count_layers_not_actions() {
        let report = ResourceReport::from_layers(&[ScheduleLayer {
            cycle: 0,
            actions: vec![
                NeutralAtomAction::Measure {
                    atom: atom(0),
                    basis: MeasurementBasis::X,
                    duration_us: 3,
                },
                NeutralAtomAction::Measure {
                    atom: atom(1),
                    basis: MeasurementBasis::Y,
                    duration_us: 5,
                },
                NeutralAtomAction::Reset {
                    atom: atom(2),
                    duration_us: 7,
                },
                NeutralAtomAction::Reset {
                    atom: atom(3),
                    duration_us: 2,
                },
            ],
        }]);

        assert_eq!(report.measurement_rounds, 1);
        assert_eq!(report.reset_rounds, 1);
        assert_eq!(report.total_time_us, 7);
        assert_eq!(report.estimated_cycles, 1);
        assert_eq!(report.bottleneck, BottleneckKind::Measurement);
    }

    #[test]
    fn simultaneous_layer_time_is_the_max() {
        for current in 0..16 {
            for next in 0..16 {
                let elapsed = simultaneous_layer_time(current, next);
                assert!(current <= elapsed);
                assert!(next <= elapsed);
                assert!(elapsed == current || elapsed == next);
            }
        }
    }

    #[test]
    fn with_physical_atoms_sets_one_to_one_sizing() {
        let report = ResourceReport::from_layers(&toy_layers()).with_physical_atoms(8);
        assert_eq!(report.logical_qubits, 8);
        assert_eq!(report.physical_atoms, 8);
        assert_eq!(report.atoms_per_logical, None);
        assert_eq!(report.code_family, None);
        assert_eq!(report.estimated_cycles, 3);
    }

    #[test]
    fn with_code_blocks_repetition_d3() {
        let blocks = repetition_code_blocks(&[3]);
        let report = match ResourceReport::from_layers(&[]).with_code_blocks(&blocks) {
            Ok(r) => r,
            Err(e) => panic!("with_code_blocks: {e}"),
        };
        assert_eq!(report.logical_qubits, 1);
        assert_eq!(report.physical_atoms, 5);
        assert_eq!(report.atoms_per_logical, Some(5));
        assert_eq!(report.code_family.as_deref(), Some("repetition_code_toy"));
    }

    #[test]
    fn with_code_blocks_qldpc_rate_one_over_twenty_four() {
        let block = sizing_only_block(
            CodeFamily::HighRateQldpcLike {
                net_rate: NetRate {
                    numerator: 1,
                    denominator: 24,
                },
            },
            12,
        );
        assert_eq!(block.atoms.len(), 288);
        let report = match ResourceReport::from_layers(&[]).with_code_blocks(&[block]) {
            Ok(r) => r,
            Err(e) => panic!("with_code_blocks: {e}"),
        };
        assert_eq!(report.logical_qubits, 12);
        assert_eq!(report.physical_atoms, 288);
        assert_eq!(report.atoms_per_logical, Some(24));
        assert_eq!(report.code_family.as_deref(), Some("high_rate_qldpc_like"));
    }

    #[test]
    fn mixed_code_families_leave_optional_rows_unset() {
        // Two repetition blocks at different distances share no single family
        // (distance differs) → the report leaves optional QEC rows unset.
        let blocks = repetition_code_blocks(&[3, 5]);
        let report = match ResourceReport::from_layers(&[]).with_code_blocks(&blocks) {
            Ok(r) => r,
            Err(e) => panic!("with_code_blocks: {e}"),
        };
        assert_eq!(report.logical_qubits, 2);
        assert_eq!(report.physical_atoms, 5 + 9);
        assert_eq!(report.atoms_per_logical, None);
        assert_eq!(report.code_family, None);
    }

    #[test]
    fn build_resource_report_empty_qec_slice_errors() {
        let err = build_resource_report(&[], Some(&[]), None);
        assert_eq!(err, Err(ReportError::EmptyCodeBlocks));
    }

    #[test]
    fn build_resource_report_prefers_qec_over_physical_hint() {
        let blocks = repetition_code_blocks(&[3]);
        let report = match build_resource_report(&[], Some(&blocks), Some(99)) {
            Ok(r) => r,
            Err(e) => panic!("build: {e}"),
        };
        assert_eq!(report.physical_atoms, 5);
        assert_ne!(report.physical_atoms, 99);
    }

    #[test]
    fn serializes_resource_report_metrics_to_json() {
        let report = ResourceReport {
            rydberg_stages: 2,
            rearrangement_steps: 3,
            rearrangement_time_us: 17,
            trap_transfers: 5,
            transfer_time_us: 11,
            entangle2_count: 7,
            entangle_n_count: 1,
            measurement_rounds: 13,
            reset_rounds: 19,
            wait_time_us: 23,
            total_time_us: 29,
            logical_qubits: 0,
            physical_atoms: 0,
            atoms_per_logical: None,
            code_family: None,
            distance: None,
            memory_rounds: None,
            estimated_cycles: 4,
            bottleneck: BottleneckKind::Rydberg,
            error_budget: None,
            ..ResourceReport::default()
        };

        let value = match serde_json::to_value(&report) {
            Ok(value) => value,
            Err(error) => panic!("resource report serialization failed: {error}"),
        };

        assert_eq!(
            value,
            json!({
                "evidence_kind": "analytic",
                "evidence_disclaimer": RESOURCE_REPORT_EVIDENCE_DISCLAIMER,
                "rydberg_stages": 2,
                "rearrangement_steps": 3,
                "rearrangement_time_us": 17,
                "trap_transfers": 5,
                "transfer_time_us": 11,
                "entangle2_count": 7,
                "entangle_n_count": 1,
                "measurement_rounds": 13,
                "reset_rounds": 19,
                "wait_time_us": 23,
                "total_time_us": 29,
                "logical_qubits": 0,
                "physical_atoms": 0,
                "estimated_cycles": 4,
                "bottleneck": "rydberg",
                "temporal_atom_metrics": {
                    "peak_atoms": 0,
                    "allocated_atoms_series": [],
                    "measurement_count": 0,
                    "reset_count": 0,
                    "reuse_count": 0,
                    "reused_ancilla_count": 0,
                },
            })
        );
    }

    #[test]
    fn deserializes_legacy_json_without_new_fields() {
        let value = json!({
            "rydberg_stages": 1,
            "rearrangement_steps": 0,
            "rearrangement_time_us": 0,
            "trap_transfers": 0,
            "transfer_time_us": 0,
            "entangle2_count": 0,
            "entangle_n_count": 0,
            "measurement_rounds": 0,
            "reset_rounds": 0,
            "wait_time_us": 0,
            "total_time_us": 0,
        });
        let report: ResourceReport = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => panic!("deserialize: {e}"),
        };
        assert_eq!(report.logical_qubits, 0);
        assert_eq!(report.physical_atoms, 0);
        assert_eq!(report.estimated_cycles, 0);
        assert_eq!(report.bottleneck, BottleneckKind::None);
        assert_eq!(report.atoms_per_logical, None);
        assert_eq!(report.evidence_kind, RESOURCE_REPORT_EVIDENCE_KIND);
        assert_eq!(
            report.evidence_disclaimer,
            RESOURCE_REPORT_EVIDENCE_DISCLAIMER
        );
    }

    #[test]
    fn markdown_omits_qec_rows_for_non_qec() {
        let report = ResourceReport::from_layers(&[]).with_physical_atoms(4);
        let md = resource_report_to_markdown(&report);
        assert!(md.contains("| Logical qubits | 4 |"));
        assert!(md.contains("| Physical atoms | 4 |"));
        assert!(!md.contains("Atoms per logical"));
        assert!(!md.contains("Code family"));
        assert!(!md.contains("N/A"));
        assert!(md.contains("# Neutral-atom analytic resource report"));
        assert!(md.contains("## Qubit resources"));
        assert!(md.contains("## Schedule metrics"));
        assert!(md.contains("## Notes"));
        assert!(md.contains("| Bottleneck | none |"));
    }

    #[test]
    fn markdown_includes_qec_rows_when_set() {
        let blocks = repetition_code_blocks(&[3]);
        let report = match ResourceReport::from_layers(&[]).with_code_blocks(&blocks) {
            Ok(r) => r,
            Err(e) => panic!("with_code_blocks: {e}"),
        };
        let md = resource_report_to_markdown(&report);
        assert!(md.contains("| Atoms per logical | 5 |"));
        assert!(md.contains("| Code family | repetition_code_toy |"));
        assert!(md.contains("| Distance | 3 |"));
    }

    fn example_error_model() -> NeutralAtomErrorModel {
        NeutralAtomErrorModel {
            rydberg: 0.002,
            measurement: 0.003,
            reset: 0.004,
            movement: 0.0005,
            transfer: 0.0007,
            idle_per_us: 2e-9,
        }
    }

    #[test]
    fn error_budget_is_rate_times_schedule_counts() {
        let report =
            ResourceReport::from_layers(&toy_layers()).with_error_budget(&example_error_model());
        let budget = match report.error_budget {
            Some(b) => b,
            None => panic!("budget attached"),
        };
        // toy_layers: 2 rydberg stages, 1 meas round, 1 reset, 1 move, 1 transfer, 4 wait µs
        assert!((budget.rydberg - 0.004).abs() < 1e-12);
        assert!((budget.measurement - 0.003).abs() < 1e-12);
        assert!((budget.reset - 0.004).abs() < 1e-12);
        assert!((budget.movement - 0.0005).abs() < 1e-12);
        assert!((budget.transfer - 0.0007).abs() < 1e-12);
        assert!((budget.idle - 8e-9).abs() < 1e-18);
    }

    fn example_fidelity() -> NeutralAtomFidelity {
        NeutralAtomFidelity {
            cz: 0.99,
            single_qubit: 0.98,
            atom_transfer: 0.97,
            coherence_time_us: 100.0,
        }
    }

    /// Hand-computable schedule for issue #305's unit-test acceptance
    /// criterion: 2 atoms, 1 `Entangle2` (CZ), 1 local rotation, known idle
    /// time.
    ///
    /// - Layer 0: `Entangle2` on atoms 0 and 1, 10 µs — both atoms active.
    /// - Layer 1: `LocalGate { gate: H }` on atom 0 only, 5 µs — atom 0
    ///   active, atom 1 idle for this layer's 5 µs.
    ///
    /// `total_time_us = 10 + 5 = 15`. `active(atom0) = 10 + 5 = 15` → idle
    /// `0`. `active(atom1) = 10` (layer 0 only) → idle `15 - 10 = 5`.
    fn fidelity_toy_layers() -> Vec<ScheduleLayer> {
        vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![NeutralAtomAction::Entangle2 {
                    atoms: [atom(0), atom(1)],
                    duration_us: 10,
                }],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![NeutralAtomAction::LocalGate {
                    atom: atom(0),
                    gate: LocalGateKind::H,
                    duration_us: 5,
                }],
            },
        ]
    }

    #[test]
    fn fidelity_estimate_matches_hand_computation() {
        let layers = fidelity_toy_layers();
        let report = ResourceReport::from_layers(&layers)
            .with_fidelity_estimate(&layers, &example_fidelity());

        assert_eq!(report.total_time_us, 15);
        assert_eq!(report.entangle2_count, 1);
        assert_eq!(report.local_h_count, Some(1));

        // gate_fidelity_product = cz^1 * single_qubit^1 (no transfers):
        // 0.99 * 0.98 = 0.9702. No idle decay folded in.
        let expected_gate_product = 0.99 * 0.98;
        let gate_fidelity_product = match report.gate_fidelity_product {
            Some(v) => v,
            None => panic!("gate_fidelity_product attached"),
        };
        assert!(
            (gate_fidelity_product - expected_gate_product).abs() < 1e-12,
            "gate_fidelity_product = {gate_fidelity_product}, expected {expected_gate_product}"
        );

        // idle decay: atom 0 has zero idle time (1 - 0/100 = 1.0); atom 1 is
        // idle for 5 of the coherence_time_us=100 window (1 - 5/100 = 0.95).
        let expected_estimated_fidelity = expected_gate_product * 1.0 * 0.95;
        let estimated_fidelity = match report.estimated_fidelity {
            Some(v) => v,
            None => panic!("estimated_fidelity attached"),
        };
        assert!(
            (estimated_fidelity - expected_estimated_fidelity).abs() < 1e-12,
            "estimated_fidelity = {estimated_fidelity}, expected {expected_estimated_fidelity}"
        );
        // Idle decay strictly lowers the estimate below the raw gate product.
        assert!(estimated_fidelity < gate_fidelity_product);
    }

    /// Schedule for issue #328's regression: two atoms (0 and 1) with an
    /// atom-addressed action providing the universe, plus a bare `GlobalRy`
    /// layer that should mark *both* atoms active.
    ///
    /// - Layer 0: `Entangle2` on atoms 0 and 1, 10 µs — both atoms active,
    ///   establishes the atom universe {0, 1}.
    /// - Layer 1: bare `GlobalRy { 0.4, 6 µs }` — a whole-plane raster.
    ///   Pre-#328 bug: `accumulate_action_atoms` inserts no atoms for
    ///   `GlobalRy`, so neither atom gets credit for this layer's 6 µs and
    ///   both are misattributed 6 µs of idle time. Post-#328 fix: both
    ///   atoms are active (the raster hits every trapped atom), so each
    ///   gets 6 µs of active credit.
    ///
    /// `total_time_us = 10 + 6 = 16`. Post-fix:
    /// `active(atom0) = 10 + 6 = 16` → idle 0.
    /// `active(atom1) = 10 + 6 = 16` → idle 0.
    /// Pre-fix (bug): `active(atom0) = 10`, `active(atom1) = 10` → idle 6 each.
    fn globalry_idle_toy_layers() -> Vec<ScheduleLayer> {
        vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![NeutralAtomAction::Entangle2 {
                    atoms: [atom(0), atom(1)],
                    duration_us: 10,
                }],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![NeutralAtomAction::GlobalRy {
                    theta_rad: 0.4,
                    duration_us: 6,
                }],
            },
        ]
    }

    #[test]
    fn idle_decay_marks_all_universe_atoms_active_for_globalry_layer() {
        // Issue #328 regression: a bare `GlobalRy` layer must mark every atom
        // in the schedule's atom universe as active for that layer's
        // max-duration, so the raster time is not misattributed to idle decay.
        let layers = globalry_idle_toy_layers();
        let report = ResourceReport::from_layers(&layers)
            .with_fidelity_estimate(&layers, &example_fidelity());

        assert_eq!(report.total_time_us, 16);
        assert_eq!(report.global_ry_count, Some(1));

        // gate_fidelity_product = cz^1 * single_qubit^1 (1 GlobalRy):
        // 0.99 * 0.98 = 0.9702.
        let expected_gate_product = 0.99 * 0.98;
        let gate_fidelity_product = match report.gate_fidelity_product {
            Some(v) => v,
            None => panic!("gate_fidelity_product attached"),
        };
        assert!(
            (gate_fidelity_product - expected_gate_product).abs() < 1e-12,
            "gate_fidelity_product = {gate_fidelity_product}, expected {expected_gate_product}"
        );

        // Post-fix: both atoms have zero idle time (active 16 of 16 µs), so
        // idle_decay = 1.0 * 1.0 = 1.0 → estimated == gate product.
        // Pre-fix (bug): each atom has 6 µs idle → (1 - 6/100) = 0.94 each,
        // so estimated = gate_product * 0.94^2 = 0.9702 * 0.8836 ≈ 0.8572.
        let estimated_fidelity = match report.estimated_fidelity {
            Some(v) => v,
            None => panic!("estimated_fidelity attached"),
        };
        assert!(
            (estimated_fidelity - expected_gate_product).abs() < 1e-12,
            "estimated_fidelity = {estimated_fidelity}, expected {expected_gate_product} \
             (both atoms active for the GlobalRy layer → zero idle decay)"
        );
    }

    #[test]
    fn idle_decay_excludes_atoms_known_only_through_globalry() {
        // Issue #328 invariant: an atom that appears *only* via `GlobalRy`
        // (no atom-addressed action anywhere in the schedule) is NOT part
        // of the atom universe and must not contribute an idle term.
        // A `GlobalRy` layer alone, with no other atom-addressed actions,
        // should produce an empty atom universe → idle_decay_product = 1.0
        // (empty product).
        let layers = vec![ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::GlobalRy {
                theta_rad: 0.4,
                duration_us: 6,
            }],
        }];
        let report = ResourceReport::from_layers(&layers)
            .with_fidelity_estimate(&layers, &example_fidelity());

        assert_eq!(report.total_time_us, 6);
        assert_eq!(report.global_ry_count, Some(1));

        let gate_fidelity_product = match report.gate_fidelity_product {
            Some(v) => v,
            None => panic!("gate_fidelity_product attached"),
        };
        // gate_fidelity_product = single_qubit^1 = 0.98.
        assert!((gate_fidelity_product - 0.98).abs() < 1e-12);

        // No atom-addressed actions → empty universe → idle_decay = 1.0
        // (product over zero atoms). estimated == gate product.
        let estimated_fidelity = match report.estimated_fidelity {
            Some(v) => v,
            None => panic!("estimated_fidelity attached"),
        };
        assert!(
            (estimated_fidelity - gate_fidelity_product).abs() < 1e-12,
            "estimated_fidelity = {estimated_fidelity}, expected {gate_fidelity_product} \
             (empty atom universe → no idle decay term)"
        );
    }

    #[test]
    fn idle_decay_globalry_in_mixed_layer_marks_all_universe_atoms_active() {
        // Issue #328: when a `GlobalRy` shares a layer with an atom-addressed
        // action (e.g. a local gate on one atom), the `GlobalRy` still marks
        // *all* universe atoms active for that layer — not just the atom
        // addressed by the co-resident local gate.
        //
        // - Layer 0: `Entangle2` on atoms 0,1 (10 µs) → universe {0, 1}.
        // - Layer 1: `LocalGate { H }` on atom 0 (3 µs) + `GlobalRy` (4 µs).
        //   Layer max-duration = 4 µs. Both atoms must be active for 4 µs.
        //
        // total_time = 10 + 4 = 14.
        // active(0) = 10 + 4 = 14 → idle 0.
        // active(1) = 10 + 4 = 14 → idle 0.  (Pre-fix: active(1) = 10 → idle 4.)
        let layers = vec![
            ScheduleLayer {
                cycle: 0,
                actions: vec![NeutralAtomAction::Entangle2 {
                    atoms: [atom(0), atom(1)],
                    duration_us: 10,
                }],
            },
            ScheduleLayer {
                cycle: 1,
                actions: vec![
                    NeutralAtomAction::LocalGate {
                        atom: atom(0),
                        gate: LocalGateKind::H,
                        duration_us: 3,
                    },
                    NeutralAtomAction::GlobalRy {
                        theta_rad: 0.4,
                        duration_us: 4,
                    },
                ],
            },
        ];
        let report = ResourceReport::from_layers(&layers)
            .with_fidelity_estimate(&layers, &example_fidelity());

        assert_eq!(report.total_time_us, 14);

        // gate_fidelity_product = cz^1 * single_qubit^2 (1 H + 1 GlobalRy):
        // 0.99 * 0.98^2 = 0.99 * 0.9604 = 0.950796.
        let expected_gate_product = 0.99 * 0.98 * 0.98;
        let gate_fidelity_product = match report.gate_fidelity_product {
            Some(v) => v,
            None => panic!("gate_fidelity_product attached"),
        };
        assert!(
            (gate_fidelity_product - expected_gate_product).abs() < 1e-12,
            "gate_fidelity_product = {gate_fidelity_product}, expected {expected_gate_product}"
        );

        // Post-fix: both atoms active for 14 of 14 µs → idle 0 → no decay.
        let estimated_fidelity = match report.estimated_fidelity {
            Some(v) => v,
            None => panic!("estimated_fidelity attached"),
        };
        assert!(
            (estimated_fidelity - expected_gate_product).abs() < 1e-12,
            "estimated_fidelity = {estimated_fidelity}, expected {expected_gate_product} \
             (GlobalRy in mixed layer marks all universe atoms active → zero idle decay)"
        );
    }

    #[test]
    fn fidelity_estimate_counts_global_ry_echo_pulses() {
        // Regression for issue #305's central correctness point: a `GlobalRy`
        // with a bystander atom present splits into two half-pulses plus a
        // Rz(pi)/Rz(-pi) echo pair on the bystander (issue #298's
        // `push_global_ry_with_refocus`) — every one of those is a genuine
        // schedule action and must be counted as its own `single_qubit`
        // factor, not collapsed to "one source-program `ry` gate".
        let layers = vec![ScheduleLayer {
            cycle: 0,
            actions: vec![
                NeutralAtomAction::GlobalRy {
                    theta_rad: 0.4,
                    duration_us: 1,
                },
                NeutralAtomAction::LocalGate {
                    atom: atom(1),
                    gate: LocalGateKind::Rz(std::f64::consts::PI),
                    duration_us: 1,
                },
                NeutralAtomAction::GlobalRy {
                    theta_rad: 0.4,
                    duration_us: 1,
                },
                NeutralAtomAction::LocalGate {
                    atom: atom(1),
                    gate: LocalGateKind::Rz(-std::f64::consts::PI),
                    duration_us: 1,
                },
            ],
        }];
        let report = ResourceReport::from_layers(&layers)
            .with_fidelity_estimate(&layers, &example_fidelity());

        // 2 GlobalRy half-pulses + 2 Rz echo pulses = 4 single_qubit factors,
        // not 1 (a naive "logical ry gate count" would undercount this).
        assert_eq!(report.global_ry_count, Some(2));
        assert_eq!(report.local_rz_count, Some(2));
        let expected = example_fidelity().single_qubit.powf(4.0);
        let gate_fidelity_product = match report.gate_fidelity_product {
            Some(v) => v,
            None => panic!("gate_fidelity_product attached"),
        };
        assert!(
            (gate_fidelity_product - expected).abs() < 1e-12,
            "gate_fidelity_product = {gate_fidelity_product}, expected {expected} \
             (4 single_qubit-fidelity factors: 2 GlobalRy half-pulses + 2 Rz echo pulses)"
        );
    }

    #[test]
    fn fidelity_estimate_unset_by_default() {
        let report = ResourceReport::from_layers(&toy_layers());
        assert_eq!(report.gate_fidelity_product, None);
        assert_eq!(report.estimated_fidelity, None);
        let json = match serde_json::to_string(&report) {
            Ok(s) => s,
            Err(e) => panic!("serialize: {e}"),
        };
        assert!(!json.contains("gate_fidelity_product"));
        assert!(!json.contains("estimated_fidelity"));
    }

    #[test]
    fn error_budget_json_round_trip_fields() {
        let report =
            ResourceReport::from_layers(&toy_layers()).with_error_budget(&example_error_model());
        let value = match serde_json::to_value(&report) {
            Ok(v) => v,
            Err(e) => panic!("serialize: {e}"),
        };
        let budget = match value.get("error_budget") {
            Some(b) => b,
            None => panic!("error_budget present in JSON"),
        };
        assert_eq!(budget["rydberg"], json!(0.004));
        assert_eq!(budget["measurement"], json!(0.003));
        assert_eq!(budget["reset"], json!(0.004));
        assert_eq!(budget["movement"], json!(0.0005));
        assert_eq!(budget["transfer"], json!(0.0007));
        assert_eq!(budget["idle"], json!(8e-9));
        let back: ResourceReport = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => panic!("deserialize: {e}"),
        };
        assert_eq!(back.error_budget, report.error_budget);
    }

    #[test]
    fn attach_qec_error_budget_fails_when_model_missing() {
        let report = ResourceReport::from_layers(&[]);
        let err = attach_qec_error_budget(report, None);
        assert_eq!(err, Err(ReportError::MissingErrorModel));
    }

    #[test]
    fn attach_qec_error_budget_succeeds_when_model_present() {
        let report = ResourceReport::from_layers(&toy_layers());
        let report = match attach_qec_error_budget(report, Some(&example_error_model())) {
            Ok(r) => r,
            Err(e) => panic!("attach: {e}"),
        };
        assert!(report.error_budget.is_some());
    }

    #[test]
    fn require_target_error_model_maps_missing() {
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../targets/neutral_atom/generic_rna_v0.json"
        ));
        let loaded = match backend::json::load(path) {
            Ok(t) => t,
            Err(e) => panic!("load: {e}"),
        };
        let mut na = match loaded.neutral_atom_target() {
            Some(t) => t.clone(),
            None => panic!("expected neutral atom target"),
        };
        assert!(require_target_error_model(&na).is_ok());
        na.error_model = None;
        assert_eq!(
            require_target_error_model(&na),
            Err(ReportError::MissingErrorModel)
        );
    }

    #[test]
    fn markdown_includes_error_budget_when_set() {
        let report =
            ResourceReport::from_layers(&toy_layers()).with_error_budget(&example_error_model());
        let md = resource_report_to_markdown(&report);
        assert!(md.contains("## Physical error budget"));
        assert!(md.contains("| Category | Contribution (rate × count) |"));
        assert!(md.contains("| Rydberg | 0.004 |"));
        assert!(md.contains("| Idle | 8e-9 |"));
        assert!(md.contains("schedule-count × rate"));
        assert!(md.contains("analytic"));
        assert!(md.contains("Sinter"));
        assert!(md.contains("ADR-0020"));
        assert!(!md.contains("logical_failures"));
    }

    #[test]
    fn resource_report_json_excludes_sampled_sinter_fields() {
        let report =
            ResourceReport::from_layers(&toy_layers()).with_error_budget(&example_error_model());
        let json = match resource_report_to_json(&report) {
            Ok(s) => s,
            Err(e) => panic!("serialize: {e}"),
        };
        let value: serde_json::Value = match serde_json::from_str(&json) {
            Ok(v) => v,
            Err(e) => panic!("parse emit: {e}"),
        };
        let obj = match value.as_object() {
            Some(o) => o,
            None => panic!("object"),
        };
        assert_eq!(
            obj.get("evidence_kind").and_then(|v| v.as_str()),
            Some(RESOURCE_REPORT_EVIDENCE_KIND)
        );
        assert!(
            obj.get("evidence_disclaimer")
                .and_then(|v| v.as_str())
                .is_some_and(|s| s.contains("ADR-0020") && s.contains("threshold")),
            "JSON must carry an anti-threshold analytic disclaimer"
        );
        for key in [
            "logical_failures",
            "logical_failure_rate",
            "shots",
            "sinter",
            "p_logical",
            "pL",
        ] {
            assert!(!obj.contains_key(key), "unexpected sampled field `{key}`");
        }
        assert!(obj.contains_key("error_budget"));
        assert!(obj.contains_key("estimated_cycles"));
        assert!(obj.contains_key("bottleneck"));

        // Non-tautological: deny_unknown_fields rejects fused sinter keys on round-trip.
        let mut fused = match serde_json::to_value(&report) {
            Ok(v) => v,
            Err(e) => panic!("to_value: {e}"),
        };
        fused
            .as_object_mut()
            .expect("object")
            .insert("logical_failures".into(), json!(3));
        let rejected = serde_json::from_value::<ResourceReport>(fused);
        assert!(
            rejected.is_err(),
            "ResourceReport must reject unknown sinter fields (deny_unknown_fields)"
        );
    }

    #[test]
    fn format_contribution_uses_scientific_below_1e_minus_4() {
        assert_eq!(format_contribution(0.004), "0.004");
        assert_eq!(format_contribution(0.0005), "0.0005");
        assert_eq!(format_contribution(1e-4), "0.0001");
        assert_eq!(format_contribution(8e-9), "8e-9");
        assert_eq!(format_contribution(0.0), "0");
        // Binary dust from rate × count must not appear in emit.
        assert_eq!(format_contribution(0.003 * 3.0), "0.009");
        assert_eq!(clean_contribution(0.003 * 3.0), 0.009);
    }

    #[test]
    fn markdown_omits_error_budget_note_when_unset() {
        let md = resource_report_to_markdown(&ResourceReport::from_layers(&[]));
        assert!(!md.contains("## Physical error budget"));
        assert!(!md.contains("Physical error budget lines"));
    }

    #[test]
    fn aware_search_status_defaults_to_unset() {
        let report = ResourceReport::from_layers(&toy_layers());
        assert_eq!(report.aware_search_completed_layers, None);
        assert_eq!(report.aware_search_fell_back_layers, None);
        let md = resource_report_to_markdown(&report);
        assert!(!md.contains("Routing-aware search"));
    }

    #[test]
    fn aware_search_status_overlay_round_trips_through_json() {
        let report = ResourceReport::from_layers(&toy_layers()).with_aware_search_status(3, 1);
        assert_eq!(report.aware_search_completed_layers, Some(3));
        assert_eq!(report.aware_search_fell_back_layers, Some(1));

        let value = match serde_json::to_value(&report) {
            Ok(v) => v,
            Err(e) => panic!("serialize: {e}"),
        };
        assert_eq!(value["aware_search_completed_layers"], json!(3));
        assert_eq!(value["aware_search_fell_back_layers"], json!(1));
        let back: ResourceReport = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => panic!("deserialize: {e}"),
        };
        assert_eq!(back, report);
    }

    #[test]
    fn markdown_flags_aware_fallback_when_nonzero() {
        let fell_back = ResourceReport::from_layers(&toy_layers()).with_aware_search_status(0, 4);
        let md = resource_report_to_markdown(&fell_back);
        assert!(md.contains("| Routing-aware search completed layers | 0 |"));
        assert!(md.contains("| Routing-aware search fell back to greedy (layers) | 4 |"));
        assert!(md.contains("not evidence of"));

        let completed = ResourceReport::from_layers(&toy_layers()).with_aware_search_status(4, 0);
        let md_ok = resource_report_to_markdown(&completed);
        assert!(md_ok.contains("| Routing-aware search fell back to greedy (layers) | 0 |"));
        assert!(!md_ok.contains("not evidence of"));
    }

    #[test]
    fn agnostic_placer_mechanism_defaults_to_unset() {
        let report = ResourceReport::from_layers(&toy_layers());
        assert_eq!(report.agnostic_placer_mechanism, None);
        let md = resource_report_to_markdown(&report);
        assert!(!md.contains("Routing-agnostic placement mechanism"));
    }

    #[test]
    fn agnostic_placer_mechanism_overlay_round_trips_through_json() {
        let report = ResourceReport::from_layers(&toy_layers())
            .with_agnostic_placer_mechanism(Some(AgnosticPlacerMechanism::Matching));
        assert_eq!(
            report.agnostic_placer_mechanism,
            Some(AgnosticPlacerMechanism::Matching)
        );

        let value = match serde_json::to_value(&report) {
            Ok(v) => v,
            Err(e) => panic!("serialize: {e}"),
        };
        assert_eq!(value["agnostic_placer_mechanism"], json!("matching"));
        let back: ResourceReport = match serde_json::from_value(value) {
            Ok(r) => r,
            Err(e) => panic!("deserialize: {e}"),
        };
        assert_eq!(back, report);

        // GreedyFallback variant + markdown rendering.
        let fell_back = ResourceReport::from_layers(&toy_layers())
            .with_agnostic_placer_mechanism(Some(AgnosticPlacerMechanism::GreedyFallback));
        let md = resource_report_to_markdown(&fell_back);
        assert!(md.contains("| Routing-agnostic placement mechanism | greedy_fallback |"));
        let md_match = resource_report_to_markdown(&report);
        assert!(md_match.contains("| Routing-agnostic placement mechanism | matching |"));
    }

    #[test]
    fn error_budget_never_uses_one_minus_fidelity() {
        // Load a real target with both fidelity and error_model; budget must
        // follow error_model.rydberg, not 1 - fidelity.cz.
        let path = std::path::Path::new(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../targets/neutral_atom/generic_rna_v0.json"
        ));
        let loaded = match backend::json::load(path) {
            Ok(t) => t,
            Err(e) => panic!("load: {e}"),
        };
        let na = match loaded.neutral_atom_target() {
            Some(t) => t,
            None => panic!("expected neutral atom target"),
        };
        let model = match require_target_error_model(na) {
            Ok(m) => *m,
            Err(e) => panic!("require: {e}"),
        };
        assert!(
            (model.rydberg - (1.0 - na.fidelity.cz)).abs() > 1e-9,
            "example rates must not equal 1 - fidelity.cz"
        );
        let report = ResourceReport {
            rydberg_stages: 2,
            ..Default::default()
        }
        .with_error_budget(&model);
        let budget = match report.error_budget {
            Some(b) => b,
            None => panic!("budget"),
        };
        assert!((budget.rydberg - 2.0 * model.rydberg).abs() < 1e-12);
        assert!((budget.rydberg - 2.0 * (1.0 - na.fidelity.cz)).abs() > 1e-6);
    }

    #[test]
    fn missing_error_model_messages_align() {
        let backend_msg = BackendError::MissingErrorModel.to_string();
        let report_msg = ReportError::MissingErrorModel.to_string();
        assert_eq!(backend_msg, report_msg);
        assert!(backend_msg.contains("--emit-resource-report"));
        assert!(backend_msg.contains("do not derive from fidelity"));
    }
}
