//! Schedule compaction: exclusive-cycle ASAP baseline + greedy merge (#108).
//!
//! # Exclusive-cycle ASAP (engineering baseline)
//!
//! [`asap_schedule_layers`] assigns each input layer the earliest cycle
//! `≥ max(preds)+1`, then **bumps** so at most one layer occupies a cycle.
//! It **never merges** action content. This is an engineering serialization for
//! a merge-free baseline so greedy is the only merge pass — **not**
//! stage-optimal.
//!
//! Independent layers are serialized (makespan can exceed the critical-path
//! lower bound). **True ASAP** (independent work may share a cycle) is what
//! [Enola] Sec. 3 stage-optimality refers to; that notion is **not** the v0
//! baseline API.
//!
//! Cite [Enola] Sec. 3 only for: (1) critical-path **lower bound** reporting
//! (`CriticalPathReport.critical_path_length`), (2) describing true ASAP, and
//! (3) the optional note that on **dependency chains** exclusive-cycle and
//! true ASAP coincide numerically. Do **not** claim that exclusive-cycle ASAP
//! is Enola-optimal, or that its stage count equals the Enola / critical-path
//! lower bound except as that chain coincidence.
//!
//! # Dual ASAP note (N1)
//!
//! `#108` exclusive-cycle ASAP is a **physical-layer engineering** baseline over
//! schedule layers. It does **not** replace `#105` `asap_buckets` / Enola
//! interaction ASAP on dependency-DAG segments.
//!
//! # Greedy compaction
//!
//! [`compact_schedule`] runs exclusive-cycle ASAP, then greedily merges within
//! **allowed merge classes** (v0: primarily **E0** entangle-only layers on
//! disjoint atoms). Compaction gains recover legal E0 parallelism that
//! exclusive-cycle deferred. Transfer / placeholder-AOD merges are forbidden
//! (B5; #107 emits placeholder `AodTrapRef { 0,0,0 }`).
//!
//! RAP Sec. III-A is reuse analysis only — not an ASAP citation. Do not claim
//! Enola Thm. 1 or RAP Eq. (1) optimality for this heuristic.
//!
//! # Feed-forward trust boundary (B4)
//!
//! [`infer_atom_dependencies`] never invents [`ScheduleDependencyKind::FeedForward`].
//! Without caller-supplied FeedForward edges, compaction cannot claim classical
//! control safety when correction atoms differ from the measured atom.
//!
//! # Physical legality (B3)
//!
//! When `layout` and [`CompactionOptions::legality`] are present, position-aware
//! R2/R3 geometry checks are **mandatory** on entangle merges (MLIR-free).
//! Zone re-validate is best-effort on static bindings for entangle-only merges.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::graph::InteractionId;
use crate::layout::{AodTrapRef, AtomId, NeutralAtomLayout, Position, TrapBinding};
use crate::schedule::{NeutralAtomAction, ScheduleLayer};
use crate::schedule_entry::GraphScheduleRequest;
use crate::zoned::{ZonedArchitecture, validate_zone_constraints};

/// Hard ordering constraints that compaction must never violate.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScheduleDependency {
    /// Predecessor layer index in the input schedule (pre-compaction).
    pub before: u32,
    /// Successor layer index that may not move earlier than `before` completes.
    pub after: u32,
    pub kind: ScheduleDependencyKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScheduleDependencyKind {
    /// Same atom appears in both layers (data dependence).
    AtomHazard,
    /// Explicit barrier / segment boundary.
    Barrier,
    /// Mid-circuit measurement must complete before dependent correction.
    Measurement,
    /// Classical feed-forward: measure → conditional correction (caller-supplied).
    FeedForward,
}

/// Critical-path metadata over the post-compaction layer DAG.
///
/// `critical_path_length` is the Enola Sec. 3 **lower-bound** style longest-path
/// length on dependency edges (a DAG property). It is independent of whether
/// exclusive-cycle ASAP serialized independent work.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CriticalPathReport {
    /// Number of cycles = max(cycle)+1 after renumber / compaction.
    pub makespan_cycles: u32,
    /// Longest dependency-chain length in **post-merge cycle vertices** (N3).
    pub critical_path_length: u32,
    /// Stable **pre-merge input layer indices** that lie on some longest path
    /// (layers that were merged share the successor's cycle but keep ids listed).
    pub critical_layer_indices: Vec<u32>,
    /// Best-effort interaction ids when layers map cleanly to entangles.
    pub critical_interaction_ids: Vec<InteractionId>,
}

/// Result of exclusive-cycle ASAP and optional greedy compaction.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompactionResult {
    pub request: GraphScheduleRequest,
    pub asap_makespan_cycles: u32,
    pub compacted_makespan_cycles: u32,
    pub critical_path: CriticalPathReport,
    /// True if greedy pass merged at least one pair of layers.
    pub compacted: bool,
}

/// Errors from schedule compaction (#108).
#[derive(Debug, Error, Clone, PartialEq)]
pub enum CompactionError {
    #[error("empty schedule")]
    EmptySchedule,
    #[error("schedule layer conflict after compaction: {0}")]
    Conflict(String),
    #[error("occupancy conflict after compaction: {0}")]
    Occupancy(String),
    #[error("zone constraint violated after compaction: {0}")]
    Zone(String),
    #[error("physical legality violated after compaction: {0}")]
    PhysicalLegality(String),
    #[error("merge class forbidden in v0: {0}")]
    ForbiddenMergeClass(String),
    #[error("feed-forward / measurement dependency would be violated")]
    DependencyViolation,
    #[error("invalid dependency edge {0:?} → {1:?}")]
    InvalidDependency(u32, u32),
    #[error("layout required for position-aware legality")]
    LayoutRequired,
}

/// Geometry / AOD limits for post-merge physical legality.
#[derive(Clone, Debug, PartialEq)]
pub struct LegalityLimits {
    pub rydberg_range_um: f64,
    pub min_rydberg_spacing_um: f64,
    pub aod_min_separation_um: f64,
}

/// Options for [`compact_schedule`].
#[derive(Clone, Debug, Default)]
pub struct CompactionOptions {
    /// When set with layout, run zone checks after compaction (best-effort; see B3).
    pub arch: Option<ZonedArchitecture>,
    /// When set **with** `request.layout`, enable mandatory position-aware R2/R3.
    pub legality: Option<LegalityLimits>,
    /// If true, run greedy compaction after ASAP; if false, ASAP-only baseline.
    pub greedy: bool,
}

/// Placeholder AOD ref emitted by #107 `schedule_zoned` (all zeros).
const PLACEHOLDER_AOD: AodTrapRef = AodTrapRef {
    aod_id: 0,
    row: 0,
    col: 0,
};

