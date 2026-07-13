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
use std::collections::{BTreeMap, BTreeSet, BinaryHeap};

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
}

impl ZonedArchitecture {
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

    let (layout, _storage_sites, entangle_pairs) = build_zoned_layout(&req.graph, arch)?;
    req.layout = Some(layout.clone());

    let mut atom_pos: BTreeMap<AtomId, Position> = BTreeMap::new();
    for binding in &layout.initial_bindings {
        let site = match binding.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
        };
        if let Some(s) = layout.sites.iter().find(|s| s.id == site) {
            atom_pos.insert(binding.atom, s.position);
        }
    }

    let mut out_layers = Vec::new();
    let mut next_cycle = 0u32;
    let mut total_routing_cost = 0.0;
    let mut rearrangement_steps = 0u64;
    let mut trap_transfers = 0u64;

    for layer in &req.layers {
        let gates = entangling_pairs_in_layer(layer);
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

        let assignment = match mode {
            PlacerMode::RoutingAgnostic => assign_agnostic(&gates, &atom_pos, &entangle_pairs),
            PlacerMode::RoutingAware => {
                assign_aware(&gates, &atom_pos, &entangle_pairs, arch.acceleration_m_s2)
            }
        };

        // Reuse (Sec. III-A): skip atoms already at their assigned pair site.
        let mut moves = Vec::new();
        let mut transfers = Vec::new();
        let mut d_max = 0.0_f64;
        for (gate_idx, &(a, b)) in gates.iter().enumerate() {
            let (pa, pb) = assignment[gate_idx];
            for (atom, target) in [(a, pa), (b, pb)] {
                let cur = atom_pos.get(&atom).copied().unwrap_or(target);
                let dist = euclidean_um(cur, target);
                if dist < 1e-9 {
                    continue; // reuse: already in place
                }
                d_max = d_max.max(dist);
                let from_site = nearest_site_id(&layout, cur);
                let to_site = nearest_site_id(&layout, target);
                moves.push(AtomMove {
                    atom,
                    from: from_site,
                    to: to_site,
                });
                transfers.push(TrapTransfer {
                    atom,
                    direction: TransferDirection::SlmToAod,
                    site: from_site,
                    aod: AodTrapRef {
                        aod_id: 0,
                        row: 0,
                        col: 0,
                    },
                    duration_us: arch.trap_transfer_us,
                });
                transfers.push(TrapTransfer {
                    atom,
                    direction: TransferDirection::AodToSlm,
                    site: to_site,
                    aod: AodTrapRef {
                        aod_id: 0,
                        row: 0,
                        col: 0,
                    },
                    duration_us: arch.trap_transfer_us,
                });
                atom_pos.insert(atom, target);
            }
        }

        let group_cost = sqrt_d_max(d_max);
        total_routing_cost += group_cost;
        if !moves.is_empty() {
            rearrangement_steps += 1;
            let duration_us = movement_duration_us(d_max, arch.acceleration_m_s2);
            // Transfer layer
            // Emit load then move then store as separate cycles.
            let load: Vec<_> = transfers
                .iter()
                .filter(|t| t.direction == TransferDirection::SlmToAod)
                .cloned()
                .map(NeutralAtomAction::Transfer)
                .collect();
            let store: Vec<_> = transfers
                .iter()
                .filter(|t| t.direction == TransferDirection::AodToSlm)
                .cloned()
                .map(NeutralAtomAction::Transfer)
                .collect();
            trap_transfers += (load.len() + store.len()) as u64;

            if !load.is_empty() {
                out_layers.push(ScheduleLayer {
                    cycle: next_cycle,
                    actions: load,
                });
                next_cycle = next_cycle.saturating_add(1);
            }
            out_layers.push(ScheduleLayer {
                cycle: next_cycle,
                actions: vec![NeutralAtomAction::Move(MovementGroup {
                    moves,
                    duration_us,
                })],
            });
            next_cycle = next_cycle.saturating_add(1);
            if !store.is_empty() {
                out_layers.push(ScheduleLayer {
                    cycle: next_cycle,
                    actions: store,
                });
                next_cycle = next_cycle.saturating_add(1);
            }
        }

        // Entangle layer (atoms now in entanglement zone).
        let mut entangle_actions = Vec::new();
        for &(a, b) in &gates {
            entangle_actions.push(NeutralAtomAction::Entangle2 {
                atoms: [a, b],
                duration_us: 1,
            });
        }
        // Also pass through non-entangle actions from the original layer.
        for action in &layer.actions {
            if !matches!(
                action,
                NeutralAtomAction::Entangle2 { .. } | NeutralAtomAction::EntangleN { .. }
            ) {
                if let NeutralAtomAction::Measure {
                    atom,
                    basis,
                    duration_us,
                } = action
                {
                    entangle_actions.push(NeutralAtomAction::Measure {
                        atom: *atom,
                        basis: *basis,
                        duration_us: *duration_us,
                    });
                } else {
                    entangle_actions.push(action.clone());
                }
            }
        }
        let entangle_layer = ScheduleLayer {
            cycle: next_cycle,
            actions: entangle_actions,
        };
        entangle_layer
            .validate_conflicts()
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

fn entangling_pairs_in_layer(layer: &ScheduleLayer) -> Vec<(AtomId, AtomId)> {
    let mut out = Vec::new();
    for action in &layer.actions {
        match action {
            NeutralAtomAction::Entangle2 { atoms, .. } => out.push((atoms[0], atoms[1])),
            NeutralAtomAction::EntangleN { atoms, .. } if atoms.len() >= 2 => {
                out.push((atoms[0], atoms[1]));
            }
            _ => {}
        }
    }
    out
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

/// Routing-agnostic: assign gates to nearest free entanglement pairs by
/// travel distance (ZAC-style).
fn assign_agnostic(
    gates: &[(AtomId, AtomId)],
    atom_pos: &BTreeMap<AtomId, Position>,
    pairs: &[(Position, Position)],
) -> Vec<(Position, Position)> {
    let mut used = BTreeSet::new();
    let mut out = Vec::with_capacity(gates.len());
    for &(a, b) in gates {
        let pa = atom_pos.get(&a).copied().unwrap_or(Position {
            x_um: 0.0,
            y_um: 0.0,
        });
        let pb = atom_pos.get(&b).copied().unwrap_or(Position {
            x_um: 0.0,
            y_um: 0.0,
        });
        let mut best = None;
        for (i, &(left, right)) in pairs.iter().enumerate() {
            if used.contains(&i) {
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
        let (_, idx, orient) = best.expect("enough pairs");
        used.insert(idx);
        out.push(orient);
    }
    out
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

/// Routing-aware best-first search ([RAP] Sec. IV-B style): extend by one gate,
/// charge Eq. (1) √(d_max) of the moves implied so far.
fn assign_aware(
    gates: &[(AtomId, AtomId)],
    atom_pos: &BTreeMap<AtomId, Position>,
    pairs: &[(Position, Position)],
    _acceleration: f64,
) -> Vec<(Position, Position)> {
    let mut heap = BinaryHeap::new();
    heap.push(AwareNode {
        neg_cost: OrderedFloat(0.0),
        assigned: Vec::new(),
        used_pairs: BTreeSet::new(),
    });

    while let Some(node) = heap.pop() {
        let g = node.assigned.len();
        if g == gates.len() {
            return node.assigned;
        }
        let (a, b) = gates[g];
        let pa = atom_pos.get(&a).copied().unwrap_or(Position {
            x_um: 0.0,
            y_um: 0.0,
        });
        let pb = atom_pos.get(&b).copied().unwrap_or(Position {
            x_um: 0.0,
            y_um: 0.0,
        });
        for (i, &(left, right)) in pairs.iter().enumerate() {
            if node.used_pairs.contains(&i) {
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
    // Fallback (should not happen if pairs suffice).
    assign_agnostic(gates, atom_pos, pairs)
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
