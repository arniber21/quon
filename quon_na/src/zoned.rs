//! Zoned routing-aware placement (issue #107, heuristic search #297).
//!
//! Reproduces the **placement cost = routing cost** formulation of
//! [RAP] (Stade, Lin, Cong, Wille, ICCAD 2025, arXiv:2505.22715):
//!
//! - Sec. III-B — routing-aware definition (layer-by-layer; cost is routing)
//! - Sec. III-A — reuse analysis (“don’t move atoms already in place”)
//! - Sec. IV-A — cost Eq. (1): `cost(p) = Σ_G √(d_max(G))` over greedily
//!   grouped compatible movements ([`routing_cost_eq1`])
//! - Sec. IV-B — search extends by assigning one gate’s atoms to entanglement
//!   pairs (A*-style / best-first): [`assign_aware_legal`]
//! - Sec. IV-C / V-C, Eqs. (3)-(5) — the guiding heuristic
//!   ([`AwareSearchParams`], `heuristic_estimate`): an admissible-lower-bound
//!   term (Eq. 3: worst-case nearest-available distance among unplaced gates
//!   vs. the current largest group distance) plus an inadmissible
//!   accelerating term (Eq. 4: `δ · (β + ΣSD(groups)) · |unplaced|`, favoring
//!   partial placements whose per-group displacement is close to uniform —
//!   i.e. parallel, easy to extend). The Eq. (3) sign convention (`max(0,
//!   √unplaced − √placed)`, not a bare subtraction) and `δ`/`β` as qmap's
//!   `deepeningFactor`/`deepeningValue` are cross-checked against qmap's
//!   reference `HeuristicPlacer` (`munich-quantum-toolkit/qmap`,
//!   `src/na/zoned/layout_synthesizer/placer/HeuristicPlacer.cpp`,
//!   `getHeuristic`/`sumStdDeviationForGroups`) and match. `SD(G)` itself is
//!   *inspired by* qmap's rank-based approach, not a literal port: qmap
//!   discretizes row/column indices once, globally, over the whole layer's
//!   job list (real lattice grid positions, capped via `min(1.0, …)`, shared
//!   across groups via its BST); this module instead recomputes ranks
//!   locally, per group, per axis, at heuristic-evaluation time (uncapped) —
//!   a materially coarser quantity for small groups (e.g. a 2-member group
//!   always reduces to ranks `{0,1}` regardless of actual lattice position).
//!   Since this term is already inadmissible/heuristic-only by design (Eq.
//!   4 has no correctness requirement — [`AwareSearchOutcome::Completed`]
//!   only claims *a* full assignment was found, not the true joint optimum,
//!   see below), this doesn't threaten correctness, but it is not a faithful
//!   reproduction of qmap's discretization. Eq.
//!   (5)'s cross-layer look-ahead term (and Eq. (2)'s `α`/`γ` reuse-cost
//!   terms) are **not** ported: this module's search only decides gate→pair
//!   assignment within one layer (no atom-by-atom intermediate/storage
//!   placement across layers, so there is no natural "next gate's partner"
//!   hook to look ahead to) — documented scope reduction, not an oversight.
//! - Sec. V-A — binary-search-tree movement-group compatibility check: this
//!   module uses the same per-axis order/coupling test
//!   ([`positions_aod_compatible`]) both post-search (`partition_aod_compatible`)
//!   and, as of #297, *during* the search itself (search-time `groups` on
//!   [`AwareNode`]) so a node's cost matches what routing will actually emit.
//!   Linear-scan compatibility checking (not a literal BST) — legal at this
//!   crate's layer sizes (≤ tens of gates), a documented simplification of
//!   the paper's data structure, not of its legality semantics.
//! - Sec. V-D — pruning: [`AwareSearchParams::pruning_window`] bounds each
//!   gate's considered pair choices to the nearest-`window` *legal* ones
//!   (scanned from the full legal set, not truncated before legality
//!   filtering — so a gate is never denied a choice purely because a closer
//!   *illegal* pair occupied a window slot). This does **not** preserve full
//!   completeness: if the only full legal assignment for a layer requires
//!   some gate to take a choice outside its window (e.g. its near pairs are
//!   needed by other gates and its only remaining legal pair is the
//!   `window + 1`-th nearest), that assignment is never generated, and the
//!   search can report [`AwareSearchOutcome::NoLegalAssignment`] even though
//!   a full legal assignment exists. Same tradeoff spirit as `beam_width`
//!   below (bounded search, not exhaustive) — not observed to matter on the
//!   `ising_n42` anchor fixture (0 fallbacks either way at `window = 32`),
//!   but a real, intentional gap, not an oversight.
//! - **Not from the paper** — [`AwareSearchParams::beam_width`]: the search
//!   frontier is trimmed to the best `beam_width` nodes whenever it exceeds
//!   `2 × beam_width`. Needed empirically (#297 review): at real fixture
//!   scale (`ising_n42`'s 20-21-gate/340-pair layers with real crosstalk
//!   conflicts between adjacent entanglement-zone pairs), an untrimmed
//!   priority queue fills with shallow alternatives faster than deep ones
//!   get explored, and `node_budget` expansions are spent almost entirely
//!   widening rather than deepening — measured reaching a 1M+-node frontier
//!   while still stuck below half depth. Standard beam search; a departure
//!   from the paper's (unspecified) search-loop mechanics, not from its
//!   legality semantics.
//!
//! Readout-zone measurement constraints come from [AbstractModel]
//! (arXiv:2405.08068) Sec. III-A, **not** from [RAP] (which models only
//! storage + entanglement). Flat AOD movement (#106) is a distinct Enola /
//! OLSQ-DPQA line — do not cite this module as that planner.
//!
//! Dual modes ([RAP] Sec. VI-B comparison methodology):
//! - [`PlacerMode::RoutingAgnostic`] — ZAC-style distance-minimizing placement
//! - [`PlacerMode::RoutingAware`] — heuristic-guided search minimizing Eq. (1)
//!   routing cost. With the Eq. (3)-(5) heuristic (#297), this is true A*
//!   (not uniform-cost): the accelerating term is intentionally inadmissible,
//!   so a completed search is the search's best find, not a proven joint
//!   optimum — matching the paper's own framing ("accepting that the
//!   resulting heuristic may not be admissible anymore", Sec. IV-C).

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::graph::{InteractionGraph, LogicalQubitId, VertexId};
use crate::layout::AodTrapRef;
use crate::layout::{
    AtomBinding, AtomId, AtomSite, NeutralAtomLayout, Position, SiteId, TrapBinding,
};
use crate::matching::{FORBIDDEN_COST, min_weight_assignment};
use crate::schedule::{
    AtomMove, MovementGroup, NeutralAtomAction, ScheduleLayer, TransferDirection, TrapTransfer,
};
use crate::schedule_entry::GraphScheduleRequest;

#[cfg(feature = "flux")]
use flux_rs::attrs::*;

/// Zone capability taxonomy ([AbstractModel] Sec. III-A; [RAP] Sec. II-A).
///
/// Owned by `backend` (issue #212): one `ZoneKind` for the workspace,
/// re-exported here so the zoned placer's public API is unchanged.
pub use backend::ZoneKind;

/// One zone on a zoned neutral-atom architecture.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZoneSpec {
    pub zone_id: u32,
    pub kind: ZoneKind,
    pub rows: u32,
    pub cols: u32,
    pub origin_um: (f64, f64),
    pub site_pitch_um: (f64, f64),
    /// Entanglement zones only: gap between the two traps of a pair.
    pub pair_gap_um: Option<f64>,
}

impl ZoneSpec {
    pub fn capacity_sites(&self) -> u32 {
        self.rows.saturating_mul(self.cols)
    }

    /// Entanglement-zone pair capacity (rows × cols when kind is Entanglement).
    pub fn pair_capacity(&self) -> u32 {
        match self.kind {
            ZoneKind::Entanglement => self.capacity_sites(),
            _ => 0,
        }
    }

    pub fn site_position(&self, row: u32, col: u32) -> Position {
        Position {
            x_um: self.origin_um.0 + f64::from(col) * self.site_pitch_um.0,
            y_um: self.origin_um.1 + f64::from(row) * self.site_pitch_um.1,
        }
    }
}

/// Architecture slice needed by the zoned placer (mirrors target JSON fields).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZonedArchitecture {
    pub zones: Vec<ZoneSpec>,
    pub acceleration_m_s2: f64,
    pub trap_transfer_us: u64,
    pub require_readout_zone: bool,
    /// Rydberg interaction range (µm) for simultaneous-gate legality.
    /// `0.0` disables the constraint (hand-built test architectures).
    #[serde(default)]
    pub rydberg_range_um: f64,
    /// Isolation spacing (µm) required between non-partner atoms of
    /// simultaneous gates. `0.0` disables the constraint.
    #[serde(default)]
    pub min_rydberg_spacing_um: f64,
    /// Minimum separation (µm) between distinct AOD rows / columns at a
    /// grouped move's destination. `0.0` disables the constraint.
    #[serde(default)]
    pub aod_min_separation_um: f64,
}

impl ZonedArchitecture {
    /// Minimum legal distance between atoms of two *different* simultaneous
    /// entangling pairs: closer than the Rydberg range is compulsory
    /// entanglement, closer than the isolation spacing is crosstalk (dialect
    /// R2/R3). `0.0` when the architecture carries no interaction limits.
    pub fn pair_conflict_um(&self) -> f64 {
        self.rydberg_range_um.max(self.min_rydberg_spacing_um)
    }

    pub fn zone_capacity(&self, kind: ZoneKind) -> u32 {
        self.zones
            .iter()
            .filter(|z| z.kind == kind)
            .map(ZoneSpec::capacity_sites)
            .sum()
    }

    pub fn entanglement_zones(&self) -> impl Iterator<Item = &ZoneSpec> {
        self.zones
            .iter()
            .filter(|z| z.kind == ZoneKind::Entanglement)
    }

    pub fn storage_zones(&self) -> impl Iterator<Item = &ZoneSpec> {
        self.zones.iter().filter(|z| z.kind == ZoneKind::Storage)
    }

    pub fn validate(&self) -> Result<(), ZonedScheduleError> {
        if self.acceleration_m_s2 <= 0.0 {
            return Err(ZonedScheduleError::InvalidAcceleration(
                self.acceleration_m_s2,
            ));
        }
        if !self.entanglement_zones().any(|_| true) {
            return Err(ZonedScheduleError::MissingEntanglementZone);
        }
        if !self.storage_zones().any(|_| true) {
            return Err(ZonedScheduleError::MissingStorageZone);
        }
        Ok(())
    }
}

/// Placer mode ([RAP] Sec. VI-B agnostic-vs-aware pairs).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacerMode {
    /// ZAC-style: minimize travel distance into entanglement pairs.
    RoutingAgnostic,
    /// RAP: minimize Eq. (1) routing cost of the transition.
    RoutingAware,
    /// SMT-optimal placement (issue #302, Deliverable B). Uses z3 to find
    /// the optimal initial atom→site assignment on the flat AOD path, or
    /// the optimal initial storage placement on the zoned path. Falls back
    /// to [`PlacerMode::RoutingAgnostic`] with a logged optimality gap when
    /// the `solver` feature is off or z3 times out.
    Exact,
}

/// Which routing-agnostic placement mechanism ran for a layer (issue #300).
///
/// The agnostic path now has two mechanisms: the new min-weight bipartite
/// matching placer ([`assign_matching_legal`], the Lin et al. 2025
/// `VertexMatchingPlacer` parity target) and the original greedy nearest-legal
/// placer ([`assign_greedy_legal`]), kept as a fast fallback for very large
/// layers and for when matching's conflict-repair cannot find a spacing-legal
/// assignment. This enum records which one produced a given schedule so a
/// `routing-agnostic` compile is never silently one or the other.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgnosticPlacerMechanism {
    /// [`assign_matching_legal`] produced the layer's assignment (min-weight
    /// bipartite matching, the #300 default for normal-size layers).
    Matching,
    /// [`assign_greedy_legal`] produced the layer's assignment — either because
    /// the layer exceeded the [`MATCHING_FALLBACK_GATE_PAIR_PRODUCT`] threshold
    /// (very large, where the O(n²·m) matching is skipped for speed) or because
    /// the dispatch's group-count comparison ([`pick_agnostic_assignment`])
    /// kept greedy: matching's min-travel optimum can pack gates into *more*
    /// AOD movement stages than the spread-out greedy choice, so greedy is kept
    /// whenever it yields ≤ matching's rearrangement-step count. In both cases
    /// the matching placer still ran (for normal-size layers); this records
    /// that its result was not the one emitted.
    GreedyFallback,
}

impl AgnosticPlacerMechanism {
    /// Snake_case wire / Markdown cell text matching JSON serde.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Matching => "matching",
            Self::GreedyFallback => "greedy_fallback",
        }
    }
}

/// Result of [`schedule_zoned`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZonedScheduleResult<V = LogicalQubitId> {
    pub request: GraphScheduleRequest<V>,
    pub mode: PlacerMode,
    /// Σ_G √(d_max(G)) over emitted movement groups ([RAP] Eq. (1)).
    pub routing_cost: f64,
    pub rearrangement_steps: u64,
    pub trap_transfers: u64,
    /// Number of per-layer gate-assignment calls where [`assign_aware_legal`]
    /// found a full legal assignment within budget. As of #297 this is a
    /// heuristic-guided A* with an intentionally inadmissible accelerating
    /// term (Eqs. (3)-(4)), so `Completed` means "the search's best find",
    /// not a proven joint optimum for the layer (matching [RAP]'s own
    /// framing — see the module doc). Always `0` under
    /// [`PlacerMode::RoutingAgnostic`] (the concept doesn't apply).
    pub aware_search_completed_layers: u64,
    /// Per-layer calls where the aware search exhausted the expansion budget
    /// before finding a full assignment and fell back to
    /// [`assign_greedy_legal`] (issue #111 review finding: this makes a
    /// budget-exhaustion fallback — which can silently reproduce the greedy
    /// schedule byte-for-byte — visible instead of indistinguishable from "no
    /// routing contention"). Always `0` under [`PlacerMode::RoutingAgnostic`].
    pub aware_search_budget_exceeded_layers: u64,
    /// Per-layer calls where the aware search exhausted its entire search
    /// space (no legal full assignment exists, e.g. spacing/occupancy
    /// conflicts) and fell back to [`assign_greedy_legal`]. Always `0` under
    /// [`PlacerMode::RoutingAgnostic`].
    pub aware_search_no_legal_assignment_layers: u64,
    /// Sum of best-first search node expansions across every
    /// [`assign_aware_legal`] call this schedule made (issue #307: exposes
    /// search cost, not just its pass/fail outcome). Always `0` under
    /// [`PlacerMode::RoutingAgnostic`].
    pub aware_search_node_expansions: u64,
    /// Per-layer routing-agnostic calls where [`assign_matching_legal`] (the
    /// #300 min-weight bipartite matching placer) produced the layer's
    /// assignment. Always `0` under [`PlacerMode::RoutingAware`].
    pub agnostic_matching_layers: u64,
    /// Per-layer routing-agnostic calls where the agnostic path instead used
    /// [`assign_greedy_legal`] — either because the layer exceeded the
    /// [`MATCHING_FALLBACK_GATE_PAIR_PRODUCT`] threshold (very large layer,
    /// where the O(n²·m) matching is skipped for speed) or because the
    /// dispatch's group-count comparison ([`pick_agnostic_assignment`]) kept
    /// greedy (matching's min-travel optimum grouped into ≥ as many AOD
    /// movement stages). Always `0` under [`PlacerMode::RoutingAware`]. See
    /// [`AgnosticPlacerMechanism`].
    pub agnostic_greedy_fallback_layers: u64,
}