/// Infer AtomHazard deps only. Does **not** invent FeedForward (B4).
pub fn infer_atom_dependencies(layers: &[ScheduleLayer]) -> Vec<ScheduleDependency> {
    let mut last_use: BTreeMap<AtomId, u32> = BTreeMap::new();
    let mut deps = Vec::new();
    let mut seen: BTreeSet<(u32, u32, ScheduleDependencyKind)> = BTreeSet::new();

    for (idx, layer) in layers.iter().enumerate() {
        let i = idx as u32;
        let atoms = layer_atoms(layer);
        for atom in atoms {
            if let Some(&pred) = last_use.get(&atom) {
                let key = (pred, i, ScheduleDependencyKind::AtomHazard);
                if seen.insert(key) {
                    deps.push(ScheduleDependency {
                        before: pred,
                        after: i,
                        kind: ScheduleDependencyKind::AtomHazard,
                    });
                }
            }
            last_use.insert(atom, i);
        }
    }
    deps
}

/// Build measure→correction FeedForward edges from an explicit pairing list.
pub fn feed_forward_dependencies(
    measure_layer: u32,
    correction_layers: &[u32],
) -> Vec<ScheduleDependency> {
    correction_layers
        .iter()
        .map(|&after| ScheduleDependency {
            before: measure_layer,
            after,
            kind: ScheduleDependencyKind::FeedForward,
        })
        .collect()
}

/// Exclusive-cycle ASAP: earliest ≥ preds+1, one layer/cycle; **never merges** (B1).
///
/// Engineering baseline — **not** Enola-optimal true ASAP. On a pure dependency
/// chain, exclusive-cycle and true ASAP coincide numerically with the critical-path
/// lower bound; that is coincidence from lack of independent work, not an
/// Enola-optimality claim for this algorithm.
pub fn asap_schedule_layers(
    req: GraphScheduleRequest,
    deps: &[ScheduleDependency],
) -> Result<CompactionResult, CompactionError> {
    if req.layers.is_empty() {
        return Err(CompactionError::EmptySchedule);
    }
    let merged_deps = merge_deps(&req.layers, deps)?;
    let (layers, asap_makespan, lineage) = exclusive_cycle_asap(&req.layers, &merged_deps)?;
    let critical_path = critical_path_report(&layers, &lineage, &merged_deps, asap_makespan);
    Ok(CompactionResult {
        request: GraphScheduleRequest {
            graph: req.graph,
            layers,
            layout: req.layout,
        },
        asap_makespan_cycles: asap_makespan,
        compacted_makespan_cycles: asap_makespan,
        critical_path,
        compacted: false,
    })
}

/// Exclusive-cycle ASAP then greedy merge within allowed classes; re-verify legality.
pub fn compact_schedule(
    req: GraphScheduleRequest,
    deps: &[ScheduleDependency],
    opts: &CompactionOptions,
) -> Result<CompactionResult, CompactionError> {
    if req.layers.is_empty() {
        return Err(CompactionError::EmptySchedule);
    }
    let merged_deps = merge_deps(&req.layers, deps)?;
    let (mut layers, asap_makespan, mut lineage) = exclusive_cycle_asap(&req.layers, &merged_deps)?;
    let mut compacted = false;

    if opts.greedy {
        loop {
            let before_makespan = makespan(&layers);
            let Some((keep, drop)) =
                find_first_improving_merge(&layers, &lineage, &merged_deps, &req.layout, opts)?
            else {
                break;
            };
            apply_merge(&mut layers, &mut lineage, keep, drop)?;
            renumber_dense(&mut layers);
            compacted = true;
            let after_makespan = makespan(&layers);
            if after_makespan >= before_makespan {
                // Should not happen if find_first only returns improving merges.
                break;
            }
        }
    }

    // Final software + physical validation on all layers.
    for layer in &layers {
        validate_layer_software(layer)?;
        if let (Some(limits), Some(layout)) = (&opts.legality, &req.layout)
            && layer_has_entangle(layer)
        {
            verify_entangling_geometry_mlir_free(layer, layout, limits)?;
        }
    }
    if let (Some(arch), Some(layout)) = (&opts.arch, &req.layout) {
        validate_zone_constraints(&layers, layout, arch)
            .map_err(|e| CompactionError::Zone(e.to_string()))?;
    }

    let compacted_makespan = makespan(&layers);
    let critical_path = critical_path_report(&layers, &lineage, &merged_deps, compacted_makespan);

    Ok(CompactionResult {
        request: GraphScheduleRequest {
            graph: req.graph,
            layers,
            layout: req.layout,
        },
        asap_makespan_cycles: asap_makespan,
        compacted_makespan_cycles: compacted_makespan,
        critical_path,
        compacted,
    })
}

// ---------------------------------------------------------------------------
// Dependency DAG
// ---------------------------------------------------------------------------

fn merge_deps(
    layers: &[ScheduleLayer],
    caller: &[ScheduleDependency],
) -> Result<Vec<ScheduleDependency>, CompactionError> {
    let n = layers.len() as u32;
    let mut deps = infer_atom_dependencies(layers);
    let mut seen: BTreeSet<(u32, u32, ScheduleDependencyKind)> =
        deps.iter().map(|d| (d.before, d.after, d.kind)).collect();

    for d in caller {
        if d.before >= n || d.after >= n || d.before == d.after {
            return Err(CompactionError::InvalidDependency(d.before, d.after));
        }
        let key = (d.before, d.after, d.kind);
        if seen.insert(key) {
            deps.push(d.clone());
        }
        // Barrier cut: every pre-barrier layer → barrier, and barrier → every post.
        if d.kind == ScheduleDependencyKind::Barrier {
            for i in 0..d.before {
                let key = (i, d.before, ScheduleDependencyKind::Barrier);
                if seen.insert(key) {
                    deps.push(ScheduleDependency {
                        before: i,
                        after: d.before,
                        kind: ScheduleDependencyKind::Barrier,
                    });
                }
            }
            for j in (d.before + 1)..n {
                let key = (d.before, j, ScheduleDependencyKind::Barrier);
                if seen.insert(key) {
                    deps.push(ScheduleDependency {
                        before: d.before,
                        after: j,
                        kind: ScheduleDependencyKind::Barrier,
                    });
                }
            }
        }
    }
    Ok(deps)
}

fn preds_map(n: usize, deps: &[ScheduleDependency]) -> Vec<Vec<usize>> {
    let mut preds = vec![Vec::new(); n];
    for d in deps {
        let b = d.before as usize;
        let a = d.after as usize;
        if b < n && a < n && !preds[a].contains(&b) {
            preds[a].push(b);
        }
    }
    for p in &mut preds {
        p.sort_unstable();
    }
    preds
}

