//! Zoned routing-aware placement (issue #107).
//!
//! Reproduces the **placement cost = routing cost** formulation of
//! [RAP] (Stade, Lin, Cong, Wille, ICCAD 2025, arXiv:2505.22715):
//!
//! - Sec. III-B — routing-aware definition (layer-by-layer; cost is routing)
//! - Sec. III-A — reuse analysis (“don’t move atoms already in place”)
//! - Sec. IV-A — cost Eq. (1): `cost(p) = Σ_G √(d_max(G))` over greedily
//!   grouped compatible movements; Eq. (2) adds reuse / look-ahead terms
//! - Sec. IV-B — search extends by assigning one gate’s atoms to entanglement
//!   pairs (A*-style / best-first)
//!
//! Readout-zone measurement constraints come from [AbstractModel]
//! (arXiv:2405.08068) Sec. III-A, **not** from [RAP] (which models only
//! storage + entanglement). Flat AOD movement (#106) is a distinct Enola /
//! OLSQ-DPQA line — do not cite this module as that planner.
//!
//! Dual modes ([RAP] Sec. VI-B comparison methodology):
//! - [`PlacerMode::RoutingAgnostic`] — ZAC-style distance-minimizing placement
//! - [`PlacerMode::RoutingAware`] — search minimizing Eq. (1) routing cost

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet, BinaryHeap, VecDeque};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::graph::InteractionGraph;
use crate::layout::AodTrapRef;
use crate::layout::{
    AtomBinding, AtomId, AtomSite, NeutralAtomLayout, Position, SiteId, TrapBinding,
};
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
}

/// Result of [`schedule_zoned`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ZonedScheduleResult {
    pub request: GraphScheduleRequest,
    pub mode: PlacerMode,
    /// Σ_G √(d_max(G)) over emitted movement groups ([RAP] Eq. (1)).
    pub routing_cost: f64,
    pub rearrangement_steps: u64,
    pub trap_transfers: u64,
    /// Number of per-layer gate-assignment calls where [`assign_aware_legal`]
    /// found a full legal assignment within [`AWARE_NODE_BUDGET`] (uniform-cost
    /// search, so this is the true joint-optimal for that layer). Always `0`
    /// under [`PlacerMode::RoutingAgnostic`] (the concept doesn't apply).
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

/// Schedule a graph request onto a zoned architecture.
///
/// Expects `req.layers` to already contain entangling layers (#105). Fills
/// movement/transfer actions around those entangles and updates `layout` to a
/// storage+entanglement site map. Soft-unblocked vs #106: uses RAP-local
/// greedy move grouping, not Enola sortIS.
pub fn schedule_zoned(
    mut req: GraphScheduleRequest,
    arch: &ZonedArchitecture,
    mode: PlacerMode,
) -> Result<ZonedScheduleResult, ZonedScheduleError> {
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
        };
        let gate_atoms: Vec<(AtomId, AtomId)> = gates.iter().map(|g| g.atoms).collect();
        let assignment = match mode {
            PlacerMode::RoutingAgnostic => assign_greedy_legal(&gate_atoms, &inputs),
            PlacerMode::RoutingAware => assign_aware_legal(&gate_atoms, &inputs),
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
    axis_aod_compatible(a.from.y_um, a.to.y_um, b.from.y_um, b.to.y_um, min_sep_um)
        && axis_aod_compatible(a.from.x_um, a.to.x_um, b.from.x_um, b.to.x_um, min_sep_um)
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

fn build_zoned_layout(
    graph: &InteractionGraph,
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
            atom: AtomId(q.0),
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
}

/// Whether [`assign_aware_legal`]'s best-first search found the true
/// joint-optimal assignment for a layer, or gave up and fell back to
/// [`assign_greedy_legal`] (issue #111 review finding: a silent fallback here
/// is indistinguishable from "no routing contention" unless it is surfaced).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AwareSearchOutcome {
    /// [`assign_greedy_legal`] was called directly (routing-agnostic mode);
    /// the aware-search-completion concept doesn't apply.
    NotApplicable,
    /// The search popped a full-assignment goal node within
    /// [`AWARE_NODE_BUDGET`] expansions. Uniform-cost search (h = 0) pops
    /// nodes in nondecreasing cost order, so this is the true joint-optimal
    /// assignment for the layer, not an approximation.
    Completed,
    /// The search exhausted [`AWARE_NODE_BUDGET`] expansions before popping a
    /// full-assignment goal node and fell back to [`assign_greedy_legal`].
    BudgetExceeded,
    /// The search exhausted its entire reachable space (heap emptied) with
    /// no full legal assignment reachable at all (e.g. spacing/occupancy
    /// conflicts) and fell back to [`assign_greedy_legal`].
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

#[derive(Clone)]
struct AwareNode {
    /// Negative cost for max-heap as min-cost.
    neg_cost: OrderedFloat,
    assigned: Vec<(Position, Position)>,
    used_pairs: BTreeSet<usize>,
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
        self.neg_cost == other.neg_cost
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
        // BinaryHeap is max-heap; larger neg_cost (less negative) = worse.
        // We store -cost so smaller cost → larger neg_cost → pops first?
        // Actually max-heap pops largest. We want smallest cost first.
        // Store OrderedFloat(-cost): larger (-cost) means smaller cost → pops first. ✓
        self.neg_cost.cmp(&other.neg_cost)
    }
}