#[derive(Debug, Error, Clone, PartialEq)]
pub enum ZonedScheduleError {
    #[error("acceleration_m_s2 must be positive, got {0}")]
    InvalidAcceleration(f64),
    #[error("zoned architecture requires at least one entanglement zone")]
    MissingEntanglementZone,
    #[error("zoned architecture requires at least one storage zone")]
    MissingStorageZone,
    #[error("entangling action outside the entanglement zone (atom {0:?})")]
    EntangleOutsideZone(AtomId),
    #[error("measurement outside readout zone while require_readout_zone is set (atom {0:?})")]
    MeasureOutsideReadout(AtomId),
    #[error("zone {0} occupancy {1} exceeds capacity {2}")]
    OccupancyExceeded(u32, u32, u32),
    #[error("not enough entanglement-zone pairs for {0} simultaneous gates")]
    InsufficientPairs(usize),
    #[error(
        "no legal entanglement pair for gate ({0:?}, {1:?}): every pair is occupied by a parked atom or spacing-conflicted"
    )]
    NoLegalPair(AtomId, AtomId),
    #[error("empty schedule: no entangling layers to place")]
    EmptySchedule,
    #[error("schedule layer conflict: {0}")]
    Conflict(String),
}

/// √(d_max / a) duration contribution for one movement group ([RAP] Eq. (1)).
///
/// `d_max_um` is micrometres; `acceleration_m_s2` is m/s². Returns seconds×1e6
/// scaled consistently as a dimensionless √(µm) proxy when a is fixed — we
/// return `sqrt(d_max_um)` when comparing placements at fixed a (Eq. (1)
/// factor 1/√a cancels in argmin). For absolute time use
/// [`movement_duration_us`].
#[cfg_attr(
    feature = "flux",
    spec(fn(d_max_um: f64) -> f64{v: v >= 0.0})
)]
pub fn sqrt_d_max(d_max_um: f64) -> f64 {
    if d_max_um <= 0.0 {
        0.0
    } else {
        d_max_um.sqrt()
    }
}

/// Move duration in µs: t = √(d/a) with d in metres, a in m/s², result ×1e6.
pub fn movement_duration_us(d_max_um: f64, acceleration_m_s2: f64) -> u64 {
    if d_max_um <= 0.0 || acceleration_m_s2 <= 0.0 {
        return 0;
    }
    let d_m = d_max_um * 1e-6;
    let t_s = (d_m / acceleration_m_s2).sqrt();
    (t_s * 1e6).ceil() as u64
}

/// Sum of √(d_max) over groups — [RAP] Eq. (1) (up to 1/√a).
pub fn routing_cost_eq1(group_d_max_um: &[f64]) -> f64 {
    group_d_max_um.iter().map(|&d| sqrt_d_max(d)).sum()
}

/// Euclidean distance between positions (µm).
pub fn euclidean_um(a: Position, b: Position) -> f64 {
    let dx = a.x_um - b.x_um;
    let dy = a.y_um - b.y_um;
    (dx * dx + dy * dy).sqrt()
}

/// Validate that every entangling action’s atoms sit in an entanglement zone
/// according to `layout` site positions, and occupancy ≤ capacity.
pub fn validate_zone_constraints(
    layers: &[ScheduleLayer],
    layout: &NeutralAtomLayout,
    arch: &ZonedArchitecture,
) -> Result<(), ZonedScheduleError> {
    arch.validate()?;
    let site_zone = site_to_zone(layout, arch);
    let atom_site: BTreeMap<AtomId, SiteId> = layout
        .initial_bindings
        .iter()
        .map(|b| {
            let site = match b.trap {
                TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
            };
            (b.atom, site)
        })
        .collect();

    for layer in layers {
        let mut occupancy: BTreeMap<u32, u32> = BTreeMap::new();
        for action in &layer.actions {
            match action {
                NeutralAtomAction::Entangle2 { atoms, .. } => {
                    for &atom in atoms {
                        check_entangle_zone(atom, &atom_site, &site_zone)?;
                    }
                }
                NeutralAtomAction::EntangleN { atoms, .. } => {
                    for &atom in atoms {
                        check_entangle_zone(atom, &atom_site, &site_zone)?;
                    }
                }
                NeutralAtomAction::Measure { atom, .. } => {
                    if arch.require_readout_zone {
                        let site = atom_site.get(atom).copied();
                        let zone = site.and_then(|s| site_zone.get(&s).copied());
                        if zone != Some(ZoneKind::Readout) {
                            return Err(ZonedScheduleError::MeasureOutsideReadout(*atom));
                        }
                    }
                }
                NeutralAtomAction::Move(group) => {
                    for m in &group.moves {
                        if let Some(kind) = site_zone.get(&m.to) {
                            *occupancy
                                .entry(zone_id_for_site(layout, arch, m.to))
                                .or_insert(0) += 1;
                            let _ = kind;
                        }
                    }
                }
                _ => {}
            }
        }
        for zone in &arch.zones {
            let used = occupancy.get(&zone.zone_id).copied().unwrap_or(0);
            let cap = zone.capacity_sites();
            if used > cap {
                return Err(ZonedScheduleError::OccupancyExceeded(
                    zone.zone_id,
                    used,
                    cap,
                ));
            }
        }
    }
    Ok(())
}

fn check_entangle_zone(
    atom: AtomId,
    atom_site: &BTreeMap<AtomId, SiteId>,
    site_zone: &BTreeMap<SiteId, ZoneKind>,
) -> Result<(), ZonedScheduleError> {
    let site = atom_site
        .get(&atom)
        .copied()
        .ok_or(ZonedScheduleError::EntangleOutsideZone(atom))?;
    match site_zone.get(&site) {
        Some(ZoneKind::Entanglement) => Ok(()),
        _ => Err(ZonedScheduleError::EntangleOutsideZone(atom)),
    }
}

fn site_to_zone(
    layout: &NeutralAtomLayout,
    arch: &ZonedArchitecture,
) -> BTreeMap<SiteId, ZoneKind> {
    let mut map = BTreeMap::new();
    for site in &layout.sites {
        if let Some(kind) = classify_site(site.position, arch) {
            map.insert(site.id, kind);
        }
    }
    map
}

fn classify_site(pos: Position, arch: &ZonedArchitecture) -> Option<ZoneKind> {
    for zone in &arch.zones {
        let (x0, y0, x1, y1) = (
            zone.origin_um.0,
            zone.origin_um.1,
            zone.origin_um.0 + f64::from(zone.cols) * zone.site_pitch_um.0,
            zone.origin_um.1 + f64::from(zone.rows) * zone.site_pitch_um.1,
        );
        if pos.x_um >= x0 && pos.x_um <= x1 && pos.y_um >= y0 && pos.y_um <= y1 {
            return Some(zone.kind);
        }
    }
    None
}

fn zone_id_for_site(layout: &NeutralAtomLayout, arch: &ZonedArchitecture, site: SiteId) -> u32 {
    let pos = layout
        .sites
        .iter()
        .find(|s| s.id == site)
        .map(|s| s.position);
    if let Some(pos) = pos {
        for zone in &arch.zones {
            let (x0, y0, x1, y1) = (
                zone.origin_um.0,
                zone.origin_um.1,
                zone.origin_um.0 + f64::from(zone.cols) * zone.site_pitch_um.0,
                zone.origin_um.1 + f64::from(zone.rows) * zone.site_pitch_um.1,
            );
            if pos.x_um >= x0 && pos.x_um <= x1 && pos.y_um >= y0 && pos.y_um <= y1 {
                return zone.zone_id;
            }
        }
    }
    0
}

/// Schedule a graph request onto a zoned architecture, using the default
/// [`AwareSearchParams`] ([RAP] Sec. VI-A QASMBench set) for
/// [`PlacerMode::RoutingAware`]. Use
/// [`schedule_zoned_with_aware_params`] to override the A* search's
/// tunables (node budget, deepening factor/value, pruning window).
///
/// Expects `req.layers` to already contain entangling layers (#105). Fills
/// movement/transfer actions around those entangles and updates `layout` to a
/// storage+entanglement site map. Soft-unblocked vs #106: uses RAP-local
/// greedy move grouping, not Enola sortIS.
pub fn schedule_zoned<V: VertexId>(
    req: GraphScheduleRequest<V>,
    arch: &ZonedArchitecture,
    mode: PlacerMode,
) -> Result<ZonedScheduleResult<V>, ZonedScheduleError> {
    schedule_zoned_with_aware_params(req, arch, mode, AwareSearchParams::default())
}