// ---------------------------------------------------------------------------
// Exclusive-cycle ASAP
// ---------------------------------------------------------------------------

type AsapSchedule = (Vec<ScheduleLayer>, u32, Vec<Vec<u32>>);

fn exclusive_cycle_asap(
    layers: &[ScheduleLayer],
    deps: &[ScheduleDependency],
) -> Result<AsapSchedule, CompactionError> {
    let n = layers.len();
    let preds = preds_map(n, deps);
    let mut asap = vec![0u32; n];
    let mut occupied: BTreeSet<u32> = BTreeSet::new();

    for i in 0..n {
        let mut t = preds[i]
            .iter()
            .map(|&p| asap[p].saturating_add(1))
            .max()
            .unwrap_or(0);
        while occupied.contains(&t) {
            t = t.saturating_add(1);
        }
        asap[i] = t;
        occupied.insert(t);
    }

    let mut out = Vec::with_capacity(n);
    let mut lineage = Vec::with_capacity(n);
    for i in 0..n {
        out.push(ScheduleLayer {
            cycle: asap[i],
            actions: layers[i].actions.clone(),
        });
        lineage.push(vec![i as u32]);
    }
    // Keep layers in input order (cycles may be non-monotonic in index order
    // only if deps force it; exclusive-cycle with increasing i keeps cycles
    // non-decreasing when deps are forward). Sort by cycle for stable output.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (asap[i], i));
    let layers_sorted: Vec<ScheduleLayer> = order.iter().map(|&i| out[i].clone()).collect();
    let lineage_sorted: Vec<Vec<u32>> = order.iter().map(|&i| lineage[i].clone()).collect();
    let makespan = asap.iter().copied().max().unwrap_or(0).saturating_add(1);
    Ok((layers_sorted, makespan, lineage_sorted))
}

fn makespan(layers: &[ScheduleLayer]) -> u32 {
    layers
        .iter()
        .map(|l| l.cycle)
        .max()
        .map(|c| c.saturating_add(1))
        .unwrap_or(0)
}

fn renumber_dense(layers: &mut [ScheduleLayer]) {
    let mut cycles: Vec<u32> = layers.iter().map(|l| l.cycle).collect();
    cycles.sort_unstable();
    cycles.dedup();
    let map: BTreeMap<u32, u32> = cycles
        .into_iter()
        .enumerate()
        .map(|(i, c)| (c, i as u32))
        .collect();
    for layer in layers.iter_mut() {
        if let Some(&new_c) = map.get(&layer.cycle) {
            layer.cycle = new_c;
        }
    }
}

// ---------------------------------------------------------------------------
// Critical path (N3)
// ---------------------------------------------------------------------------

fn critical_path_report(
    layers: &[ScheduleLayer],
    lineage: &[Vec<u32>],
    original_deps: &[ScheduleDependency],
    makespan_cycles: u32,
) -> CriticalPathReport {
    let n = layers.len();
    // Map pre-merge index → current vertex.
    let mut pre_to_vertex: BTreeMap<u32, usize> = BTreeMap::new();
    for (v, ids) in lineage.iter().enumerate() {
        for &id in ids {
            pre_to_vertex.insert(id, v);
        }
    }

    // Build post-merge DAG: edge u→v if some original dep crosses vertices.
    let mut preds = vec![Vec::new(); n];
    let mut succs = vec![Vec::new(); n];
    let mut edge_seen: BTreeSet<(usize, usize)> = BTreeSet::new();
    for d in original_deps {
        let Some(&u) = pre_to_vertex.get(&d.before) else {
            continue;
        };
        let Some(&v) = pre_to_vertex.get(&d.after) else {
            continue;
        };
        if u == v {
            continue;
        }
        if edge_seen.insert((u, v)) {
            preds[v].push(u);
            succs[u].push(v);
        }
    }

    // Longest-path length in vertices (number of vertices on a longest chain).
    // Enola Sec. 3 lower bound style: chain of k dependent layers → length k.
    let mut longest_from = vec![1u32; n];
    // Process in topological order by cycle then index.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| (layers[i].cycle, i));
    for &i in &order {
        let best_pred = preds[i].iter().map(|&p| longest_from[p]).max().unwrap_or(0);
        longest_from[i] = best_pred.saturating_add(1);
    }
    let critical_path_length = longest_from.iter().copied().max().unwrap_or(0);

    // Mark critical: vertices on some longest path.
    let mut longest_suffix = vec![0u32; n];
    for &i in order.iter().rev() {
        let suffix = succs[i]
            .iter()
            .map(|&s| longest_suffix[s])
            .max()
            .unwrap_or(0);
        longest_suffix[i] = suffix.saturating_add(1);
    }
    // A vertex is critical if longest_from[i] + longest_suffix[i] - 1 == global max.
    let mut critical_layer_indices = Vec::new();
    for i in 0..n {
        if longest_from[i]
            .saturating_add(longest_suffix[i])
            .saturating_sub(1)
            == critical_path_length
        {
            critical_layer_indices.extend(lineage[i].iter().copied());
        }
    }
    critical_layer_indices.sort_unstable();
    critical_layer_indices.dedup();

    // Best-effort: no stable InteractionId on ScheduleLayer actions in v0.
    let critical_interaction_ids = Vec::new();

    CriticalPathReport {
        makespan_cycles,
        critical_path_length,
        critical_layer_indices,
        critical_interaction_ids,
    }
}

// ---------------------------------------------------------------------------
// Layer / action helpers
// ---------------------------------------------------------------------------

fn layer_atoms(layer: &ScheduleLayer) -> BTreeSet<AtomId> {
    let mut atoms = BTreeSet::new();
    for action in &layer.actions {
        match action {
            NeutralAtomAction::Move(group) => {
                for m in &group.moves {
                    atoms.insert(m.atom);
                }
            }
            NeutralAtomAction::Transfer(t) => {
                atoms.insert(t.atom);
            }
            NeutralAtomAction::Entangle2 { atoms: a, .. } => {
                atoms.insert(a[0]);
                atoms.insert(a[1]);
            }
            NeutralAtomAction::EntangleN { atoms: a, .. } => {
                for &atom in a {
                    atoms.insert(atom);
                }
            }
            NeutralAtomAction::Measure { atom, .. } | NeutralAtomAction::Reset { atom, .. } => {
                atoms.insert(*atom);
            }
            NeutralAtomAction::Wait { .. } => {}
        }
    }
    atoms
}