/// Expansion budget for the routing-aware search before falling back to the
/// greedy assignment (which always terminates and supports deferral).
pub const AWARE_NODE_BUDGET: usize = 100_000;

/// Routing-aware best-first search ([RAP] Sec. IV-B style): extend by one gate,
/// charge Eq. (1) √(d_max) of the moves implied so far. Extensions are limited
/// to occupancy- and spacing-legal pairs; when no legal full assignment exists
/// (or the node budget is exhausted) it falls back to
/// [`assign_greedy_legal`], which defers unplaceable gates.
fn assign_aware_legal(gates: &[(AtomId, AtomId)], inputs: &AssignInputs<'_>) -> GateAssignment {
    let mut heap = BinaryHeap::new();
    heap.push(AwareNode {
        neg_cost: OrderedFloat(0.0),
        assigned: Vec::new(),
        used_pairs: BTreeSet::new(),
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
        if expansions > AWARE_NODE_BUDGET {
            let mut fallback = assign_greedy_legal(gates, inputs);
            fallback.outcome = AwareSearchOutcome::BudgetExceeded;
            fallback.node_expansions = expansions;
            return fallback;
        }
        let (a, b) = gates[g];
        let pa = atom_position_or_origin(inputs.atom_pos, a);
        let pb = atom_position_or_origin(inputs.atom_pos, b);
        for (i, &(left, right)) in inputs.pairs.iter().enumerate() {
            if node.used_pairs.contains(&i) || !pair_legal(i, (a, b), &node.used_pairs, inputs) {
                continue;
            }
            for orient in [(left, right), (right, left)] {
                let d1 = euclidean_um(pa, orient.0);
                let d2 = euclidean_um(pb, orient.1);
                // Reuse: zero-distance moves don't contribute to d_max.
                let d_max = d1.max(d2);
                let step_cost = sqrt_d_max(d_max);
                let cost_so_far = -node.neg_cost.0 + step_cost;
                let mut used = node.used_pairs.clone();
                used.insert(i);
                let mut assigned = node.assigned.clone();
                assigned.push(orient);
                heap.push(AwareNode {
                    neg_cost: OrderedFloat(-cost_so_far),
                    assigned,
                    used_pairs: used,
                });
            }
        }
    }
    // Heap exhausted: no legal assignment of every gate exists this stage.
    // Place what greedy can and defer the rest.
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
        };

        let greedy = assign_greedy_legal(&gates, &inputs);
        assert_eq!(greedy.outcome, AwareSearchOutcome::NotApplicable);
        assert!(greedy.deferred.is_empty());

        let aware = assign_aware_legal(&gates, &inputs);
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