/// [`schedule_zoned`], but with explicit [`AwareSearchParams`] for
/// [`PlacerMode::RoutingAware`]'s A* search (issue #297). Ignored under
/// [`PlacerMode::RoutingAgnostic`].
pub fn schedule_zoned_with_aware_params<V: VertexId>(
    mut req: GraphScheduleRequest<V>,
    arch: &ZonedArchitecture,
    mode: PlacerMode,
    aware_search: AwareSearchParams,
) -> Result<ZonedScheduleResult<V>, ZonedScheduleError> {
    arch.validate()?;
    if req.layers.is_empty() {
        // Synthesize one layer per commutation/ ASAP is caller's job; allow
        // building from graph interactions as a single parallel set when empty.
        return Err(ZonedScheduleError::EmptySchedule);
    }

    // Reuse an existing layout when present (QEC hybrid: place→move per round
    // with continuous atom positions across rounds). Otherwise place fresh.
    let (layout, _storage_sites, entangle_pairs) = match req.layout.take() {
        Some(existing) => {
            let (fresh, storage, pairs) = build_zoned_layout(&req.graph, arch)?;
            let layout = NeutralAtomLayout {
                sites: fresh.sites,
                initial_bindings: existing.initial_bindings,
            };
            (layout, storage, pairs)
        }
        None => build_zoned_layout(&req.graph, arch)?,
    };
    req.layout = Some(layout.clone());

    let mut atom_pos: BTreeMap<AtomId, Position> = BTreeMap::new();
    let mut site_occupant: BTreeMap<SiteId, AtomId> = BTreeMap::new();
    for binding in &layout.initial_bindings {
        let site = match binding.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
        };
        if let Some(s) = layout.sites.iter().find(|s| s.id == site) {
            atom_pos.insert(binding.atom, s.position);
            site_occupant.insert(site, binding.atom);
        }
    }

    let pair_sites: Vec<(SiteId, SiteId)> = entangle_pairs
        .iter()
        .map(|&(left, right)| {
            (
                nearest_site_id(&layout, left),
                nearest_site_id(&layout, right),
            )
        })
        .collect();
    let conflict_um = arch.pair_conflict_um();

    let mut out_layers = Vec::new();
    let mut next_cycle = 0u32;
    let mut total_routing_cost = 0.0;
    let mut rearrangement_steps = 0u64;
    let mut trap_transfers = 0u64;
    let mut aware_search_completed_layers = 0u64;
    let mut aware_search_budget_exceeded_layers = 0u64;
    let mut aware_search_no_legal_assignment_layers = 0u64;
    let mut aware_search_node_expansions = 0u64;
    let mut agnostic_matching_layers = 0u64;
    let mut agnostic_greedy_fallback_layers = 0u64;

    let mut worklist: VecDeque<ScheduleLayer> = req.layers.iter().cloned().collect();
    while let Some(layer) = worklist.pop_front() {
        let gates = entangling_gate_actions(&layer);
        if gates.is_empty() {
            let mut passthrough = layer.clone();
            passthrough.cycle = next_cycle;
            out_layers.push(passthrough);
            next_cycle = next_cycle.saturating_add(1);
            continue;
        }
        if gates.len() > entangle_pairs.len() {
            return Err(ZonedScheduleError::InsufficientPairs(gates.len()));
        }

        let inputs = AssignInputs {
            atom_pos: &atom_pos,
            pairs: &entangle_pairs,
            pair_sites: &pair_sites,
            site_occupant: &site_occupant,
            conflict_um,
            aod_min_sep_um: arch.aod_min_separation_um,
        };
        let gate_atoms: Vec<(AtomId, AtomId)> = gates.iter().map(|g| g.atoms).collect();
        let assignment = match mode {
            PlacerMode::RoutingAgnostic | PlacerMode::Exact => {
                // Exact mode uses the same agnostic per-layer assignment as
                // RoutingAgnostic — the exact optimization is in the initial
                // placement (handled by the flat-AOD path or the caller),
                // not in per-layer gate-to-pair assignment. Issue #302.
                let (a, used_matching) = if gate_atoms.len() * entangle_pairs.len()
                    > MATCHING_FALLBACK_GATE_PAIR_PRODUCT
                {
                    (assign_greedy_legal(&gate_atoms, &inputs), false)
                } else {
                    pick_agnostic_assignment(
                        &gate_atoms,
                        &inputs,
                        &layout,
                        &atom_pos,
                        arch.aod_min_separation_um,
                    )
                };
                if used_matching {
                    agnostic_matching_layers += 1;
                } else {
                    agnostic_greedy_fallback_layers += 1;
                }
                a
            }
            PlacerMode::RoutingAware => assign_aware_legal(&gate_atoms, &inputs, &aware_search),
        };
        aware_search_node_expansions += assignment.node_expansions as u64;
        match assignment.outcome {
            AwareSearchOutcome::NotApplicable => {}
            AwareSearchOutcome::Completed => aware_search_completed_layers += 1,
            AwareSearchOutcome::BudgetExceeded => aware_search_budget_exceeded_layers += 1,
            AwareSearchOutcome::NoLegalAssignment => aware_search_no_legal_assignment_layers += 1,
        }
        if assignment.placed.is_empty() {
            let (a, b) = gate_atoms[*assignment.deferred.first().unwrap_or(&0)];
            return Err(ZonedScheduleError::NoLegalPair(a, b));
        }
        if !assignment.deferred.is_empty() {
            // Deferred gates (no occupancy- and spacing-legal pair this
            // stage) run in their own later stage — pushed to the worklist
            // front so they still precede every later original layer.
            let deferred_actions: Vec<NeutralAtomAction> = assignment
                .deferred
                .iter()
                .map(|&gate_index| gates[gate_index].action.clone())
                .collect();
            worklist.push_front(ScheduleLayer {
                cycle: 0, // renumbered when emitted
                actions: deferred_actions,
            });
        }

        // Reuse (Sec. III-A): skip atoms already at their assigned pair site.
        let mut planned_moves = Vec::new();
        for &(gate_index, (pa, pb)) in &assignment.placed {
            let (a, b) = gate_atoms[gate_index];
            for (atom, target) in [(a, pa), (b, pb)] {
                let cur = atom_pos.get(&atom).copied().unwrap_or(target);
                let dist = euclidean_um(cur, target);
                if dist < 1e-9 {
                    continue; // reuse: already in place
                }
                let from_site = nearest_site_id(&layout, cur);
                let to_site = nearest_site_id(&layout, target);
                planned_moves.push(PlannedMove {
                    atom,
                    from_site,
                    to_site,
                    from: cur,
                    to: target,
                    distance_um: dist,
                });
                atom_pos.insert(atom, target);
                if site_occupant.get(&from_site) == Some(&atom) {
                    site_occupant.remove(&from_site);
                }
                site_occupant.insert(to_site, atom);
            }
        }

        // AOD row/column coupling makes some move sets unrealizable as one
        // grab (e.g. storage- and zone-sourced atoms converging on the same
        // row). Partition into compatible groups — the greedily grouped
        // compatible movements [RAP] Eq. (1) sums over — and emit each as
        // its own load → move → store stage.
        for group in partition_aod_compatible(&planned_moves, arch.aod_min_separation_um) {
            let d_max = group.iter().fold(0.0_f64, |d, m| d.max(m.distance_um));
            total_routing_cost += sqrt_d_max(d_max);
            rearrangement_steps += 1;
            let duration_us = movement_duration_us(d_max, arch.acceleration_m_s2);
            trap_transfers += 2 * group.len() as u64;

            let load: Vec<_> = group
                .iter()
                .map(|m| {
                    NeutralAtomAction::Transfer(TrapTransfer {
                        atom: m.atom,
                        direction: TransferDirection::SlmToAod,
                        site: m.from_site,
                        aod: AodTrapRef {
                            aod_id: 0,
                            row: 0,
                            col: 0,
                        },
                        duration_us: arch.trap_transfer_us,
                    })
                })
                .collect();
            let moves: Vec<_> = group
                .iter()
                .map(|m| AtomMove {
                    atom: m.atom,
                    from: m.from_site,
                    to: m.to_site,
                })
                .collect();
            let store: Vec<_> = group
                .iter()
                .map(|m| {
                    NeutralAtomAction::Transfer(TrapTransfer {
                        atom: m.atom,
                        direction: TransferDirection::AodToSlm,
                        site: m.to_site,
                        aod: AodTrapRef {
                            aod_id: 0,
                            row: 0,
                            col: 0,
                        },
                        duration_us: arch.trap_transfer_us,
                    })
                })
                .collect();

            push_validated_layer(&mut out_layers, &mut next_cycle, load)?;
            push_validated_layer(
                &mut out_layers,
                &mut next_cycle,
                vec![NeutralAtomAction::Move(MovementGroup {
                    moves,
                    duration_us,
                })],
            )?;
            push_validated_layer(&mut out_layers, &mut next_cycle, store)?;
        }

        // Entangle layer (atoms now in entanglement zone). Gates are emitted
        // as pairwise Entangle2 (same rewrite as before deferral existed).
        let mut entangle_actions: Vec<NeutralAtomAction> = assignment
            .placed
            .iter()
            .map(|&(gate_index, _)| NeutralAtomAction::Entangle2 {
                atoms: [gate_atoms[gate_index].0, gate_atoms[gate_index].1],
                duration_us: 1,
            })
            .collect();
        // Also pass through non-entangle actions from the original layer.
        for action in &layer.actions {
            if !matches!(
                action,
                NeutralAtomAction::Entangle2 { .. } | NeutralAtomAction::EntangleN { .. }
            ) {
                entangle_actions.push(action.clone());
            }
        }
        let entangle_layer = ScheduleLayer {
            cycle: next_cycle,
            actions: entangle_actions,
        };
        entangle_layer
            .validate_conflicts()
            .map_err(|e| ZonedScheduleError::Conflict(e.to_string()))?;
        entangle_layer
            .validate_occupancy()
            .map_err(|e| ZonedScheduleError::Conflict(e.to_string()))?;
        out_layers.push(entangle_layer);
        next_cycle = next_cycle.saturating_add(1);
    }

    // Update layout bindings to final atom positions (entanglement or storage).
    let mut final_layout = layout;
    let site_updates: Vec<(AtomId, SiteId)> = final_layout
        .initial_bindings
        .iter()
        .filter_map(|binding| {
            atom_pos
                .get(&binding.atom)
                .map(|pos| (binding.atom, nearest_site_id(&final_layout, *pos)))
        })
        .collect();
    for binding in &mut final_layout.initial_bindings {
        if let Some((_, site)) = site_updates.iter().find(|(a, _)| *a == binding.atom) {
            binding.trap = TrapBinding::Slm { site: *site };
        }
    }
    let validate_layout = layout_with_atoms_at(&final_layout, &atom_pos);
    validate_zone_constraints(&out_layers, &validate_layout, arch)?;

    req.layers = out_layers;
    req.layout = Some(final_layout);

    Ok(ZonedScheduleResult {
        request: req,
        mode,
        routing_cost: total_routing_cost,
        rearrangement_steps,
        trap_transfers,
        aware_search_completed_layers,
        aware_search_budget_exceeded_layers,
        aware_search_no_legal_assignment_layers,
        aware_search_node_expansions,
        agnostic_matching_layers,
        agnostic_greedy_fallback_layers,
    })
}

fn layout_with_atoms_at(
    base: &NeutralAtomLayout,
    atom_pos: &BTreeMap<AtomId, Position>,
) -> NeutralAtomLayout {
    let mut layout = base.clone();
    let updates: Vec<(AtomId, SiteId)> = layout
        .initial_bindings
        .iter()
        .filter_map(|binding| {
            atom_pos
                .get(&binding.atom)
                .map(|pos| (binding.atom, nearest_site_id(&layout, *pos)))
        })
        .collect();
    for binding in &mut layout.initial_bindings {
        if let Some((_, site)) = updates.iter().find(|(a, _)| *a == binding.atom) {
            binding.trap = TrapBinding::Slm { site: *site };
        }
    }
    layout
}

/// One entangling action of an input layer plus the pair the placer assigns.
struct LayerGate {
    /// Original action, kept verbatim for deferral to a later stage.
    action: NeutralAtomAction,
    atoms: (AtomId, AtomId),
}

fn entangling_gate_actions(layer: &ScheduleLayer) -> Vec<LayerGate> {
    let mut out = Vec::new();
    for action in &layer.actions {
        match action {
            NeutralAtomAction::Entangle2 { atoms, .. } => out.push(LayerGate {
                action: action.clone(),
                atoms: (atoms[0], atoms[1]),
            }),
            NeutralAtomAction::EntangleN { atoms, .. } if atoms.len() >= 2 => {
                out.push(LayerGate {
                    action: action.clone(),
                    atoms: (atoms[0], atoms[1]),
                });
            }
            _ => {}
        }
    }
    out
}

/// One atom displacement the placer decided on, with stage coordinates.
struct PlannedMove {
    atom: AtomId,
    from_site: SiteId,
    to_site: SiteId,
    from: Position,
    to: Position,
    distance_um: f64,
}

/// Partition planned moves into AOD-coupled-motion-compatible groups
/// (first-fit, in placement order — deterministic).
///
/// Under the dense source-coordinate overlay (B4 / `quon_na::lower`), two
/// moves can share one AOD grab iff, per axis: equal source coordinates imply
/// equal destination coordinates (row/column coupling), distinct source
/// coordinates keep their strict order at the destination (order
/// preservation), and distinct rows/columns end at least `min_sep_um` apart.
fn partition_aod_compatible(moves: &[PlannedMove], min_sep_um: f64) -> Vec<Vec<&PlannedMove>> {
    let mut groups: Vec<Vec<&PlannedMove>> = Vec::new();
    for planned in moves {
        let slot = groups.iter_mut().find(|group| {
            group
                .iter()
                .all(|member| moves_aod_compatible(member, planned, min_sep_um))
        });
        match slot {
            Some(group) => group.push(planned),
            None => groups.push(vec![planned]),
        }
    }
    groups
}

fn moves_aod_compatible(a: &PlannedMove, b: &PlannedMove, min_sep_um: f64) -> bool {
    positions_aod_compatible((a.from, a.to), (b.from, b.to), min_sep_um)
}

/// [RAP] Sec. V-A's movement-group compatibility check (non-crossing +
/// preservation), on raw `(from, to)` position pairs rather than
/// [`PlannedMove`] — shared by the post-search routing grouper
/// ([`moves_aod_compatible`]) and the search-time grouping
/// [`assign_aware_legal`] performs to keep its Eq. (1) cost consistent with
/// what routing will actually emit (#297). The paper implements this with a
/// binary search tree per group for O(log n) lookups (Sec. V-A); this is a
/// linear scan against existing group members instead — legal at this
/// crate's per-layer group sizes, a data-structure simplification, not a
/// semantics one.
fn positions_aod_compatible(
    a: (Position, Position),
    b: (Position, Position),
    min_sep_um: f64,
) -> bool {
    axis_aod_compatible(a.0.y_um, a.1.y_um, b.0.y_um, b.1.y_um, min_sep_um)
        && axis_aod_compatible(a.0.x_um, a.1.x_um, b.0.x_um, b.1.x_um, min_sep_um)
}

fn axis_aod_compatible(from_a: f64, to_a: f64, from_b: f64, to_b: f64, min_sep_um: f64) -> bool {
    match from_a.total_cmp(&from_b) {
        // Same AOD row/column: coupled motion forces one displacement.
        Ordering::Equal => to_a.total_cmp(&to_b) == Ordering::Equal,
        // Distinct rows/columns: preserve order, keep destination separation.
        order => to_a.total_cmp(&to_b) == order && (to_a - to_b).abs() >= min_sep_um,
    }
}

/// Emit one movement-stage layer at the next cycle after software validation
/// (occupancy: no same-cycle duplicate atom/site claims). Planner bugs fail
/// loudly here rather than surfacing at MLIR emit.
fn push_validated_layer(
    out_layers: &mut Vec<ScheduleLayer>,
    next_cycle: &mut u32,
    actions: Vec<NeutralAtomAction>,
) -> Result<(), ZonedScheduleError> {
    let layer = ScheduleLayer {
        cycle: *next_cycle,
        actions,
    };
    layer
        .validate_occupancy()
        .map_err(|e| ZonedScheduleError::Conflict(e.to_string()))?;
    out_layers.push(layer);
    *next_cycle = next_cycle.saturating_add(1);
    Ok(())
}

type EntanglePair = (Position, Position);
type ZonedLayoutParts = (NeutralAtomLayout, Vec<Position>, Vec<EntanglePair>);

fn build_zoned_layout<V: VertexId>(
    graph: &InteractionGraph<V>,
    arch: &ZonedArchitecture,
) -> Result<ZonedLayoutParts, ZonedScheduleError> {
    let mut sites = Vec::new();
    let mut next_site = 0u32;
    let mut storage_sites = Vec::new();
    let mut entangle_pairs = Vec::new();

    for zone in &arch.zones {
        match zone.kind {
            ZoneKind::Storage | ZoneKind::Readout => {
                for r in 0..zone.rows {
                    for c in 0..zone.cols {
                        let pos = zone.site_position(r, c);
                        sites.push(AtomSite {
                            id: SiteId(next_site),
                            position: pos,
                        });
                        if zone.kind == ZoneKind::Storage {
                            storage_sites.push(pos);
                        }
                        next_site += 1;
                    }
                }
            }
            ZoneKind::Entanglement => {
                let gap = zone.pair_gap_um.unwrap_or(2.0);
                for r in 0..zone.rows {
                    for c in 0..zone.cols {
                        let left = Position {
                            x_um: zone.origin_um.0 + f64::from(c) * zone.site_pitch_um.0,
                            y_um: zone.origin_um.1 + f64::from(r) * zone.site_pitch_um.1,
                        };
                        let right = Position {
                            x_um: left.x_um + gap,
                            y_um: left.y_um,
                        };
                        sites.push(AtomSite {
                            id: SiteId(next_site),
                            position: left,
                        });
                        next_site += 1;
                        sites.push(AtomSite {
                            id: SiteId(next_site),
                            position: right,
                        });
                        next_site += 1;
                        entangle_pairs.push((left, right));
                    }
                }
            }
        }
    }

    if storage_sites.is_empty() {
        return Err(ZonedScheduleError::MissingStorageZone);
    }
    if entangle_pairs.is_empty() {
        return Err(ZonedScheduleError::MissingEntanglementZone);
    }

    // Initial placement: row-major into storage.
    let mut bindings = Vec::new();
    for (i, &q) in graph.vertices.iter().enumerate() {
        let pos = storage_sites[i % storage_sites.len()];
        let site = nearest_site_id_in(&sites, pos);
        bindings.push(AtomBinding {
            atom: AtomId(q.index()),
            trap: TrapBinding::Slm { site },
        });
    }

    Ok((
        NeutralAtomLayout {
            sites,
            initial_bindings: bindings,
        },
        storage_sites,
        entangle_pairs,
    ))
}