fn layer_has_entangle(layer: &ScheduleLayer) -> bool {
    layer.actions.iter().any(|a| {
        matches!(
            a,
            NeutralAtomAction::Entangle2 { .. } | NeutralAtomAction::EntangleN { .. }
        )
    })
}

fn layer_has_transfer(layer: &ScheduleLayer) -> bool {
    layer
        .actions
        .iter()
        .any(|a| matches!(a, NeutralAtomAction::Transfer(_)))
}

fn layer_has_move(layer: &ScheduleLayer) -> bool {
    layer
        .actions
        .iter()
        .any(|a| matches!(a, NeutralAtomAction::Move(_)))
}

fn layer_has_measure_or_reset(layer: &ScheduleLayer) -> bool {
    layer.actions.iter().any(|a| {
        matches!(
            a,
            NeutralAtomAction::Measure { .. } | NeutralAtomAction::Reset { .. }
        )
    })
}

fn is_entangle_only(layer: &ScheduleLayer) -> bool {
    !layer.actions.is_empty()
        && layer.actions.iter().all(|a| {
            matches!(
                a,
                NeutralAtomAction::Entangle2 { .. }
                    | NeutralAtomAction::EntangleN { .. }
                    | NeutralAtomAction::Wait { .. }
            )
        })
        && layer_has_entangle(layer)
}

fn is_move_only(layer: &ScheduleLayer) -> bool {
    !layer.actions.is_empty()
        && layer.actions.iter().all(|a| {
            matches!(
                a,
                NeutralAtomAction::Move(_) | NeutralAtomAction::Wait { .. }
            )
        })
        && layer_has_move(layer)
}

fn uses_placeholder_aod(layer: &ScheduleLayer) -> bool {
    let mut saw_transfer = false;
    for action in &layer.actions {
        if let NeutralAtomAction::Transfer(t) = action {
            saw_transfer = true;
            if t.aod != PLACEHOLDER_AOD {
                return false;
            }
        }
    }
    saw_transfer
}