fn nearest_site_id(layout: &NeutralAtomLayout, pos: Position) -> SiteId {
    nearest_site_id_in(&layout.sites, pos)
}

fn nearest_site_id_in(sites: &[AtomSite], pos: Position) -> SiteId {
    sites
        .iter()
        .min_by(|a, b| {
            euclidean_um(a.position, pos)
                .partial_cmp(&euclidean_um(b.position, pos))
                .unwrap_or(Ordering::Equal)
        })
        .map(|s| s.id)
        .unwrap_or(SiteId(0))
}

/// Everything a layer's pair assignment needs besides the gates themselves.
struct AssignInputs<'a> {
    atom_pos: &'a BTreeMap<AtomId, Position>,
    pairs: &'a [(Position, Position)],
    pair_sites: &'a [(SiteId, SiteId)],
    /// Site occupancy at the **start** of the layer (parked atoms included).
    site_occupant: &'a BTreeMap<SiteId, AtomId>,
    /// Minimum legal distance between atoms of different simultaneous pairs
    /// ([`ZonedArchitecture::pair_conflict_um`]); `0.0` disables the check.
    conflict_um: f64,
    /// Minimum AOD row/column separation ([`ZonedArchitecture::aod_min_separation_um`],
    /// here `aod_min_separation_um` field) used only by
    /// [`assign_aware_legal`]'s search-time movement grouping (Eq. (1)); `0.0`
    /// disables the separation sub-check (see [`positions_aod_compatible`]).
    aod_min_sep_um: f64,
}

/// Whether [`assign_aware_legal`]'s A* search found a full legal assignment
/// for a layer, or gave up and fell back to [`assign_greedy_legal`] (issue
/// #111 review finding: a silent fallback here is indistinguishable from "no
/// routing contention" unless it is surfaced).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AwareSearchOutcome {
    /// [`assign_greedy_legal`] was called directly (routing-agnostic mode);
    /// the aware-search-completion concept doesn't apply.
    NotApplicable,
    /// The search popped a full-assignment goal node within
    /// [`AwareSearchParams::node_budget`] expansions. As of #297 the search
    /// is heuristic-guided with an intentionally inadmissible accelerating
    /// term (Eqs. (3)-(4)), so this is the search's best find for the layer,
    /// not a proven joint optimum — see the module doc.
    Completed,
    /// The search exhausted [`AwareSearchParams::node_budget`] expansions
    /// before popping a full-assignment goal node and fell back to
    /// [`assign_greedy_legal`].
    BudgetExceeded,
    /// The search exhausted its reachable space (heap emptied) within the
    /// current [`AwareSearchParams::pruning_window`]/[`AwareSearchParams::beam_width`]
    /// bounds without popping a full-assignment goal node, and fell back to
    /// [`assign_greedy_legal`]. Pre-#297 (unwindowed, unbeamed uniform-cost
    /// search) this proved no full legal assignment existed at all (e.g.
    /// spacing/occupancy conflicts); as of #297 it does **not** prove that —
    /// `pruning_window` can exclude the only choice a full assignment
    /// requires for some gate (see the module doc), so this outcome can also
    /// mean "no full assignment within the windowed/beamed search space,"
    /// not "no full assignment exists."
    NoLegalAssignment,
}

/// Result of a layer's pair assignment: gates placed this stage (with their
/// oriented pair positions) and gates deferred to a follow-up stage.
struct GateAssignment {
    placed: Vec<(usize, (Position, Position))>,
    deferred: Vec<usize>,
    /// See [`AwareSearchOutcome`]. [`assign_greedy_legal`] always reports
    /// [`AwareSearchOutcome::NotApplicable`] unless it was called as a
    /// fallback from [`assign_aware_legal`], in which case the caller
    /// overwrites this with the specific fallback reason.
    outcome: AwareSearchOutcome,
    /// Best-first search node expansions this call performed (issue #307).
    /// `0` from [`assign_greedy_legal`] unless it is a fallback from
    /// [`assign_aware_legal`], in which case the caller overwrites this with
    /// the aware search's expansion count before falling back.
    node_expansions: usize,
}

fn atom_position_or_origin(atom_pos: &BTreeMap<AtomId, Position>, atom: AtomId) -> Position {
    atom_pos.get(&atom).copied().unwrap_or(Position {
        x_um: 0.0,
        y_um: 0.0,
    })
}

/// True when two simultaneous pairs would put non-partner atoms within
/// `conflict_um` of each other (compulsory entanglement / R3 crosstalk).
fn pairs_conflict(lhs: (Position, Position), rhs: (Position, Position), conflict_um: f64) -> bool {
    if conflict_um <= 0.0 {
        return false;
    }
    [lhs.0, lhs.1].iter().any(|a| {
        [rhs.0, rhs.1]
            .iter()
            .any(|b| euclidean_um(*a, *b) <= conflict_um)
    })
}

/// A pair is occupancy-legal for gate `(a, b)` iff each of its sites is free
/// or already holds one of the gate's own atoms (reuse). Parked atoms from
/// earlier layers block the pair — assigning it anyway is how two atoms end
/// up claiming one site (the qft_small / qaoa_graph double-occupancy bug).
fn pair_occupancy_ok(
    sites: (SiteId, SiteId),
    gate: (AtomId, AtomId),
    site_occupant: &BTreeMap<SiteId, AtomId>,
) -> bool {
    [sites.0, sites.1].iter().all(|site| {
        site_occupant
            .get(site)
            .is_none_or(|&occupant| occupant == gate.0 || occupant == gate.1)
    })
}

fn pair_legal(
    index: usize,
    gate: (AtomId, AtomId),
    chosen: &BTreeSet<usize>,
    inputs: &AssignInputs<'_>,
) -> bool {
    pair_occupancy_ok(inputs.pair_sites[index], gate, inputs.site_occupant)
        && !chosen
            .iter()
            .any(|&j| pairs_conflict(inputs.pairs[index], inputs.pairs[j], inputs.conflict_um))
}

/// Routing-agnostic: assign gates to the nearest **legal** free pair by
/// travel distance (ZAC-style). Gates with no occupancy- and spacing-legal
/// pair are deferred to a follow-up stage.
fn assign_greedy_legal(gates: &[(AtomId, AtomId)], inputs: &AssignInputs<'_>) -> GateAssignment {
    let mut used: BTreeSet<usize> = BTreeSet::new();
    let mut placed = Vec::with_capacity(gates.len());
    let mut deferred = Vec::new();
    for (gate_index, &(a, b)) in gates.iter().enumerate() {
        let pa = atom_position_or_origin(inputs.atom_pos, a);
        let pb = atom_position_or_origin(inputs.atom_pos, b);
        let mut best: Option<(f64, usize, (Position, Position))> = None;
        for (i, &(left, right)) in inputs.pairs.iter().enumerate() {
            if used.contains(&i) || !pair_legal(i, (a, b), &used, inputs) {
                continue;
            }
            let cost_fwd = euclidean_um(pa, left) + euclidean_um(pb, right);
            let cost_rev = euclidean_um(pa, right) + euclidean_um(pb, left);
            let (cost, orient) = if cost_fwd <= cost_rev {
                (cost_fwd, (left, right))
            } else {
                (cost_rev, (right, left))
            };
            match best {
                None => best = Some((cost, i, orient)),
                Some((c, _, _)) if cost < c => best = Some((cost, i, orient)),
                _ => {}
            }
        }
        match best {
            Some((_, idx, orient)) => {
                used.insert(idx);
                placed.push((gate_index, orient));
            }
            None => deferred.push(gate_index),
        }
    }
    GateAssignment {
        placed,
        deferred,
        outcome: AwareSearchOutcome::NotApplicable,
        node_expansions: 0,
    }
}

/// For one gate, one occupancy-legal entanglement pair: `(pair_index, d_max
/// cost, oriented destination positions)`. Built per gate by
/// [`assign_matching_legal`] for the cost matrix and orientation lookup.
type LegalPairOption = (usize, f64, (Position, Position));

/// Routing-agnostic placement via min-weight bipartite matching (issue #300):
/// the Lin et al. 2025 `VertexMatchingPlacer` parity target. Solves a
/// minimum-total-travel assignment of gates to distinct entanglement pairs
/// (respecting occupancy legality), then repairs the pairwise spacing conflicts
/// the unconstrained travel-optimum incurs, keeping matching's choices where
/// they are legal and re-finding legal pairs only for the conflicting tail.
///
/// # Cost model
///
/// The per-`(gate, pair)` cost is the √-law per-gate move distance the agnostic
/// path charges ([RAP] Eq. (1)): `d_max = max(euclidean(pa, left),
/// euclidean(pb, right))` for the cheaper of the two pair orientations
/// (orientation does not affect [`pairs_conflict`], which is between *pair
/// sites*, so the cheaper orientation is picked freely per matched pair).
/// Occupancy-illegal ([`pair_occupancy_ok`] false) edges are
/// [`FORBIDDEN_COST`]. The matching minimizes the *total* per-gate `d_max` over
/// the layer, so it dominates greedy's myopic first-fit on contended layers
/// (the crossing-pairs unit test).
///
/// # Spacing conflicts are not a bipartite problem
///
/// [`pairs_conflict`] depends on the *set* of chosen pairs, not on any single
/// `(gate, pair)` edge, so it cannot be encoded in the cost matrix. This is the
/// task's "solve matching, then check `pairs_conflict` among the chosen set; on
/// conflict, exclude the conflicting pairs": after matching, this accepts the
/// travel-optimal choices in increasing-cost order, skipping any whose pair is
/// already taken or which [`pairs_conflict`]s with an already-accepted pair,
/// then greedily re-finds a legal pair (via the same [`pair_legal`] oracle
/// [`assign_greedy_legal`] uses, seeded with the accepted set) for every gate
/// whose matching choice was rejected. Legality is therefore never violated:
/// accepted pairs are conflict-free by construction and repaired pairs are
/// checked against the accepted set.
///
/// Note: the min-travel optimum tends to *pack* gates into nearby (dense) pairs,
/// which on a densely-conflicting entanglement zone (e.g. ising_n42, where the
/// 18.75 µm isolation spacing exceeds the 12 µm pair pitch) groups into *more*
/// AOD movement stages than the spread-out greedy choice. The dispatch
/// ([`pick_agnostic_assignment`]) therefore compares the two on the actual
/// rearrangement-step metric and keeps the better — guaranteeing the agnostic
/// path never regresses the step count while still using matching wherever it
/// does group better.
///
/// # Reuse (Sec. III-A)
///
/// Reuse — atoms already at their target pair site staying put — is captured
/// *by construction*: an atom whose current position equals a pair site has
/// zero travel to that pair, so the matching scores that pair at zero cost and
/// preferentially keeps the atom there. There is no separate first-fit reuse
/// stage; the per-layer matching *is* the reuse decision, made optimally
/// rather than greedily.
///
/// Returns the [`GateAssignment`] and a flag: `true` if matching produced it,
/// `false` if it fell back to [`assign_greedy_legal`]. (The current
/// implementation always returns `true`; the flag is retained for the
/// dispatch's group-count comparison and any future conflict-repair variant
/// that does fall back.)
fn assign_matching_legal(
    gates: &[(AtomId, AtomId)],
    inputs: &AssignInputs<'_>,
) -> (GateAssignment, bool) {
    let n_pairs = inputs.pairs.len();

    // 1. Per gate, precompute the (pair_index, cost, orientation) of each
    //    occupancy-legal pair, so the cost matrix only carries finite entries
    //    where a gate may actually go (avoids all-forbidden rows, which would
    //    inflate the Kuhn–Munkres potentials needlessly). Gates with no legal
    //    pair are deferred immediately and excluded from the matching.
    let mut legal_for: Vec<Vec<LegalPairOption>> = Vec::with_capacity(gates.len());
    let mut immediate_deferred: Vec<usize> = Vec::new();
    let mut active_gates: Vec<usize> = Vec::new();
    for (gate_index, &(a, b)) in gates.iter().enumerate() {
        let pa = atom_position_or_origin(inputs.atom_pos, a);
        let pb = atom_position_or_origin(inputs.atom_pos, b);
        let mut legal: Vec<LegalPairOption> = Vec::new();
        for (i, &(left, right)) in inputs.pairs.iter().enumerate() {
            if !pair_occupancy_ok(inputs.pair_sites[i], (a, b), inputs.site_occupant) {
                continue;
            }
            // Cost = the √-law per-gate move distance the agnostic path
            // charges ([RAP] Eq. (1)): d_max = max of the two atoms' travels
            // for the cheaper orientation (orientation does not affect
            // [`pairs_conflict`], which is between pair sites, so the cheaper
            // orientation is picked freely). Minimizing the sum of per-gate
            // d_max (rather than the sum of both atoms' distances, which the
            // greedy selection uses) tracks the routing cost Eq. (1) sums
            // over, and so correlates with — rather than fights — the
            // rearrangement-step count.
            let cost_fwd = euclidean_um(pa, left).max(euclidean_um(pb, right));
            let cost_rev = euclidean_um(pa, right).max(euclidean_um(pb, left));
            let (cost, orient) = if cost_fwd <= cost_rev {
                (cost_fwd, (left, right))
            } else {
                (cost_rev, (right, left))
            };
            legal.push((i, cost, orient));
        }
        if legal.is_empty() {
            immediate_deferred.push(gate_index);
        } else {
            active_gates.push(gate_index);
            legal_for.push(legal);
        }
    }

    let n_active = active_gates.len();
    if n_active == 0 {
        // No gate has any legal pair; nothing to match. (The caller will then
        // surface this as `NoLegalPair`, exactly as greedy would.) Matching
        // ran and answered "defer all" — this is not a greedy fallback.
        immediate_deferred.sort_unstable();
        return (
            GateAssignment {
                placed: Vec::new(),
                deferred: immediate_deferred,
                outcome: AwareSearchOutcome::NotApplicable,
                node_expansions: 0,
            },
            true,
        );
    }

    // 2. Solve the min-travel matching once over occupancy-legal edges only
    //    (pairwise spacing conflicts are repaired below). The cost matrix is
    //    [n_active × n_pairs] with [`FORBIDDEN_COST`] on occupancy-illegal
    //    edges; the matching minimizes total travel, so it dominates greedy's
    //    myopic first-fit wherever gates contend for the same nearest pair.
    let mut cost: Vec<Vec<f64>> = vec![vec![FORBIDDEN_COST; n_pairs]; n_active];
    for (a, legal) in legal_for.iter().enumerate() {
        for &(p, c, _) in legal {
            cost[a][p] = c;
        }
    }
    let assign = min_weight_assignment(&cost);

    // Each active gate's matched (pair, orientation), dropping gates the
    // matching could only give a forbidden (no-legal-pair) column. Sorted by
    // travel cost so the cheapest, best-matched gates keep their matching
    // choice and only the conflicting tail is repaired.
    let mut proposals: Vec<(usize, usize, (Position, Position), f64)> =
        Vec::with_capacity(n_active);
    for a in 0..n_active {
        let p = assign[a];
        if let Some((_, c, orient)) = legal_for[a].iter().find(|(pp, _, _)| *pp == p) {
            proposals.push((active_gates[a], p, *orient, *c));
        }
    }
    proposals.sort_by(|x, y| x.3.partial_cmp(&y.3).unwrap_or(Ordering::Equal));

    // 3. Conflict repair (the task's "solve matching, then check `pairs_conflict`
    //    among the chosen set; on conflict, exclude the conflicting pairs"):
    //    accept matching choices in increasing-travel order, skipping any whose
    //    pair is already taken or which [`pairs_conflict`]s with an
    //    already-accepted pair. Accepted gates keep their travel-optimal
    //    matching assignment; only the conflicting tail is repaired. A gate
    //    whose matching choice is rejected is re-found a legal pair below
    //    (step 4) — so legality is never violated: accepted pairs are
    //    conflict-free by construction, and repaired pairs are checked against
    //    the accepted set via [`pair_legal`].
    let mut used: BTreeSet<usize> = BTreeSet::new();
    let mut accepted_pairs: Vec<usize> = Vec::with_capacity(n_active);
    let mut accepted_gates: BTreeSet<usize> = BTreeSet::new();
    let mut placed: Vec<(usize, (Position, Position))> = Vec::with_capacity(n_active);
    for (gate_index, p, orient, _) in &proposals {
        let ok = !used.contains(p)
            && accepted_pairs
                .iter()
                .all(|&q| !pairs_conflict(inputs.pairs[*p], inputs.pairs[q], inputs.conflict_um));
        if ok {
            used.insert(*p);
            accepted_pairs.push(*p);
            accepted_gates.insert(*gate_index);
            placed.push((*gate_index, *orient));
        }
    }

    // 4. Greedily reassign every active gate whose matching choice was rejected
    //    (taken or conflicting) to its nearest legal free pair that does not
    //    conflict with the accepted set — the same legality oracle
    //    [`assign_greedy_legal`] uses ([`pair_legal`]), seeded with the
    //    matching's accepted pairs. Gates with no such pair are deferred
    //    (exactly as greedy would). This keeps matching's gain for the
    //    non-conflicting bulk of the layer and falls back to greedy only for
    //    the gates the matching could not legally place.
    let mut deferred: Vec<usize> = immediate_deferred.clone();
    for &gate_index in &active_gates {
        if accepted_gates.contains(&gate_index) {
            continue;
        }
        let (a, b) = gates[gate_index];
        let pa = atom_position_or_origin(inputs.atom_pos, a);
        let pb = atom_position_or_origin(inputs.atom_pos, b);
        let mut best: Option<(f64, usize, (Position, Position))> = None;
        for (i, &(left, right)) in inputs.pairs.iter().enumerate() {
            if used.contains(&i) || !pair_legal(i, (a, b), &used, inputs) {
                continue;
            }
            let cost_fwd = euclidean_um(pa, left).max(euclidean_um(pb, right));
            let cost_rev = euclidean_um(pa, right).max(euclidean_um(pb, left));
            let (c, orient) = if cost_fwd <= cost_rev {
                (cost_fwd, (left, right))
            } else {
                (cost_rev, (right, left))
            };
            match best {
                None => best = Some((c, i, orient)),
                Some((bc, _, _)) if c < bc => best = Some((c, i, orient)),
                _ => {}
            }
        }
        match best {
            Some((_, i, orient)) => {
                used.insert(i);
                placed.push((gate_index, orient));
            }
            None => deferred.push(gate_index),
        }
    }

    placed.sort_by_key(|(g, _)| *g);
    deferred.sort_unstable();
    deferred.dedup();
    (
        GateAssignment {
            placed,
            deferred,
            outcome: AwareSearchOutcome::NotApplicable,
            node_expansions: 0,
        },
        true,
    )
}

/// Number of AOD-coupled-motion-compatible movement groups an assignment would
/// emit — the actual per-layer rearrangement-step contribution ([RAP] Eq.
/// (1) sums `√(d_max)` over exactly these groups). Used by
/// [`pick_agnostic_assignment`] to compare the matching and greedy placers on
/// the metric that matters (steps), not just travel distance.
fn assignment_group_count(
    assignment: &GateAssignment,
    gates: &[(AtomId, AtomId)],
    atom_pos: &BTreeMap<AtomId, Position>,
    layout: &NeutralAtomLayout,
    aod_min_sep_um: f64,
) -> usize {
    let mut moves: Vec<PlannedMove> = Vec::new();
    for &(gate_index, (pa, pb)) in &assignment.placed {
        let (a, b) = gates[gate_index];
        for (atom, target) in [(a, pa), (b, pb)] {
            let cur = atom_pos.get(&atom).copied().unwrap_or(target);
            let dist = euclidean_um(cur, target);
            if dist < 1e-9 {
                continue; // reuse: already in place, no move
            }
            moves.push(PlannedMove {
                atom,
                from_site: nearest_site_id(layout, cur),
                to_site: nearest_site_id(layout, target),
                from: cur,
                to: target,
                distance_um: dist,
            });
        }
    }
    partition_aod_compatible(&moves, aod_min_sep_um).len()
}

/// Choose the routing-agnostic layer assignment: compute both the
/// min-weight matching placer ([`assign_matching_legal`]) and the greedy
/// placer ([`assign_greedy_legal`]), keep whichever yields fewer AOD movement
/// groups (the rearrangement-step metric) — with greedy winning ties and any
/// case where matching defers more gates. This guarantees the agnostic path
/// never produces more rearrangement steps than the greedy baseline (issue
/// #300 acceptance: matching ≤ greedy steps), while still using matching
/// wherever it genuinely groups better (low-contention / conflict-free
/// layers; the crossing-pairs unit test). Returns the assignment and whether
/// matching produced it (`true`) or greedy did (`false`).
fn pick_agnostic_assignment(
    gates: &[(AtomId, AtomId)],
    inputs: &AssignInputs<'_>,
    layout: &NeutralAtomLayout,
    atom_pos: &BTreeMap<AtomId, Position>,
    aod_min_sep_um: f64,
) -> (GateAssignment, bool) {
    let greedy = assign_greedy_legal(gates, inputs);
    let (matching, _) = assign_matching_legal(gates, inputs);
    let greedy_groups = assignment_group_count(&greedy, gates, atom_pos, layout, aod_min_sep_um);
    let matching_groups =
        assignment_group_count(&matching, gates, atom_pos, layout, aod_min_sep_um);
    // Matching must place at least as many gates (≤ deferrals) AND strictly
    // fewer movement groups to be worth the swap; otherwise the known-good
    // greedy baseline is kept.
    if matching.deferred.len() <= greedy.deferred.len() && matching_groups < greedy_groups {
        (matching, true)
    } else {
        (greedy, false)
    }
}

/// Tunable parameters for [`assign_aware_legal`]'s A* search ([RAP] Secs.
/// IV-C / V-C / V-D, Eqs. (3)-(5)). Defaults are the paper's QASMBench
/// parameter set (Sec. VI-A: α=0.2, β=0.2, γ=5, δ=0.6) restricted to the
/// terms this port implements — see the module doc for why α (Eq. (2)'s
/// look-ahead weight) and γ (Eq. (2)'s reuse-cost offset) have no field
/// here. `ising_n42` (the #297 anchor fixture, `docs/neutral_atom/rap_table_i_methodology.md`)
/// is itself one of the paper's QASMBench benchmarks (Table I's `ising2`
/// row), so this is also the qmap-comparable default for that fixture, not
/// an arbitrary pick.
///
/// Exposed via [`crate::pipeline::NaScheduleOptions::aware_search`] (a
/// per-run option, not a target/hardware JSON field) — matching qmap's own
/// shape, where the analogous `Config` is passed to the placer per call, not
/// baked into the architecture description.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AwareSearchParams {
    /// δ (Eq. (4)): scales the accelerating (spacing-preservation) term.
    /// qmap: `deepeningFactor`.
    pub deepening_factor: f64,
    /// β (Eq. (4)): additive floor inside the accelerating term, so a
    /// perfectly uniform (SD = 0) partial placement still contributes a
    /// small nonzero penalty proportional to how many gates remain. qmap:
    /// `deepeningValue`.
    pub deepening_value: f64,
    /// Expansion budget before falling back to [`assign_greedy_legal`].
    pub node_budget: usize,
    /// [RAP] Sec. V-D pruning: number of nearest *legal* entanglement pairs
    /// considered per gate at each node expansion (bounds branching factor
    /// on layers with many simultaneous gates / candidate pairs).
    pub pruning_window: usize,
    /// Beam width: the frontier is trimmed back to the best `beam_width`
    /// nodes (by priority) whenever it grows past `2 * beam_width`. Not a
    /// term from the paper's Eqs. (3)-(5) — an engineering addition this
    /// port needed (#297 review finding) because a plain priority queue's
    /// frontier, at real fixture scale (20+ simultaneous gates, hundreds of
    /// candidate pairs, tens of them mutually conflicting), fills with
    /// shallow (early-gate) alternatives faster than deep ones ever get
    /// explored — without a cap, `node_budget` expansions are spent almost
    /// entirely widening rather than deepening the search, and it never
    /// reaches a full assignment. See the module doc.
    pub beam_width: usize,
}

impl Default for AwareSearchParams {
    fn default() -> Self {
        Self {
            deepening_factor: 0.6, // δ, [RAP] Sec. VI-A QASMBench set
            deepening_value: 0.2,  // β, [RAP] Sec. VI-A QASMBench set
            node_budget: AWARE_NODE_BUDGET,
            pruning_window: 32,
            beam_width: 2_000,
        }
    }
}

/// One AOD-compatible movement group forming during the search — mirrors
/// [`partition_aod_compatible`]'s post-search grouping so a node's Eq. (1)
/// cost matches what routing will actually charge for it (#297; previously
/// the search charged each gate's own `d_max` independently, which did not
/// account for grouping at all).
#[derive(Clone)]
struct SearchGroup {
    /// `(from, to)` per atom placed into this group so far — used only for
    /// the [`positions_aod_compatible`] check against new candidate moves
    /// and for [`sum_std_deviation_for_groups`].
    moves: Vec<(Position, Position)>,
    max_dist_um: f64,
}

#[derive(Clone)]
struct AwareNode {
    assigned: Vec<(Position, Position)>,
    used_pairs: BTreeSet<usize>,
    /// `groups`' cost g(p) = Eq. (1) `Σ_G √(d_max(G))` is always recomputable
    /// via [`groups_cost`] — not cached on the node, since only the combined
    /// priority (`neg_priority`) is needed once a node is queued.
    groups: Vec<SearchGroup>,
    /// f(p) = g(p) + h(p), negated for max-heap-as-min-heap (`OrderedFloat`
    /// pops the numerically largest value first; negating cost turns that
    /// into "smallest f(p) first"). The only field `Ord`/`Eq` examine.
    neg_priority: OrderedFloat,
}

#[derive(Clone, Copy, PartialEq)]
struct OrderedFloat(f64);
impl Eq for OrderedFloat {}
impl PartialOrd for OrderedFloat {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for OrderedFloat {
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.partial_cmp(&other.0).unwrap_or(Ordering::Equal)
    }
}