fn validate_layer_software(layer: &ScheduleLayer) -> Result<(), CompactionError> {
    layer
        .validate_conflicts()
        .map_err(|e| CompactionError::Conflict(e.to_string()))?;
    layer
        .validate_occupancy()
        .map_err(|e| CompactionError::Occupancy(e.to_string()))?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Merge class gate (B5)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MergeClass {
    /// Entangle-only ∪ entangle-only.
    E0,
    /// Move-only ∥ entangle-only (requires non-placeholder AOD).
    M0,
    Forbidden,
}

fn classify_merge(a: &ScheduleLayer, b: &ScheduleLayer) -> MergeClass {
    if layer_has_transfer(a) || layer_has_transfer(b) {
        return MergeClass::Forbidden;
    }
    if is_entangle_only(a) && is_entangle_only(b) {
        return MergeClass::E0;
    }
    if (is_move_only(a) && is_entangle_only(b)) || (is_entangle_only(a) && is_move_only(b)) {
        return MergeClass::M0;
    }
    // Measure / Reset / mixed → forbidden in v0.
    if layer_has_measure_or_reset(a) || layer_has_measure_or_reset(b) {
        return MergeClass::Forbidden;
    }
    MergeClass::Forbidden
}

fn hard_dep_forbids_same_cycle(
    lineage_i: &[u32],
    lineage_j: &[u32],
    deps: &[ScheduleDependency],
) -> bool {
    let set_i: BTreeSet<u32> = lineage_i.iter().copied().collect();
    let set_j: BTreeSet<u32> = lineage_j.iter().copied().collect();
    for d in deps {
        let hard = matches!(
            d.kind,
            ScheduleDependencyKind::Barrier
                | ScheduleDependencyKind::Measurement
                | ScheduleDependencyKind::FeedForward
        );
        if !hard {
            continue;
        }
        let crosses = (set_i.contains(&d.before) && set_j.contains(&d.after))
            || (set_j.contains(&d.before) && set_i.contains(&d.after));
        if crosses {
            return true;
        }
    }
    // Also AtomHazard: merging into same cycle is ok only if atoms disjoint —
    // checked separately via validate_conflicts. Soft AtomHazard across layers
    // with disjoint atoms after merge is fine; shared atoms fail conflicts.
    false
}

// ---------------------------------------------------------------------------
// Position-aware R2/R3 (MLIR-free)
// ---------------------------------------------------------------------------

fn atom_position(layout: &NeutralAtomLayout, atom: AtomId) -> Result<Position, CompactionError> {
    let binding = layout
        .initial_bindings
        .iter()
        .find(|b| b.atom == atom)
        .ok_or_else(|| {
            CompactionError::PhysicalLegality(format!("missing binding for atom {atom:?}"))
        })?;
    let site = match binding.trap {
        TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => site,
    };
    layout
        .sites
        .iter()
        .find(|s| s.id == site)
        .map(|s| s.position)
        .ok_or_else(|| {
            CompactionError::PhysicalLegality(format!("missing site {site:?} for atom {atom:?}"))
        })
}

fn distance_um(a: &Position, b: &Position) -> f64 {
    let dx = a.x_um - b.x_um;
    let dy = a.y_um - b.y_um;
    (dx * dx + dy * dy).sqrt()
}

fn atom_pair_key(a: AtomId, b: AtomId) -> (AtomId, AtomId) {
    if a <= b { (a, b) } else { (b, a) }
}

/// Port of dialect `verify_entangling_geometry` — MLIR-free R2/R3.
fn verify_entangling_geometry_mlir_free(
    layer: &ScheduleLayer,
    layout: &NeutralAtomLayout,
    limits: &LegalityLimits,
) -> Result<(), CompactionError> {
    let mut partner_pairs = BTreeSet::new();
    let mut atoms: Vec<(AtomId, Position)> = Vec::new();

    for action in &layer.actions {
        match action {
            NeutralAtomAction::Entangle2 { atoms: pair, .. } => {
                let p0 = atom_position(layout, pair[0])?;
                let p1 = atom_position(layout, pair[1])?;
                let d = distance_um(&p0, &p1);
                if d > limits.rydberg_range_um {
                    return Err(CompactionError::PhysicalLegality(format!(
                        "cycle {}: entangling pair ({:?}, {:?}) distance {d} um beyond Rydberg range {}",
                        layer.cycle, pair[0], pair[1], limits.rydberg_range_um
                    )));
                }
                partner_pairs.insert(atom_pair_key(pair[0], pair[1]));
                atoms.push((pair[0], p0));
                atoms.push((pair[1], p1));
            }
            NeutralAtomAction::EntangleN { atoms: group, .. } => {
                // Treat as complete partner graph among the group for R2 range;
                // non-partners across different EntangleN/2 still checked below.
                for i in 0..group.len() {
                    for j in (i + 1)..group.len() {
                        let p0 = atom_position(layout, group[i])?;
                        let p1 = atom_position(layout, group[j])?;
                        let d = distance_um(&p0, &p1);
                        if d > limits.rydberg_range_um {
                            return Err(CompactionError::PhysicalLegality(format!(
                                "cycle {}: entangling pair ({:?}, {:?}) distance {d} um beyond Rydberg range {}",
                                layer.cycle, group[i], group[j], limits.rydberg_range_um
                            )));
                        }
                        partner_pairs.insert(atom_pair_key(group[i], group[j]));
                    }
                }
                for &atom in group {
                    let pos = atom_position(layout, atom)?;
                    atoms.push((atom, pos));
                }
            }
            _ => {}
        }
    }

    for i in 0..atoms.len() {
        for j in (i + 1)..atoms.len() {
            let (a_id, a_pos) = &atoms[i];
            let (b_id, b_pos) = &atoms[j];
            if a_id == b_id {
                continue;
            }
            if partner_pairs.contains(&atom_pair_key(*a_id, *b_id)) {
                continue;
            }
            let d = distance_um(a_pos, b_pos);
            if d <= limits.rydberg_range_um {
                return Err(CompactionError::PhysicalLegality(format!(
                    "cycle {}: non-partner atoms ({:?}, {:?}) distance {d} um within compulsory Rydberg range {}",
                    layer.cycle, a_id, b_id, limits.rydberg_range_um
                )));
            }
            if d <= limits.min_rydberg_spacing_um {
                return Err(CompactionError::PhysicalLegality(format!(
                    "cycle {}: non-partner atoms ({:?}, {:?}) distance {d} um within min Rydberg spacing {}",
                    layer.cycle, a_id, b_id, limits.min_rydberg_spacing_um
                )));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Greedy merge
// ---------------------------------------------------------------------------

fn find_first_improving_merge(
    layers: &[ScheduleLayer],
    lineage: &[Vec<u32>],
    deps: &[ScheduleDependency],
    layout: &Option<NeutralAtomLayout>,
    opts: &CompactionOptions,
) -> Result<Option<(usize, usize)>, CompactionError> {
    let n = layers.len();
    if n < 2 {
        return Ok(None);
    }

    // Candidates sorted by (cycle[j], cycle[i], i, j); prefer adjacent cycles.
    let mut candidates: Vec<(u32, u32, usize, usize)> = Vec::new();
    for i in 0..n {
        for j in (i + 1)..n {
            let (a, b) = if layers[i].cycle <= layers[j].cycle {
                (i, j)
            } else {
                (j, i)
            };
            // a has cycle <= b
            candidates.push((layers[b].cycle, layers[a].cycle, a, b));
        }
    }
    candidates.sort_unstable();

    let before_makespan = makespan(layers);

    for &(_, _, i, j) in &candidates {
        match try_merge_pair(layers, lineage, deps, layout, opts, i, j)? {
            MergeAttempt::Ok => {
                // Simulate makespan after merge + dense renumber.
                let mut sim_layers = layers.to_vec();
                let mut sim_lineage = lineage.to_vec();
                apply_merge(&mut sim_layers, &mut sim_lineage, i, j)?;
                renumber_dense(&mut sim_layers);
                if makespan(&sim_layers) < before_makespan {
                    return Ok(Some((i, j)));
                }
            }
            MergeAttempt::Skip => {}
            MergeAttempt::HardFail(err) => {
                // ForbiddenMergeClass / DependencyViolation when the pair is
                // structurally illegal — skip for greedy scan (do not abort
                // the whole compaction). Callers that force a merge get the error.
                let _ = err;
            }
        }
    }
    Ok(None)
}

enum MergeAttempt {
    Ok,
    Skip,
    HardFail(CompactionError),
}

fn try_merge_pair(
    layers: &[ScheduleLayer],
    lineage: &[Vec<u32>],
    deps: &[ScheduleDependency],
    layout: &Option<NeutralAtomLayout>,
    opts: &CompactionOptions,
    i: usize,
    j: usize,
) -> Result<MergeAttempt, CompactionError> {
    if hard_dep_forbids_same_cycle(&lineage[i], &lineage[j], deps) {
        return Ok(MergeAttempt::HardFail(CompactionError::DependencyViolation));
    }
    // Soft AtomHazard between layers: only merge if atoms become conflict-free
    // (disjoint entangling atoms). any_dep_edge with AtomHazard alone does not
    // forbid if validate_conflicts passes.

    let class = classify_merge(&layers[i], &layers[j]);
    match class {
        MergeClass::Forbidden => {
            if layer_has_transfer(&layers[i]) || layer_has_transfer(&layers[j]) {
                return Ok(MergeAttempt::HardFail(
                    CompactionError::ForbiddenMergeClass(
                        "Transfer layers cannot be merged in v0 (placeholder AOD / T*)".into(),
                    ),
                ));
            }
            return Ok(MergeAttempt::Skip);
        }
        MergeClass::M0 => {
            // Fail closed unless non-placeholder AOD on move side — v0 default
            // has no real AOD on Move (moves lack AodTrapRef). Reject M0.
            return Ok(MergeAttempt::HardFail(
                CompactionError::ForbiddenMergeClass(
                    "M0 move∥entangle requires non-placeholder AOD metadata".into(),
                ),
            ));
        }
        MergeClass::E0 => {}
    }

    if uses_placeholder_aod(&layers[i]) || uses_placeholder_aod(&layers[j]) {
        return Ok(MergeAttempt::HardFail(
            CompactionError::ForbiddenMergeClass(
                "placeholder AOD transfers cannot be merged".into(),
            ),
        ));
    }

    // Zone: reject merges that include Move/Transfer changing occupancy.
    if opts.arch.is_some() && (layer_has_move(&layers[i]) || layer_has_move(&layers[j])) {
        return Ok(MergeAttempt::HardFail(
            CompactionError::ForbiddenMergeClass(
                "zone-aware merge of Move/Transfer rejected without motion simulator".into(),
            ),
        ));
    }

    let mut union = ScheduleLayer {
        cycle: layers[i].cycle.min(layers[j].cycle),
        actions: layers[i].actions.clone(),
    };
    union.actions.extend(layers[j].actions.iter().cloned());

    if let Err(e) = union.validate_conflicts() {
        let _ = e;
        return Ok(MergeAttempt::Skip);
    }
    if let Err(e) = union.validate_occupancy() {
        let _ = e;
        return Ok(MergeAttempt::Skip);
    }

    // Position-aware R2/R3 when legality + layout present.
    if let Some(limits) = &opts.legality {
        let Some(layout) = layout.as_ref() else {
            return Ok(MergeAttempt::HardFail(CompactionError::LayoutRequired));
        };
        if layer_has_entangle(&union)
            && let Err(e) = verify_entangling_geometry_mlir_free(&union, layout, limits)
        {
            return Ok(MergeAttempt::HardFail(e));
        }
    }

    if let (Some(arch), Some(layout)) = (&opts.arch, layout.as_ref()) {
        // Best-effort zone check on entangle-only union using static bindings.
        let probe = [union.clone()];
        if let Err(e) = validate_zone_constraints(&probe, layout, arch) {
            return Ok(MergeAttempt::HardFail(CompactionError::Zone(e.to_string())));
        }
    }

    Ok(MergeAttempt::Ok)
}

fn apply_merge(
    layers: &mut Vec<ScheduleLayer>,
    lineage: &mut Vec<Vec<u32>>,
    keep: usize,
    drop: usize,
) -> Result<(), CompactionError> {
    if keep == drop || keep >= layers.len() || drop >= layers.len() {
        return Err(CompactionError::Conflict("invalid merge indices".into()));
    }
    let (lo, hi) = if keep < drop {
        (keep, drop)
    } else {
        (drop, keep)
    };
    // Always keep the lower index slot; union into keep's cycle min.
    let keep_idx = keep;
    let drop_idx = drop;

    let mut union_actions = layers[keep_idx].actions.clone();
    union_actions.extend(layers[drop_idx].actions.iter().cloned());
    let cycle = layers[keep_idx].cycle.min(layers[drop_idx].cycle);
    layers[keep_idx] = ScheduleLayer {
        cycle,
        actions: union_actions,
    };
    let mut merged_lineage = lineage[keep_idx].clone();
    merged_lineage.extend(lineage[drop_idx].iter().copied());
    merged_lineage.sort_unstable();
    merged_lineage.dedup();
    lineage[keep_idx] = merged_lineage;

    layers.remove(drop_idx);
    lineage.remove(drop_idx);
    // If drop < keep, keep index shifted — but we already wrote into keep_idx
    // before remove; when drop < keep, remove shifts keep down.
    if drop_idx < keep_idx {
        // keep_idx content is now at keep_idx - 1; already correct after remove
        // because we mutated keep_idx before removing a lower index... wait:
        // if drop < keep, we mutated layers[keep], then remove(drop) shifts
        // keep down by 1 — the mutated layer moves to keep-1. Good.
        let _ = lo;
        let _ = hi;
    }
    Ok(())
}

/// Public helper for tests: attempt a forced merge of two layer indices after ASAP.
#[cfg(test)]
pub(crate) fn force_merge_layers(
    req: GraphScheduleRequest,
    deps: &[ScheduleDependency],
    i: usize,
    j: usize,
    opts: &CompactionOptions,
) -> Result<CompactionResult, CompactionError> {
    let merged_deps = merge_deps(&req.layers, deps)?;
    let (mut layers, asap_makespan, mut lineage) = exclusive_cycle_asap(&req.layers, &merged_deps)?;
    match try_merge_pair(&layers, &lineage, &merged_deps, &req.layout, opts, i, j)? {
        MergeAttempt::Ok => {}
        MergeAttempt::Skip => {
            return Err(CompactionError::Conflict("merge skipped".into()));
        }
        MergeAttempt::HardFail(e) => return Err(e),
    }
    apply_merge(&mut layers, &mut lineage, i, j)?;
    renumber_dense(&mut layers);
    let compacted_makespan = makespan(&layers);
    let critical_path = critical_path_report(&layers, &lineage, &merged_deps, compacted_makespan);
    Ok(CompactionResult {
        request: GraphScheduleRequest {
            graph: req.graph,
            layers,
            layout: req.layout,
        },
        asap_makespan_cycles: asap_makespan,
        compacted_makespan_cycles: compacted_makespan,
        critical_path,
        compacted: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{DEFAULT_GAMMA, InteractionGraph, LogicalQubitId};
    use crate::layout::{AtomBinding, AtomSite, SiteId};
    use crate::schedule::{
        AtomMove, MeasurementBasis, MovementGroup, TransferDirection, TrapTransfer,
    };
    use crate::schedule_entry::schedule_from_graph;
    use crate::zoned::{ZoneKind, toy_zoned_architecture};

    fn empty_req(n_qubits: u32) -> GraphScheduleRequest {
        let vertices: Vec<_> = (0..n_qubits).map(LogicalQubitId).collect();
        let graph = InteractionGraph::from_interactions(vertices, vec![], vec![], DEFAULT_GAMMA)
            .expect("empty graph");
        schedule_from_graph(graph).expect("stub")
    }

    fn entangle(a: u32, b: u32) -> NeutralAtomAction {
        NeutralAtomAction::Entangle2 {
            atoms: [AtomId(a), AtomId(b)],
            duration_us: 1,
        }
    }

    fn measure(a: u32) -> NeutralAtomAction {
        NeutralAtomAction::Measure {
            atom: AtomId(a),
            basis: MeasurementBasis::Z,
            duration_us: 1,
        }
    }

    fn layer(cycle: u32, actions: Vec<NeutralAtomAction>) -> ScheduleLayer {
        ScheduleLayer { cycle, actions }
    }

    #[test]
    fn empty_schedule_errors() {
        let req = empty_req(2);
        let err = asap_schedule_layers(req, &[]).unwrap_err();
        assert!(matches!(err, CompactionError::EmptySchedule));
    }

    #[test]
    fn feed_forward_not_inferred() {
        let layers = vec![layer(0, vec![measure(0)]), layer(1, vec![entangle(2, 3)])];
        let deps = infer_atom_dependencies(&layers);
        assert!(
            deps.iter()
                .all(|d| d.kind != ScheduleDependencyKind::FeedForward)
        );
        assert!(deps.is_empty(), "disjoint atoms → no AtomHazard either");
    }

    #[test]
    fn infer_atom_dependencies_shared_atom() {
        let layers = vec![
            layer(0, vec![entangle(0, 1)]),
            layer(1, vec![entangle(1, 2)]),
        ];
        let deps = infer_atom_dependencies(&layers);
        assert_eq!(deps.len(), 1);
        assert_eq!(deps[0].before, 0);
        assert_eq!(deps[0].after, 1);
        assert_eq!(deps[0].kind, ScheduleDependencyKind::AtomHazard);
    }

    #[test]
    fn asap_dependency_chain_matches_critical_path() {
        // Chain coincidence with lower bound — not an Enola-optimality claim.
        let mut req = empty_req(4);
        req.layers = vec![
            layer(0, vec![entangle(0, 1)]),
            layer(1, vec![entangle(1, 2)]),
            layer(2, vec![entangle(2, 3)]),
        ];
        let result = asap_schedule_layers(req, &[]).expect("asap");
        assert_eq!(result.asap_makespan_cycles, 3);
        assert_eq!(result.critical_path.critical_path_length, 3);
        assert_eq!(result.request.layers.len(), 3);
        assert!(!result.compacted);
    }

    #[test]
    fn asap_exclusive_cycle_serializes_independent() {
        let mut req = empty_req(4);
        req.layers = vec![
            layer(0, vec![entangle(0, 1)]),
            layer(0, vec![entangle(2, 3)]),
        ];
        let result = asap_schedule_layers(req, &[]).expect("asap");
        assert_eq!(result.asap_makespan_cycles, 2);
        assert_eq!(result.critical_path.critical_path_length, 1);
        assert!(result.asap_makespan_cycles > result.critical_path.critical_path_length);
    }

    #[test]
    fn asap_does_not_union_actions() {
        let mut req = empty_req(4);
        req.layers = vec![
            layer(0, vec![entangle(0, 1)]),
            layer(0, vec![entangle(2, 3)]),
        ];
        let n = req.layers.len();
        let result = asap_schedule_layers(req, &[]).expect("asap");
        assert_eq!(result.request.layers.len(), n);
        for l in &result.request.layers {
            assert_eq!(l.actions.len(), 1);
        }
    }

    #[test]
    fn greedy_reduces_vs_asap_e0() {
        let mut req = empty_req(4);
        req.layers = vec![
            layer(0, vec![entangle(0, 1)]),
            layer(0, vec![entangle(2, 3)]),
        ];
        let opts = CompactionOptions {
            greedy: true,
            ..Default::default()
        };
        let result = compact_schedule(req, &[], &opts).expect("compact");
        assert_eq!(result.asap_makespan_cycles, 2);
        assert_eq!(result.compacted_makespan_cycles, 1);
        assert!(result.compacted_makespan_cycles < result.asap_makespan_cycles);
        assert!(result.compacted);
        assert_eq!(result.request.layers.len(), 1);
        assert_eq!(result.request.layers[0].actions.len(), 2);
    }

    #[test]
    fn measure_feed_forward_disjoint() {
        let mut req = empty_req(5);
        req.layers = vec![
            layer(0, vec![entangle(0, 1)]),
            layer(1, vec![measure(0)]),
            layer(2, vec![entangle(2, 3)]),
            layer(3, vec![entangle(2, 4)]),
        ];
        let ff = feed_forward_dependencies(1, &[3]);
        let opts = CompactionOptions {
            greedy: true,
            ..Default::default()
        };
        let result = compact_schedule(req, &ff, &opts).expect("compact");
        let c_meas = result
            .request
            .layers
            .iter()
            .find(|l| {
                l.actions
                    .iter()
                    .any(|a| matches!(a, NeutralAtomAction::Measure { atom, .. } if atom.0 == 0))
            })
            .map(|l| l.cycle)
            .expect("measure layer");
        let c_corr = result
            .request
            .layers
            .iter()
            .find(|l| {
                l.actions.iter().any(|a| {
                    matches!(
                        a,
                        NeutralAtomAction::Entangle2 { atoms, .. }
                            if atoms[0] == AtomId(2) && atoms[1] == AtomId(4)
                    )
                })
            })
            .map(|l| l.cycle)
            .expect("correction layer");
        assert!(c_meas < c_corr);
    }

    #[test]
    fn cannot_merge_measure_with_correction() {
        let mut req = empty_req(5);
        req.layers = vec![layer(0, vec![measure(0)]), layer(1, vec![entangle(2, 4)])];
        let ff = feed_forward_dependencies(0, &[1]);
        let opts = CompactionOptions::default();
        let err = force_merge_layers(req, &ff, 0, 1, &opts).unwrap_err();
        assert!(matches!(err, CompactionError::DependencyViolation));
    }

    #[test]
    fn barrier_blocks_cross_merge() {
        let mut req = empty_req(6);
        req.layers = vec![
            layer(0, vec![entangle(0, 1)]),
            layer(1, vec![entangle(4, 5)]),
            layer(2, vec![entangle(2, 3)]),
        ];
        let deps = vec![ScheduleDependency {
            before: 0,
            after: 2,
            kind: ScheduleDependencyKind::Barrier,
        }];
        let opts = CompactionOptions {
            greedy: true,
            ..Default::default()
        };
        let result = compact_schedule(req, &deps, &opts).expect("compact");
        let cycle_of_pair = |a: u32, b: u32| -> Option<u32> {
            result.request.layers.iter().find_map(|l| {
                l.actions.iter().find_map(|action| match action {
                    NeutralAtomAction::Entangle2 { atoms, .. }
                        if atoms[0] == AtomId(a) && atoms[1] == AtomId(b) =>
                    {
                        Some(l.cycle)
                    }
                    _ => None,
                })
            })
        };
        assert_ne!(cycle_of_pair(0, 1), cycle_of_pair(2, 3));
    }

    #[test]
    fn forbidden_merge_transfer_layers() {
        let mut req = empty_req(2);
        let xfer = |atom: u32, site: u32| {
            NeutralAtomAction::Transfer(TrapTransfer {
                atom: AtomId(atom),
                direction: TransferDirection::SlmToAod,
                site: SiteId(site),
                aod: PLACEHOLDER_AOD,
                duration_us: 15,
            })
        };
        req.layers = vec![layer(0, vec![xfer(0, 0)]), layer(1, vec![xfer(1, 1)])];
        let opts = CompactionOptions::default();
        let err = force_merge_layers(req, &[], 0, 1, &opts).unwrap_err();
        assert!(matches!(err, CompactionError::ForbiddenMergeClass(_)));
    }

    #[test]
    fn merge_rejected_when_r2_r3_violated() {
        // Two pairs too close for isolation when merged.
        let mut req = empty_req(4);
        req.layers = vec![
            layer(0, vec![entangle(0, 1)]),
            layer(0, vec![entangle(2, 3)]),
        ];
        req.layout = Some(NeutralAtomLayout {
            sites: vec![
                AtomSite {
                    id: SiteId(0),
                    position: Position {
                        x_um: 0.0,
                        y_um: 0.0,
                    },
                },
                AtomSite {
                    id: SiteId(1),
                    position: Position {
                        x_um: 5.0,
                        y_um: 0.0,
                    },
                },
                AtomSite {
                    id: SiteId(2),
                    position: Position {
                        x_um: 6.0,
                        y_um: 0.0,
                    },
                },
                AtomSite {
                    id: SiteId(3),
                    position: Position {
                        x_um: 11.0,
                        y_um: 0.0,
                    },
                },
            ],
            initial_bindings: (0..4)
                .map(|i| AtomBinding {
                    atom: AtomId(i),
                    trap: TrapBinding::Slm { site: SiteId(i) },
                })
                .collect(),
        });
        let opts = CompactionOptions {
            greedy: true,
            legality: Some(LegalityLimits {
                rydberg_range_um: 7.5,
                min_rydberg_spacing_um: 18.75,
                aod_min_separation_um: 2.0,
            }),
            ..Default::default()
        };
        let result = compact_schedule(req, &[], &opts).expect("compact");
        // Merge must be rejected → still 2 cycles.
        assert_eq!(result.compacted_makespan_cycles, 2);
        assert!(!result.compacted);
    }

    #[test]
    fn r2_r3_runs_without_mlir_feature() {
        // Same as above — this test file compiles under --no-default-features.
        let mut req = empty_req(4);
        req.layers = vec![
            layer(0, vec![entangle(0, 1)]),
            layer(0, vec![entangle(2, 3)]),
        ];
        // Far-apart pairs: merge should succeed under legality.
        req.layout = Some(NeutralAtomLayout {
            sites: vec![
                AtomSite {
                    id: SiteId(0),
                    position: Position {
                        x_um: 0.0,
                        y_um: 0.0,
                    },
                },
                AtomSite {
                    id: SiteId(1),
                    position: Position {
                        x_um: 5.0,
                        y_um: 0.0,
                    },
                },
                AtomSite {
                    id: SiteId(2),
                    position: Position {
                        x_um: 40.0,
                        y_um: 0.0,
                    },
                },
                AtomSite {
                    id: SiteId(3),
                    position: Position {
                        x_um: 45.0,
                        y_um: 0.0,
                    },
                },
            ],
            initial_bindings: (0..4)
                .map(|i| AtomBinding {
                    atom: AtomId(i),
                    trap: TrapBinding::Slm { site: SiteId(i) },
                })
                .collect(),
        });
        let opts = CompactionOptions {
            greedy: true,
            legality: Some(LegalityLimits {
                rydberg_range_um: 7.5,
                min_rydberg_spacing_um: 18.75,
                aod_min_separation_um: 2.0,
            }),
            ..Default::default()
        };
        let result = compact_schedule(req, &[], &opts).expect("compact");
        assert_eq!(result.compacted_makespan_cycles, 1);
    }

    #[test]
    fn critical_path_report_populated() {
        let mut req = empty_req(4);
        req.layers = vec![
            layer(0, vec![entangle(0, 1)]),
            layer(1, vec![entangle(1, 2)]),
            layer(2, vec![entangle(2, 3)]),
        ];
        let result = asap_schedule_layers(req, &[]).expect("asap");
        assert_eq!(result.critical_path.makespan_cycles, 3);
        assert_eq!(result.critical_path.critical_path_length, 3);
        assert_eq!(result.critical_path.critical_layer_indices, vec![0, 1, 2]);
        let json = serde_json::to_string(&result.critical_path).expect("ser");
        let back: CriticalPathReport = serde_json::from_str(&json).expect("de");
        assert_eq!(back, result.critical_path);
    }

    #[test]
    fn zoned_entangle_only_passthrough() {
        let mut req = empty_req(4);
        req.layers = vec![
            layer(0, vec![entangle(0, 1)]),
            layer(0, vec![entangle(2, 3)]),
        ];
        let arch = toy_zoned_architecture();
        // Place atoms inside the entanglement zone (origin y=50) with pair gap
        // and wide isolation so E0 merge + zone validate succeed.
        let ent = arch
            .zones
            .iter()
            .find(|z| z.kind == ZoneKind::Entanglement)
            .expect("ent zone");
        let y = ent.origin_um.1;
        let sites = vec![
            AtomSite {
                id: SiteId(0),
                position: Position { x_um: 0.0, y_um: y },
            },
            AtomSite {
                id: SiteId(1),
                position: Position { x_um: 2.0, y_um: y },
            },
            AtomSite {
                id: SiteId(2),
                position: Position {
                    x_um: 40.0,
                    y_um: y,
                },
            },
            AtomSite {
                id: SiteId(3),
                position: Position {
                    x_um: 42.0,
                    y_um: y,
                },
            },
        ];
        req.layout = Some(NeutralAtomLayout {
            sites,
            initial_bindings: (0..4)
                .map(|i| AtomBinding {
                    atom: AtomId(i),
                    trap: TrapBinding::Slm { site: SiteId(i) },
                })
                .collect(),
        });
        let opts = CompactionOptions {
            greedy: true,
            arch: Some(arch),
            legality: Some(LegalityLimits {
                rydberg_range_um: 7.5,
                min_rydberg_spacing_um: 18.75,
                aod_min_separation_um: 2.0,
            }),
        };
        let result = compact_schedule(req, &[], &opts).expect("compact");
        assert_eq!(result.compacted_makespan_cycles, 1);
    }

    #[test]
    fn zone_reject_move_merge_without_simulator() {
        let mut req = empty_req(4);
        req.layers = vec![
            layer(
                0,
                vec![NeutralAtomAction::Move(MovementGroup {
                    moves: vec![AtomMove {
                        atom: AtomId(0),
                        from: SiteId(0),
                        to: SiteId(1),
                    }],
                    duration_us: 10,
                })],
            ),
            layer(1, vec![entangle(2, 3)]),
        ];
        let arch = toy_zoned_architecture();
        req.layout = Some(NeutralAtomLayout {
            sites: vec![
                AtomSite {
                    id: SiteId(0),
                    position: Position {
                        x_um: 0.0,
                        y_um: 0.0,
                    },
                },
                AtomSite {
                    id: SiteId(1),
                    position: Position {
                        x_um: 10.0,
                        y_um: 0.0,
                    },
                },
                AtomSite {
                    id: SiteId(2),
                    position: Position {
                        x_um: 40.0,
                        y_um: 0.0,
                    },
                },
                AtomSite {
                    id: SiteId(3),
                    position: Position {
                        x_um: 45.0,
                        y_um: 0.0,
                    },
                },
            ],
            initial_bindings: (0..4)
                .map(|i| AtomBinding {
                    atom: AtomId(i),
                    trap: TrapBinding::Slm { site: SiteId(i) },
                })
                .collect(),
        });
        let opts = CompactionOptions {
            arch: Some(arch),
            ..Default::default()
        };
        let err = force_merge_layers(req, &[], 0, 1, &opts).unwrap_err();
        assert!(matches!(err, CompactionError::ForbiddenMergeClass(_)));
    }
}