impl PartialEq for AwareNode {
    fn eq(&self, other: &Self) -> bool {
        self.neg_priority == other.neg_priority
    }
}
impl Eq for AwareNode {}
impl PartialOrd for AwareNode {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for AwareNode {
    fn cmp(&self, other: &Self) -> Ordering {
        // BinaryHeap is a max-heap; we store -f(p), so the numerically
        // smallest f(p) (best priority) pops first — standard A* order.
        self.neg_priority.cmp(&other.neg_priority)
    }
}

/// Try to add a planned atom move into the first AOD-compatible group of
/// `groups` (first-fit, same policy as [`partition_aod_compatible`]);
/// otherwise start a new singleton group. Near-zero-distance moves (reuse:
/// the atom is already at its target) join no group and cost nothing, same
/// as the post-search `schedule_zoned` accounting.
fn add_move_to_groups(groups: &mut Vec<SearchGroup>, mv: (Position, Position), min_sep_um: f64) {
    let dist = euclidean_um(mv.0, mv.1);
    if dist < 1e-9 {
        return;
    }
    if let Some(group) = groups.iter_mut().find(|group| {
        group
            .moves
            .iter()
            .all(|&member| positions_aod_compatible(member, mv, min_sep_um))
    }) {
        group.moves.push(mv);
        group.max_dist_um = group.max_dist_um.max(dist);
    } else {
        groups.push(SearchGroup {
            moves: vec![mv],
            max_dist_um: dist,
        });
    }
}

fn groups_cost(groups: &[SearchGroup]) -> f64 {
    routing_cost_eq1(&groups.iter().map(|g| g.max_dist_um).collect::<Vec<_>>())
}

/// Population standard deviation (`0.0` for fewer than 2 samples).
fn std_dev(values: &[f64]) -> f64 {
    if values.len() < 2 {
        return 0.0;
    }
    let n = values.len() as f64;
    let mean = values.iter().sum::<f64>() / n;
    let variance = values.iter().map(|v| (v - mean).powi(2)).sum::<f64>() / n;
    variance.sqrt()
}

/// [RAP] Eq. (4)'s `Σ_G SD(G)`: per axis, per group, the standard deviation
/// of `value − scale·key`, where `key`/`value` are each member's *discrete
/// rank* among the group's distinct source/target coordinates on that axis
/// (its Sec. V-A/V-C — "the source locations of the atoms are rearranged,
/// and the target traps are discretized among all the atoms to be moved"),
/// and `scale` corrects for a group whose target coordinates are more/less
/// spread out in rank-count than its source coordinates (Example 9: 8 target
/// cols vs. 4 source cols ⇒ scale 2, so a rank-uniform — not
/// physically-uniform — placement reads as `SD ≈ 0`).
///
/// Ranks, not raw µm distances: an earlier version of this function used raw
/// `to − from` µm displacement directly, which is *not* a safe substitute —
/// a #297 review finding caught it producing a heuristic that blows up
/// (`SD` in the hundreds) whenever one group member's move happens to be
/// physically much longer than another's, even when the group is perfectly
/// legal and uniform in the only sense that matters (order-preserving
/// discrete placement). Ranks are bounded by group size regardless of
/// absolute travel distance, matching qmap's own bounded behavior
/// (`sumStdDeviationForGroups` operates on the same discrete key/value pairs
/// its BST already stores, never on raw coordinates).
fn sum_std_deviation_for_groups(groups: &[SearchGroup]) -> f64 {
    groups
        .iter()
        .map(|g| {
            axis_std_dev_by_rank(g.moves.iter().map(|(from, to)| (from.x_um, to.x_um)))
                + axis_std_dev_by_rank(g.moves.iter().map(|(from, to)| (from.y_um, to.y_um)))
        })
        .sum()
}

/// One axis' contribution to [`sum_std_deviation_for_groups`]: discretize
/// `from`/`to` coordinates into ranks among their own distinct values, scale
/// the source rank to match the target rank's spread, and return the
/// standard deviation of `to_rank − scale · from_rank` across `pairs`.
fn axis_std_dev_by_rank(pairs: impl Iterator<Item = (f64, f64)> + Clone) -> f64 {
    let froms = distinct_sorted(pairs.clone().map(|(from, _)| from));
    let tos = distinct_sorted(pairs.clone().map(|(_, to)| to));
    // Scale so a placement that advances every member's target rank in
    // lockstep with its source rank (not necessarily 1:1, if the two sides
    // have different counts of distinct coordinates) reads as SD ≈ 0.
    let scale = if froms.len() > 1 {
        (tos.len().saturating_sub(1)).max(1) as f64 / (froms.len() - 1) as f64
    } else {
        1.0
    };
    let diffs: Vec<f64> = pairs
        .map(|(from, to)| rank_of(&tos, to) - scale * rank_of(&froms, from))
        .collect();
    std_dev(&diffs)
}

/// Distinct values from `values`, sorted ascending (µm-scale float
/// equality — these are trap-site coordinates, not accumulated sums, so
/// exact-bit ties from identical sites are the only ties expected).
fn distinct_sorted(values: impl Iterator<Item = f64>) -> Vec<f64> {
    let mut sorted: Vec<f64> = values.collect();
    sorted.sort_by(f64::total_cmp);
    sorted.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    sorted
}

/// Rank (0-indexed position) of `value` among `sorted` (a [`distinct_sorted`]
/// list); `0` if not found (defensive — every value passed in by
/// [`axis_std_dev_by_rank`] was itself used to build the corresponding
/// `sorted` list, so this is unreachable in practice).
fn rank_of(sorted: &[f64], value: f64) -> f64 {
    sorted
        .iter()
        .position(|&x| (x - value).abs() < 1e-9)
        .unwrap_or(0) as f64
}

/// One occupancy-legal `(pair, orientation)` choice for a gate. Occupancy
/// legality ([`pair_occupancy_ok`]) depends only on `site_occupant`, which is
/// fixed for the whole layer — layer-static, unlike the simultaneous-pair
/// spacing conflict ([`pairs_conflict`]), which depends on which pairs the
/// *current node* has already chosen and so must still be rechecked per node
/// via [`pair_legal`].
struct GateCandidate {
    pair_index: usize,
    orient: (Position, Position),
    d_max: f64,
}

/// All occupancy-legal `(pair, orientation)` choices for `gate`, sorted
/// ascending by `d_max`. Both orientations of a pair are kept as separate
/// entries — an orientation can change which movement group a placement
/// joins even when it doesn't change `d_max`. Not window-truncated: the
/// pruning window is applied by the caller while *scanning* this list for
/// legal choices (occupancy + simultaneous-conflict + unused), so
/// completeness (finding a full assignment whenever one exists) does not
/// depend on the window size.
fn gate_candidates(gate: (AtomId, AtomId), inputs: &AssignInputs<'_>) -> Vec<GateCandidate> {
    let (a, b) = gate;
    let pa = atom_position_or_origin(inputs.atom_pos, a);
    let pb = atom_position_or_origin(inputs.atom_pos, b);
    let mut options = Vec::new();
    for (i, &(left, right)) in inputs.pairs.iter().enumerate() {
        if !pair_occupancy_ok(inputs.pair_sites[i], gate, inputs.site_occupant) {
            continue;
        }
        for orient in [(left, right), (right, left)] {
            let d_max = euclidean_um(pa, orient.0).max(euclidean_um(pb, orient.1));
            options.push(GateCandidate {
                pair_index: i,
                orient,
                d_max,
            });
        }
    }
    options.sort_by(|x, y| x.d_max.total_cmp(&y.d_max));
    options
}

/// [RAP] Eqs. (3)-(4): the admissible lower-bound term plus the
/// (inadmissible) accelerating term. See the module doc for the qmap
/// cross-check that resolved Eq. (3)'s sign convention
/// (`max(0, √unplaced − √placed)`, not a bare subtraction) and for why Eq.
/// (5)'s cross-layer look-ahead term has no equivalent here.
fn heuristic_estimate(
    node: &AwareNode,
    gates: &[(AtomId, AtomId)],
    candidates: &[Vec<GateCandidate>],
    params: &AwareSearchParams,
) -> f64 {
    let level = node.assigned.len();
    let n_unplaced = gates.len() - level;
    if n_unplaced == 0 {
        return 0.0;
    }
    let max_dist_of_placed = node
        .groups
        .iter()
        .fold(0.0_f64, |m, g| m.max(g.max_dist_um));
    let mut max_dist_of_unplaced = 0.0_f64;
    for gate_candidates in &candidates[level..] {
        // Nearest choice not yet claimed by this node; if every candidate is
        // already claimed (rare — would require this node to have already
        // used every occupancy-legal pair for some other, still-unplaced,
        // gate), fall back to the worst candidate in the list as a
        // pessimistic (rather than silently-zero) estimate.
        let best = gate_candidates
            .iter()
            .find(|c| !node.used_pairs.contains(&c.pair_index))
            .or_else(|| gate_candidates.last())
            .map_or(0.0, |c| c.d_max);
        max_dist_of_unplaced = max_dist_of_unplaced.max(best);
    }
    let admissible = if max_dist_of_unplaced <= max_dist_of_placed {
        0.0
    } else {
        max_dist_of_unplaced.sqrt() - max_dist_of_placed.sqrt()
    };
    let accelerating = params.deepening_factor
        * (params.deepening_value + sum_std_deviation_for_groups(&node.groups))
        * n_unplaced as f64;
    admissible + accelerating
}

/// Default expansion budget for the routing-aware search before falling back
/// to the greedy assignment (which always terminates and supports
/// deferral). Overridable per call via [`AwareSearchParams::node_budget`].
pub const AWARE_NODE_BUDGET: usize = 100_000;

/// Above this `gates × pairs` product the routing-agnostic path skips the
/// [`assign_matching_legal`] min-weight bipartite matching (O(n²·m)) and uses
/// the O(n·m) [`assign_greedy_legal`] fallback instead (issue #300). The
/// matching is cheap at fixture scale (a 21-gate / 340-pair ising_n42 layer is
/// ~150 k operations) but its per-layer gain over the already-near-optimal
/// greedy is marginal, so a half-million-product "very large" cutoff keeps
/// genuinely huge layers (e.g. hundreds of gates × thousands of pairs) on the
/// fast path. ising_n42's largest layer (21 × 340 = 7140) is well under this.
pub const MATCHING_FALLBACK_GATE_PAIR_PRODUCT: usize = 500_000;

/// Routing-aware A* search ([RAP] Sec. IV-B encoding; Sec. IV-C / V-C
/// heuristic — see the module doc for the full section/equation map): extend
/// by one gate, charge Eq. (1) `√(d_max)` of the AOD-compatible movement
/// groups implied so far, guided by the Eq. (3)-(4) heuristic. Extensions
/// are limited to occupancy- and spacing-legal pairs, and (Sec. V-D) to the
/// nearest `pruning_window` legal choices per gate. The frontier is also
/// beam-trimmed to `beam_width` (see [`AwareSearchParams::beam_width`] —
/// an engineering addition beyond the paper's Eqs., needed to reach full
/// assignments within budget at real fixture scale). When no legal full
/// assignment exists (or the node budget is exhausted) it falls back to
/// [`assign_greedy_legal`], which defers unplaceable gates.
fn assign_aware_legal(
    gates: &[(AtomId, AtomId)],
    inputs: &AssignInputs<'_>,
    params: &AwareSearchParams,
) -> GateAssignment {
    if gates.is_empty() {
        return GateAssignment {
            placed: Vec::new(),
            deferred: Vec::new(),
            outcome: AwareSearchOutcome::Completed,
            node_expansions: 0,
        };
    }
    let candidates: Vec<Vec<GateCandidate>> = gates
        .iter()
        .map(|&gate| gate_candidates(gate, inputs))
        .collect();

    let mut heap = BinaryHeap::new();
    heap.push(AwareNode {
        assigned: Vec::new(),
        used_pairs: BTreeSet::new(),
        groups: Vec::new(),
        neg_priority: OrderedFloat(0.0),
    });

    let mut expansions = 0usize;
    while let Some(node) = heap.pop() {
        let g = node.assigned.len();
        if g == gates.len() {
            return GateAssignment {
                placed: node.assigned.into_iter().enumerate().collect(),
                deferred: Vec::new(),
                outcome: AwareSearchOutcome::Completed,
                node_expansions: expansions,
            };
        }
        expansions += 1;
        if expansions > params.node_budget {
            let mut fallback = assign_greedy_legal(gates, inputs);
            fallback.outcome = AwareSearchOutcome::BudgetExceeded;
            fallback.node_expansions = expansions;
            return fallback;
        }
        let gate = gates[g];
        let pa = atom_position_or_origin(inputs.atom_pos, gate.0);
        let pb = atom_position_or_origin(inputs.atom_pos, gate.1);
        let mut taken = 0usize;
        for cand in &candidates[g] {
            if node.used_pairs.contains(&cand.pair_index)
                || !pair_legal(cand.pair_index, gate, &node.used_pairs, inputs)
            {
                continue;
            }
            let mut groups = node.groups.clone();
            add_move_to_groups(&mut groups, (pa, cand.orient.0), inputs.aod_min_sep_um);
            add_move_to_groups(&mut groups, (pb, cand.orient.1), inputs.aod_min_sep_um);
            let cost_so_far = groups_cost(&groups);
            let mut used_pairs = node.used_pairs.clone();
            used_pairs.insert(cand.pair_index);
            let mut assigned = node.assigned.clone();
            assigned.push(cand.orient);
            let child = AwareNode {
                assigned,
                used_pairs,
                groups,
                neg_priority: OrderedFloat(0.0),
            };
            let h = heuristic_estimate(&child, gates, &candidates, params);
            let priority = cost_so_far + h;
            heap.push(AwareNode {
                neg_priority: OrderedFloat(-priority),
                ..child
            });
            taken += 1;
            if taken >= params.pruning_window {
                break;
            }
        }
        // Beam trim: without this, the frontier is dominated by a huge
        // number of shallow (early-gate) alternatives that all look
        // similarly cheap, and the search never accumulates enough budget
        // to reach a full-depth completion (#297 review finding — measured
        // on `ising_n42`'s real 20-21-gate/340-pair layers, where the
        // frontier reached over a million queued nodes while still stuck
        // below half depth). Keeping only the best `beam_width` candidates
        // forces the search to commit to its most promising partial
        // placements instead of endlessly generating new shallow ones.
        if heap.len() > params.beam_width * 2 {
            let mut kept = BinaryHeap::with_capacity(params.beam_width);
            for _ in 0..params.beam_width {
                match heap.pop() {
                    Some(n) => kept.push(n),
                    None => break,
                }
            }
            heap = kept;
        }
    }
    // Search space exhausted: no legal assignment of every gate exists this
    // stage. Place what greedy can and defer the rest.
    let mut fallback = assign_greedy_legal(gates, inputs);
    fallback.outcome = AwareSearchOutcome::NoLegalAssignment;
    fallback.node_expansions = expansions;
    fallback
}

/// Tiny reference architecture for unit tests (not the full 73×101 geometry).
pub fn toy_zoned_architecture() -> ZonedArchitecture {
    ZonedArchitecture {
        zones: vec![
            ZoneSpec {
                zone_id: 0,
                kind: ZoneKind::Storage,
                rows: 4,
                cols: 4,
                origin_um: (0.0, 0.0),
                site_pitch_um: (4.0, 4.0),
                pair_gap_um: None,
            },
            ZoneSpec {
                zone_id: 1,
                kind: ZoneKind::Entanglement,
                rows: 2,
                cols: 4,
                origin_um: (0.0, 50.0),
                site_pitch_um: (12.0, 10.0),
                pair_gap_um: Some(2.0),
            },
            ZoneSpec {
                zone_id: 2,
                kind: ZoneKind::Readout,
                rows: 2,
                cols: 4,
                origin_um: (0.0, 100.0),
                site_pitch_um: (4.0, 4.0),
                pair_gap_um: None,
            },
        ],
        acceleration_m_s2: 2750.0,
        trap_transfer_us: 15,
        require_readout_zone: false,
        rydberg_range_um: 7.5,
        min_rydberg_spacing_um: 18.75,
        aod_min_separation_um: 2.0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entangling_schedule::schedule_entangling_layers;
    use crate::graph::{
        DEFAULT_GAMMA, Interaction, InteractionGraph, InteractionId, InteractionSegment,
        LogicalQubitId, SegmentKind,
    };
    use crate::schedule::{MeasurementBasis, NeutralAtomAction, ScheduleLayer};
    use crate::schedule_entry::schedule_from_graph;

    fn matching_graph(n_pairs: u32) -> InteractionGraph {
        let n = n_pairs * 2;
        let vertices: Vec<_> = (0..n).map(LogicalQubitId).collect();
        let mut interactions = Vec::new();
        let mut ids = Vec::new();
        for i in 0..n_pairs {
            let id = InteractionId(i);
            ids.push(id);
            interactions.push(Interaction {
                id,
                qubits: vec![LogicalQubitId(2 * i), LogicalQubitId(2 * i + 1)],
                gate_name: "CZ".into(),
                dag_layer: 0,
                on_critical_path: false,
            });
        }
        InteractionGraph::from_interactions(
            vertices,
            interactions,
            vec![InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: ids,
            }],
            DEFAULT_GAMMA,
        )
        .expect("graph")
    }

    /// Chain graph: gate i couples qubits (i, i+1); shared atoms force one
    /// Misra–Gries layer per gate.
    fn chain_graph(n_gates: u32) -> InteractionGraph {
        let n = n_gates + 1;
        let vertices: Vec<_> = (0..n).map(LogicalQubitId).collect();
        let mut interactions = Vec::new();
        let mut ids = Vec::new();
        for i in 0..n_gates {
            let id = InteractionId(i);
            ids.push(id);
            interactions.push(Interaction {
                id,
                qubits: vec![LogicalQubitId(i), LogicalQubitId(i + 1)],
                gate_name: "CZ".into(),
                dag_layer: i,
                on_critical_path: false,
            });
        }
        InteractionGraph::from_interactions(
            vertices,
            interactions,
            vec![InteractionSegment {
                kind: SegmentKind::DependencyDag,
                interactions: ids,
            }],
            DEFAULT_GAMMA,
        )
        .expect("graph")
    }

    /// Replay a zoned schedule's moves against the layout, asserting no site
    /// is ever occupied by two atoms and no layer double-claims a site.
    fn assert_occupancy_sound(result: &ZonedScheduleResult) {
        let layout = result.request.layout.as_ref().expect("layout");
        // Reconstruct the initial occupancy by undoing moves from the final
        // bindings (planners rewrite initial_bindings to final occupancy).
        let mut occupancy: BTreeMap<AtomId, SiteId> = layout
            .initial_bindings
            .iter()
            .map(|b| {
                let site = match b.trap {
                    TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
                };
                (b.atom, site)
            })
            .collect();
        for layer in result.request.layers.iter().rev() {
            layer.validate_occupancy().expect("layer occupancy");
            for action in &layer.actions {
                if let NeutralAtomAction::Move(group) = action {
                    for m in &group.moves {
                        occupancy.insert(m.atom, m.from);
                    }
                }
            }
        }
        // Forward replay: destinations must be empty when moved into.
        let mut site_holder: BTreeMap<SiteId, AtomId> =
            occupancy.iter().map(|(&a, &s)| (s, a)).collect();
        assert_eq!(
            site_holder.len(),
            occupancy.len(),
            "initial double occupancy"
        );
        for layer in &result.request.layers {
            for action in &layer.actions {
                if let NeutralAtomAction::Move(group) = action {
                    for m in &group.moves {
                        assert_eq!(
                            site_holder.remove(&m.from),
                            Some(m.atom),
                            "cycle {}: atom {:?} moved from a site it did not hold",
                            layer.cycle,
                            m.atom
                        );
                        assert!(
                            !site_holder.contains_key(&m.to),
                            "cycle {}: atom {:?} moved onto occupied site {:?}",
                            layer.cycle,
                            m.atom,
                            m.to
                        );
                        site_holder.insert(m.to, m.atom);
                    }
                }
            }
        }
    }

    /// Per entangle layer, per gate: the entangling atoms' positions,
    /// replayed from the schedule's moves.
    #[allow(clippy::type_complexity)]
    fn entangle_layer_positions(result: &ZonedScheduleResult) -> Vec<Vec<Vec<(AtomId, Position)>>> {
        let layout = result.request.layout.as_ref().expect("layout");
        let site_pos: BTreeMap<SiteId, Position> =
            layout.sites.iter().map(|s| (s.id, s.position)).collect();
        let mut occupancy: BTreeMap<AtomId, SiteId> = layout
            .initial_bindings
            .iter()
            .map(|b| {
                let site = match b.trap {
                    TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
                };
                (b.atom, site)
            })
            .collect();
        let mut per_layer_after: Vec<BTreeMap<AtomId, SiteId>> =
            vec![BTreeMap::new(); result.request.layers.len()];
        for (index, layer) in result.request.layers.iter().enumerate().rev() {
            per_layer_after[index] = occupancy.clone();
            for action in &layer.actions {
                if let NeutralAtomAction::Move(group) = action {
                    for m in &group.moves {
                        occupancy.insert(m.atom, m.from);
                    }
                }
            }
        }

        let mut out = Vec::new();
        for (index, layer) in result.request.layers.iter().enumerate() {
            let mut gates: Vec<Vec<(AtomId, Position)>> = Vec::new();
            for action in &layer.actions {
                let acting: &[AtomId] = match action {
                    NeutralAtomAction::Entangle2 { atoms, .. } => atoms,
                    NeutralAtomAction::EntangleN { atoms, .. } => atoms,
                    _ => continue,
                };
                gates.push(
                    acting
                        .iter()
                        .map(|&atom| (atom, site_pos[&per_layer_after[index][&atom]]))
                        .collect(),
                );
            }
            if !gates.is_empty() {
                out.push(gates);
            }
        }
        out
    }

    #[test]
    fn sqrt_d_max_non_negative() {
        assert_eq!(sqrt_d_max(0.0), 0.0);
        assert!((sqrt_d_max(4.0) - 2.0).abs() < 1e-9);
        assert_eq!(sqrt_d_max(-1.0), 0.0);
    }

    #[test]
    fn eq1_sums_group_costs() {
        assert!((routing_cost_eq1(&[4.0, 9.0]) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn toy_arch_validates() {
        toy_zoned_architecture().validate().expect("ok");
    }

    #[test]
    fn schedule_zoned_entangles_in_entanglement_zone() {
        let graph = matching_graph(2);
        let req = schedule_from_graph(graph).expect("stub");
        let scheduled = schedule_entangling_layers(req, 340).expect("layers");
        let arch = toy_zoned_architecture();
        let result =
            schedule_zoned(scheduled.request, &arch, PlacerMode::RoutingAgnostic).expect("zoned");
        assert!(result.rearrangement_steps >= 1 || result.routing_cost >= 0.0);
        let has_entangle = result.request.layers.iter().any(|l| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Entangle2 { .. }))
        });
        assert!(has_entangle);
    }

    #[test]
    fn aware_cost_not_worse_than_agnostic_on_matching() {
        let graph = matching_graph(2);
        let req = schedule_from_graph(graph).expect("stub");
        let scheduled = schedule_entangling_layers(req, 340).expect("layers");
        let arch = toy_zoned_architecture();
        let agnostic = schedule_zoned(
            scheduled.request.clone(),
            &arch,
            PlacerMode::RoutingAgnostic,
        )
        .expect("agnostic");
        let aware =
            schedule_zoned(scheduled.request, &arch, PlacerMode::RoutingAware).expect("aware");
        assert!(
            aware.routing_cost <= agnostic.routing_cost + 1e-6,
            "aware {} vs agnostic {}",
            aware.routing_cost,
            agnostic.routing_cost
        );
    }

    /// Total Eq. (1) cost `Σ_G √(d_max(G))` a [`GateAssignment`] would incur,
    /// recomputed independently of [`assign_aware_legal`]'s own bookkeeping so
    /// this doubles as a check that its `placed` orientations are the ones it
    /// claims.
    fn assignment_cost(
        assignment: &GateAssignment,
        gates: &[(AtomId, AtomId)],
        atom_pos: &BTreeMap<AtomId, Position>,
    ) -> f64 {
        assignment
            .placed
            .iter()
            .map(|&(gate_index, (target_a, target_b))| {
                let (a, b) = gates[gate_index];
                let pa = atom_position_or_origin(atom_pos, a);
                let pb = atom_position_or_origin(atom_pos, b);
                sqrt_d_max(euclidean_um(pa, target_a).max(euclidean_um(pb, target_b)))
            })
            .sum()
    }

    /// Issue #111 review finding: a routing-aware layer that silently falls
    /// back to [`assign_greedy_legal`] (budget exhaustion or no legal full
    /// assignment) is indistinguishable, by cost alone, from "no routing
    /// contention" — unless the outcome is instrumented. This is a small,
    /// genuinely contended two-gate/two-pair layout (no target/circuit
    /// pinning, no A* budget pressure) where greedy's per-gate myopic choice
    /// is provably wrong: gate A only mildly prefers pair0 over pair1, but
    /// gate B is disastrous at pair1 and excellent at pair0. Greedy assigns
    /// gate A (processed first) its slightly-preferred pair0, stranding gate
    /// B at pair1; the joint-optimal swaps them. The aware search must
    /// (a) report [`AwareSearchOutcome::Completed`] (not a fallback) and
    /// (b) find the strictly cheaper swap, proving the search mechanism
    /// itself — not just budget luck — is what produces an aware/greedy gap.
    #[test]
    fn aware_search_completes_and_beats_greedy_on_contended_pairs() {
        let gate_a = (AtomId(0), AtomId(1));
        let gate_b = (AtomId(2), AtomId(3));
        let gates = [gate_a, gate_b];

        let mut atom_pos = BTreeMap::new();
        // Gate A sits near the midpoint between the two pairs: pair0 is only
        // marginally closer than pair1.
        atom_pos.insert(
            AtomId(0),
            Position {
                x_um: 499.0,
                y_um: 0.0,
            },
        );
        atom_pos.insert(
            AtomId(1),
            Position {
                x_um: 500.0,
                y_um: 0.0,
            },
        );
        // Gate B sits essentially on top of pair0 and 1000 um from pair1.
        atom_pos.insert(
            AtomId(2),
            Position {
                x_um: 0.4,
                y_um: 0.0,
            },
        );
        atom_pos.insert(
            AtomId(3),
            Position {
                x_um: 0.6,
                y_um: 0.0,
            },
        );

        let pair0 = (
            Position {
                x_um: 0.0,
                y_um: 0.0,
            },
            Position {
                x_um: 1.0,
                y_um: 0.0,
            },
        );
        let pair1 = (
            Position {
                x_um: 1000.0,
                y_um: 0.0,
            },
            Position {
                x_um: 1001.0,
                y_um: 0.0,
            },
        );
        let pairs = vec![pair0, pair1];
        let pair_sites = vec![(SiteId(100), SiteId(101)), (SiteId(102), SiteId(103))];
        let site_occupant = BTreeMap::new();
        let inputs = AssignInputs {
            atom_pos: &atom_pos,
            pairs: &pairs,
            pair_sites: &pair_sites,
            site_occupant: &site_occupant,
            conflict_um: 0.0,
            aod_min_sep_um: 0.0,
        };

        let greedy = assign_greedy_legal(&gates, &inputs);
        assert_eq!(greedy.outcome, AwareSearchOutcome::NotApplicable);
        assert!(greedy.deferred.is_empty());

        let aware = assign_aware_legal(&gates, &inputs, &AwareSearchParams::default());
        assert_eq!(
            aware.outcome,
            AwareSearchOutcome::Completed,
            "search space is tiny (2 gates x 2 pairs); it must complete, not fall back"
        );
        assert!(aware.deferred.is_empty());

        let greedy_cost = assignment_cost(&greedy, &gates, &atom_pos);
        let aware_cost = assignment_cost(&aware, &gates, &atom_pos);
        assert!(
            aware_cost < greedy_cost * 0.5,
            "aware ({aware_cost}) must substantially beat greedy's myopic pick ({greedy_cost})"
        );
    }

    /// Issue #300 acceptance: a "crossing-pairs" construction where the
    /// routing-agnostic min-weight matching placer strictly beats the greedy
    /// nearest-legal placer on the same inputs. With no spacing conflicts
    /// (`conflict_um = 0`) the matching's travel-optimal assignment is used
    /// directly (no greedy repair), isolating the matching mechanism itself.
    ///
    /// Gate A sits near the midpoint of two pairs and only mildly prefers
    /// pair0; gate B sits on top of pair0 and is disastrous at pair1. Greedy,
    /// processing gate A first, hands pair0 to A and strands B at pair1. The
    /// matching sees the joint optimum — give pair0 to B, pair1 to A — which
    /// strictly lowers the total √-law Eq. (1) cost (and swaps the two gates'
    /// pairs, proving the assignments differ, not just the cost).
    #[test]
    fn matching_placer_beats_greedy_on_crossing_pairs() {
        let gate_a = (AtomId(0), AtomId(1));
        let gate_b = (AtomId(2), AtomId(3));
        let gates = [gate_a, gate_b];

        let mut atom_pos = BTreeMap::new();
        atom_pos.insert(
            AtomId(0),
            Position {
                x_um: 499.0,
                y_um: 0.0,
            },
        );
        atom_pos.insert(
            AtomId(1),
            Position {
                x_um: 500.0,
                y_um: 0.0,
            },
        );
        atom_pos.insert(
            AtomId(2),
            Position {
                x_um: 0.4,
                y_um: 0.0,
            },
        );
        atom_pos.insert(
            AtomId(3),
            Position {
                x_um: 0.6,
                y_um: 0.0,
            },
        );

        let pair0 = (
            Position {
                x_um: 0.0,
                y_um: 0.0,
            },
            Position {
                x_um: 1.0,
                y_um: 0.0,
            },
        );
        let pair1 = (
            Position {
                x_um: 1000.0,
                y_um: 0.0,
            },
            Position {
                x_um: 1001.0,
                y_um: 0.0,
            },
        );
        let pairs = vec![pair0, pair1];
        let pair_sites = vec![(SiteId(100), SiteId(101)), (SiteId(102), SiteId(103))];
        let site_occupant = BTreeMap::new();
        let inputs = AssignInputs {
            atom_pos: &atom_pos,
            pairs: &pairs,
            pair_sites: &pair_sites,
            site_occupant: &site_occupant,
            conflict_um: 0.0,
            aod_min_sep_um: 0.0,
        };

        let greedy = assign_greedy_legal(&gates, &inputs);
        assert_eq!(greedy.outcome, AwareSearchOutcome::NotApplicable);
        assert!(greedy.deferred.is_empty(), "greedy must place both gates");

        let (matching, used_matching) = assign_matching_legal(&gates, &inputs);
        assert!(
            used_matching,
            "matching produced the assignment (no fallback)"
        );
        assert!(
            matching.deferred.is_empty(),
            "matching must place both gates"
        );

        // The two placers must disagree (matching swaps the pairs); a mere
        // cost tie with identical placement would not demonstrate the mechanism.
        assert_ne!(
            greedy.placed, matching.placed,
            "matching must pick the swapped assignment, not reproduce greedy"
        );
        // Each gate's pair index must differ between the two placers.
        let greedy_pairs: BTreeSet<usize> = greedy.placed.iter().map(|(g, _)| *g).collect();
        let matching_pairs: BTreeSet<usize> = matching.placed.iter().map(|(g, _)| *g).collect();
        assert_eq!(greedy_pairs, matching_pairs, "both place the same gates");

        let greedy_cost = assignment_cost(&greedy, &gates, &atom_pos);
        let matching_cost = assignment_cost(&matching, &gates, &atom_pos);
        assert!(
            matching_cost < greedy_cost * 0.5,
            "matching ({matching_cost}) must strictly beat greedy's myopic pick ({greedy_cost})"
        );
    }

    /// Scaled-up sibling of
    /// [`aware_search_completes_and_beats_greedy_on_contended_pairs`] — issue
    /// #297's acceptance criteria explicitly ask for a variant "closer to the
    /// ising_n42 scale" (`docs/neutral_atom/rap_table_i_methodology.md`'s
    /// anchor fixture has up to 21 simultaneous gates and 340 candidate
    /// entanglement-zone pairs per layer). This test replicates the small
    /// test's exact contention shape (a mildly-preferring gate A and a
    /// disastrously-mispaired gate B whose naive/greedy choices should be
    /// swapped) 10 times over, in 10 well-separated, mutually non-conflicting
    /// clusters (20 gates total), embedded in a pool of 340 candidate pairs
    /// (20 "real" + 320 far-away, uniformly-unattractive filler pairs that
    /// exist only to inflate the branching factor to match the real
    /// fixture's scale). Before #297's Eq. (3)-(4) heuristic, the *old*
    /// uniform-cost (`h = 0`) search explored nodes in nondecreasing-cost
    /// order over this same 20-gate/340-pair space — exactly the shape
    /// documented (`rap_table_i_methodology.md`'s "Phase 1 finding") to blow
    /// `AWARE_NODE_BUDGET` on every layer of the real `ising_n42` fixture, so
    /// this synthetic scenario is a fair proxy for that failure mode (the old
    /// code no longer exists to run head-to-head, since replacing it wholesale
    /// was the point of #297).
    #[test]
    fn aware_search_completes_and_beats_greedy_at_ising_n42_scale() {
        const GROUPS: usize = 10;
        const GROUP_SPACING: f64 = 5000.0;
        const FILLER_PAIRS: usize = 320;

        let mut gates = Vec::new();
        let mut atom_pos = BTreeMap::new();
        let mut pairs = Vec::new();
        let mut pair_sites = Vec::new();
        let mut next_atom = 0u32;
        let mut next_site = 200u32; // clear of the {100..103} range used elsewhere in this file.

        for group in 0..GROUPS {
            let offset = group as f64 * GROUP_SPACING;
            let atom_a0 = AtomId(next_atom);
            let atom_a1 = AtomId(next_atom + 1);
            let atom_b0 = AtomId(next_atom + 2);
            let atom_b1 = AtomId(next_atom + 3);
            next_atom += 4;

            // Gate A: near the midpoint, mildly prefers pair_near.
            atom_pos.insert(
                atom_a0,
                Position {
                    x_um: offset + 499.0,
                    y_um: 0.0,
                },
            );
            atom_pos.insert(
                atom_a1,
                Position {
                    x_um: offset + 500.0,
                    y_um: 0.0,
                },
            );
            // Gate B: essentially on top of pair_near, disastrous at pair_far.
            atom_pos.insert(
                atom_b0,
                Position {
                    x_um: offset + 0.4,
                    y_um: 0.0,
                },
            );
            atom_pos.insert(
                atom_b1,
                Position {
                    x_um: offset + 0.6,
                    y_um: 0.0,
                },
            );

            let pair_near = (
                Position {
                    x_um: offset,
                    y_um: 0.0,
                },
                Position {
                    x_um: offset + 1.0,
                    y_um: 0.0,
                },
            );
            let pair_far = (
                Position {
                    x_um: offset + 1000.0,
                    y_um: 0.0,
                },
                Position {
                    x_um: offset + 1001.0,
                    y_um: 0.0,
                },
            );
            pairs.push(pair_near);
            pairs.push(pair_far);
            pair_sites.push((SiteId(next_site), SiteId(next_site + 1)));
            pair_sites.push((SiteId(next_site + 2), SiteId(next_site + 3)));
            next_site += 4;

            gates.push((atom_a0, atom_a1));
            gates.push((atom_b0, atom_b1));
        }

        // Filler pairs: far from every group's atoms (never the cheapest
        // choice for any gate) but occupancy-legal, so they genuinely widen
        // this layer's branching factor to ~340 candidates/gate like the
        // real fixture, without perturbing the intended optimum.
        for k in 0..FILLER_PAIRS {
            let x = 10_000_000.0 + k as f64 * 10.0;
            pairs.push((
                Position { x_um: x, y_um: 0.0 },
                Position {
                    x_um: x + 1.0,
                    y_um: 0.0,
                },
            ));
            pair_sites.push((SiteId(next_site), SiteId(next_site + 1)));
            next_site += 2;
        }
        assert_eq!(pairs.len(), 2 * GROUPS + FILLER_PAIRS);
        assert_eq!(pairs.len(), 340, "matches ising_n42's per-layer pair count");
        assert_eq!(gates.len(), 20, "close to ising_n42's per-layer gate count");

        let site_occupant = BTreeMap::new();
        let inputs = AssignInputs {
            atom_pos: &atom_pos,
            pairs: &pairs,
            pair_sites: &pair_sites,
            site_occupant: &site_occupant,
            conflict_um: 0.0,
            aod_min_sep_um: 0.0,
        };

        let greedy = assign_greedy_legal(&gates, &inputs);
        assert_eq!(greedy.outcome, AwareSearchOutcome::NotApplicable);
        assert!(greedy.deferred.is_empty());

        let aware = assign_aware_legal(&gates, &inputs, &AwareSearchParams::default());
        assert_eq!(
            aware.outcome,
            AwareSearchOutcome::Completed,
            "the guided search must complete (not fall back to greedy) even at this scale"
        );
        assert!(aware.deferred.is_empty());
        assert!(
            aware.node_expansions < AWARE_NODE_BUDGET / 10,
            "a guided search should need far fewer than the full budget's worth of expansions \
             to solve 10 independent copies of a tiny contended scenario; got {} expansions \
             (budget {AWARE_NODE_BUDGET})",
            aware.node_expansions
        );

        let greedy_cost = assignment_cost(&greedy, &gates, &atom_pos);
        let aware_cost = assignment_cost(&aware, &gates, &atom_pos);
        assert!(
            aware_cost < greedy_cost * 0.5,
            "aware ({aware_cost}) must substantially beat greedy's myopic pick ({greedy_cost}), \
             summed across all {GROUPS} independent contended clusters"
        );
    }

    #[test]
    fn sequential_gates_never_share_a_parked_pair() {
        // Regression (qft_small / qaoa_graph): gate (1, 3) parks its atoms at
        // a pair; the follow-up gate (2, 3) used to be assigned the same pair
        // (per-layer `used` reset), leaving atoms 1 and 2 claiming one site.
        let vertices: Vec<_> = (0..4).map(LogicalQubitId).collect();
        let interactions = vec![
            Interaction {
                id: InteractionId(0),
                qubits: vec![LogicalQubitId(1), LogicalQubitId(3)],
                gate_name: "CZ".into(),
                dag_layer: 0,
                on_critical_path: false,
            },
            Interaction {
                id: InteractionId(1),
                qubits: vec![LogicalQubitId(2), LogicalQubitId(3)],
                gate_name: "CZ".into(),
                dag_layer: 1,
                on_critical_path: false,
            },
        ];
        let graph = InteractionGraph::from_interactions(
            vertices,
            interactions,
            vec![InteractionSegment {
                kind: SegmentKind::DependencyDag,
                interactions: vec![InteractionId(0), InteractionId(1)],
            }],
            DEFAULT_GAMMA,
        )
        .expect("graph");
        let req = schedule_from_graph(graph).expect("stub");
        let scheduled = schedule_entangling_layers(req, 340).expect("layers");
        let result = schedule_zoned(
            scheduled.request,
            &toy_zoned_architecture(),
            PlacerMode::RoutingAgnostic,
        )
        .expect("zoned");
        assert_occupancy_sound(&result);
    }

    #[test]
    fn long_chains_stay_occupancy_sound() {
        // Deeper reuse pattern across many layers.
        let graph = chain_graph(6);
        let req = schedule_from_graph(graph).expect("stub");
        let scheduled = schedule_entangling_layers(req, 340).expect("layers");
        for mode in [PlacerMode::RoutingAgnostic, PlacerMode::RoutingAware] {
            let result = schedule_zoned(scheduled.request.clone(), &toy_zoned_architecture(), mode)
                .expect("zoned");
            assert_occupancy_sound(&result);
        }
    }

    #[test]
    fn simultaneous_gates_respect_min_rydberg_spacing() {
        // Two disjoint gates in one layer: assigned pairs must keep every
        // non-partner atom pair farther apart than the isolation spacing.
        let graph = matching_graph(2);
        let req = schedule_from_graph(graph).expect("stub");
        let scheduled = schedule_entangling_layers(req, 340).expect("layers");
        let arch = toy_zoned_architecture();
        let conflict_um = arch.pair_conflict_um();
        assert!(conflict_um > 0.0, "toy arch must carry interaction limits");
        for mode in [PlacerMode::RoutingAgnostic, PlacerMode::RoutingAware] {
            let result = schedule_zoned(scheduled.request.clone(), &arch, mode).expect("zoned");
            assert_occupancy_sound(&result);
            for layer_gates in entangle_layer_positions(&result) {
                for i in 0..layer_gates.len() {
                    for other in layer_gates.iter().skip(i + 1) {
                        for &(a, pa) in &layer_gates[i] {
                            for &(b, pb) in other {
                                let d = euclidean_um(pa, pb);
                                assert!(
                                    d > conflict_um,
                                    "atoms {a:?}/{b:?} of simultaneous gates are {d} um apart"
                                );
                            }
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn spacing_defers_gates_when_no_legal_pair_coexists() {
        // Entanglement zone with two adjacent (conflicting) pairs only: the
        // two simultaneous gates cannot coexist, so one is deferred to its
        // own later Rydberg layer instead of firing illegally.
        let mut arch = toy_zoned_architecture();
        arch.zones[1].rows = 1;
        arch.zones[1].cols = 2;
        let graph = matching_graph(2);
        let req = schedule_from_graph(graph).expect("stub");
        let scheduled = schedule_entangling_layers(req, 340).expect("layers");
        let result =
            schedule_zoned(scheduled.request, &arch, PlacerMode::RoutingAgnostic).expect("zoned");
        assert_occupancy_sound(&result);
        let entangle_layers = entangle_layer_positions(&result);
        assert_eq!(
            entangle_layers.len(),
            2,
            "conflicting gates must split into two Rydberg layers"
        );
        assert!(entangle_layers.iter().all(|gates| gates.len() == 1));
    }

    #[test]
    fn mixed_source_moves_split_into_aod_compatible_groups() {
        // Gate (0, 1) parks atoms in the zone; gate (1, 2) then moves atom 1
        // (zone-sourced) and atom 2 (storage-sourced) to a fresh pair. Both
        // end on the same row, so one coupled AOD grab is unrealizable — the
        // transition must emit two movement stages.
        let graph = chain_graph(2);
        let req = schedule_from_graph(graph).expect("stub");
        let scheduled = schedule_entangling_layers(req, 340).expect("layers");
        let result = schedule_zoned(
            scheduled.request,
            &toy_zoned_architecture(),
            PlacerMode::RoutingAgnostic,
        )
        .expect("zoned");
        assert_occupancy_sound(&result);
        let move_layers = result
            .request
            .layers
            .iter()
            .filter(|l| {
                l.actions
                    .iter()
                    .any(|a| matches!(a, NeutralAtomAction::Move(_)))
            })
            .count();
        assert_eq!(
            move_layers, 3,
            "first stage is one group, second transition splits into two"
        );
        assert_eq!(result.rearrangement_steps, 3);
    }

    #[test]
    fn axis_aod_compatibility_rules() {
        // Same source coordinate: identical destination required.
        assert!(axis_aod_compatible(0.0, 50.0, 0.0, 50.0, 2.0));
        assert!(!axis_aod_compatible(0.0, 50.0, 0.0, 60.0, 2.0));
        // Distinct sources: order preserved and destination separation kept.
        assert!(axis_aod_compatible(0.0, 50.0, 4.0, 54.0, 2.0));
        assert!(!axis_aod_compatible(0.0, 50.0, 4.0, 50.0, 2.0)); // converge
        assert!(!axis_aod_compatible(0.0, 54.0, 4.0, 50.0, 2.0)); // cross
        assert!(!axis_aod_compatible(0.0, 50.0, 4.0, 51.0, 2.0)); // too close
    }

    #[test]
    fn rejects_entangle_outside_zone_in_validator() {
        let arch = toy_zoned_architecture();
        let layout = NeutralAtomLayout {
            sites: vec![AtomSite {
                id: SiteId(0),
                position: Position {
                    x_um: 0.0,
                    y_um: 0.0,
                }, // storage
            }],
            initial_bindings: vec![
                AtomBinding {
                    atom: AtomId(0),
                    trap: TrapBinding::Slm { site: SiteId(0) },
                },
                AtomBinding {
                    atom: AtomId(1),
                    trap: TrapBinding::Slm { site: SiteId(0) },
                },
            ],
        };
        let layers = vec![ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [AtomId(0), AtomId(1)],
                duration_us: 1,
            }],
        }];
        assert!(matches!(
            validate_zone_constraints(&layers, &layout, &arch),
            Err(ZonedScheduleError::EntangleOutsideZone(_))
        ));
    }

    #[test]
    fn measure_requires_readout_when_configured() {
        let mut arch = toy_zoned_architecture();
        arch.require_readout_zone = true;
        let layout = NeutralAtomLayout {
            sites: vec![AtomSite {
                id: SiteId(0),
                position: Position {
                    x_um: 0.0,
                    y_um: 0.0,
                },
            }],
            initial_bindings: vec![AtomBinding {
                atom: AtomId(0),
                trap: TrapBinding::Slm { site: SiteId(0) },
            }],
        };
        let layers = vec![ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::Measure {
                atom: AtomId(0),
                basis: MeasurementBasis::Z,
                duration_us: 1,
            }],
        }];
        assert!(matches!(
            validate_zone_constraints(&layers, &layout, &arch),
            Err(ZonedScheduleError::MeasureOutsideReadout(_))
        ));
    }
}
