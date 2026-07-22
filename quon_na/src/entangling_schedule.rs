//! Entangling-layer scheduler via Misra–Gries edge coloring (#105).
//!
//! For [`SegmentKind::CommutationGroup`] segments whose interactions are all
//! 2-qubit, this module edge-colors the qubit interaction graph with the
//! **Misra–Gries** constructive Vizing algorithm (≤ Δ+1 colors). That is the
//! algorithm behind Enola Theorem 1 (≤ S_opt + 1 Rydberg stages); do **not**
//! cite that bound for plain greedy (≤ 2Δ−1) coloring or for
//! [`SegmentKind::DependencyDag`] segments.
//!
//! Dependency-DAG segments use ASAP layering on precomputed `dag_layer`
//! indices (critical-path optimal for ordered gates; Enola Sec. 3 ordered
//! case) — never edge-coloring.
//!
//! After coloring / ASAP bucketing, oversized parallel sets are split so each
//! emitted [`ScheduleLayer`] has at most `max_parallel_entangling_pairs`
//! entangling actions. Scheduling is **graph-only**: `layout` is ignored and
//! left unchanged; atom identity is `AtomId(vertex.index())` (same as
//! placement #104).
//!
//! References: [Enola] Sec. 3 / Theorem 1; Misra & Gries, "A constructive proof
//! of Vizing's theorem", IPL 1992.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::graph::{GraphError, Interaction, InteractionId, LogicalQubitId, SegmentKind, VertexId};
use crate::layout::AtomId;
use crate::schedule::{NeutralAtomAction, ScheduleError, ScheduleLayer};
use crate::schedule_entry::GraphScheduleRequest;

#[cfg(feature = "flux")]
use flux_rs::attrs::*;

/// Per-layer parallel-capacity utilization.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerUtilization {
    pub cycle: u32,
    pub entangling_pairs: u32,
    pub capacity: u32,
    /// `entangling_pairs / capacity` when `capacity > 0`; else `0.0`.
    pub utilization: f64,
}

/// Result of [`schedule_entangling_layers`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EntanglingScheduleResult<V = LogicalQubitId> {
    pub request: GraphScheduleRequest<V>,
    pub utilizations: Vec<LayerUtilization>,
    /// Which `ScheduleLayer.cycle` each interaction landed in (issue #298):
    /// lets a post-pass splice 1-qubit gate actions into the right layer
    /// relative to the ≥2-qubit interactions on the same qubit — see
    /// `pipeline::interleave_local_gates`. `#[serde(default)]` so older
    /// serialized results without this field still deserialize.
    #[serde(default)]
    pub interaction_cycle: BTreeMap<InteractionId, u32>,
    /// Maximum degree Δ among 2Q commutation subgraphs that were Misra–Gries colored.
    pub max_degree: u32,
    /// True iff every non-empty commutation segment was 2Q-only and used Misra–Gries.
    pub misra_gries_applied: bool,
}

/// Errors from [`schedule_entangling_layers`].
#[derive(Debug, Error, Clone, PartialEq)]
pub enum EntanglingScheduleError<V = LogicalQubitId> {
    #[error(transparent)]
    InvalidGraph(#[from] GraphError<V>),
    #[error("max_parallel_entangling_pairs must be ≥ 1, got {0}")]
    InvalidCapacity(u32),
    #[error("schedule layer conflict: {0}")]
    Conflict(String),
    #[error(
        "commutation segment contains multi-qubit gate {0:?}; Misra–Gries requires 2-qubit edges"
    )]
    MultiQubitCommutation(InteractionId),
    #[error(
        "duplicate interaction on vertex pair {0:?}–{1:?} in a commutation group; Misra–Gries needs a simple graph"
    )]
    DuplicatePair(V, V),
    #[error("unknown interaction {0:?} referenced by a segment")]
    UnknownInteraction(InteractionId),
}

/// Ceiling of `n / cap` layers needed to host `n` parallel pairs under capacity `cap`.
///
/// Returns `0` when `n == 0`.
#[cfg_attr(
    feature = "flux",
    spec(fn(n: u32, cap: u32{v: v > 0}) -> u32{v: (n == 0 && v == 0) || (n > 0 && v >= 1)})
)]
pub fn capacity_layer_count(n: u32, cap: u32) -> u32 {
    if n == 0 {
        return 0;
    }
    debug_assert!(cap > 0);
    n.div_ceil(cap)
}

/// Fill `req.layers` with entangling actions from the interaction graph.
///
/// Always overwrites `layers`. Leaves `layout` unchanged (graph-only; may be
/// `None`). Atom identity: `AtomId(vertex.index())`.
pub fn schedule_entangling_layers<V: VertexId>(
    mut req: GraphScheduleRequest<V>,
    max_parallel_entangling_pairs: u32,
) -> Result<EntanglingScheduleResult<V>, EntanglingScheduleError<V>> {
    if max_parallel_entangling_pairs == 0 {
        return Err(EntanglingScheduleError::InvalidCapacity(0));
    }
    req.graph.validate()?;

    let by_id: BTreeMap<InteractionId, &Interaction<V>> =
        req.graph.interactions.iter().map(|i| (i.id, i)).collect();

    let mut layers = Vec::new();
    let mut interaction_cycle: BTreeMap<InteractionId, u32> = BTreeMap::new();
    let mut next_cycle: u32 = 0;
    let mut max_degree: u32 = 0;
    let mut misra_gries_applied = true;
    let mut saw_commutation = false;

    for segment in &req.graph.segments {
        let interactions = resolve_interactions(&segment.interactions, &by_id)?;
        if interactions.is_empty() {
            continue;
        }

        let buckets: Vec<Vec<&Interaction<V>>> = match segment.kind {
            SegmentKind::CommutationGroup => {
                saw_commutation = true;
                let (colored, delta) = color_commutation_group(&interactions)?;
                max_degree = max_degree.max(delta);
                colored
            }
            SegmentKind::DependencyDag => asap_buckets(&interactions),
        };

        for bucket in buckets {
            for chunk in bucket.chunks(max_parallel_entangling_pairs as usize) {
                let actions = chunk
                    .iter()
                    .map(|interaction| entangle_action(interaction))
                    .collect::<Vec<_>>();
                let layer = ScheduleLayer {
                    cycle: next_cycle,
                    actions,
                };
                layer
                    .validate_conflicts()
                    .map_err(|e| EntanglingScheduleError::Conflict(format_conflict(e)))?;
                for interaction in chunk {
                    interaction_cycle.insert(interaction.id, next_cycle);
                }
                layers.push(layer);
                next_cycle = next_cycle.saturating_add(1);
            }
        }
    }

    if !saw_commutation {
        misra_gries_applied = false;
    }

    let utilizations = layers
        .iter()
        .map(|layer| {
            let entangling_pairs = count_entangling_actions(&layer.actions);
            LayerUtilization {
                cycle: layer.cycle,
                entangling_pairs,
                capacity: max_parallel_entangling_pairs,
                utilization: if max_parallel_entangling_pairs == 0 {
                    0.0
                } else {
                    f64::from(entangling_pairs) / f64::from(max_parallel_entangling_pairs)
                },
            }
        })
        .collect();

    req.layers = layers;
    Ok(EntanglingScheduleResult {
        request: req,
        utilizations,
        interaction_cycle,
        max_degree,
        misra_gries_applied,
    })
}

fn format_conflict(err: ScheduleError) -> String {
    err.to_string()
}

fn resolve_interactions<'a, V>(
    ids: &[InteractionId],
    by_id: &BTreeMap<InteractionId, &'a Interaction<V>>,
) -> Result<Vec<&'a Interaction<V>>, EntanglingScheduleError<V>> {
    let mut out = Vec::with_capacity(ids.len());
    for &id in ids {
        let interaction = by_id
            .get(&id)
            .copied()
            .ok_or(EntanglingScheduleError::UnknownInteraction(id))?;
        out.push(interaction);
    }
    Ok(out)
}

fn entangle_action<V: VertexId>(interaction: &Interaction<V>) -> NeutralAtomAction {
    let atoms: Vec<AtomId> = interaction
        .qubits
        .iter()
        .map(|q| AtomId(q.index()))
        .collect();
    match atoms.as_slice() {
        [a, b] => NeutralAtomAction::Entangle2 {
            atoms: [*a, *b],
            duration_us: 1,
        },
        _ => NeutralAtomAction::EntangleN {
            atoms,
            duration_us: 1,
        },
    }
}

fn count_entangling_actions(actions: &[NeutralAtomAction]) -> u32 {
    actions
        .iter()
        .filter(|a| {
            matches!(
                a,
                NeutralAtomAction::Entangle2 { .. } | NeutralAtomAction::EntangleN { .. }
            )
        })
        .count() as u32
}

/// ASAP buckets: group by `dag_layer`, stable by `InteractionId` within a layer.
fn asap_buckets<'a, V>(interactions: &[&'a Interaction<V>]) -> Vec<Vec<&'a Interaction<V>>> {
    let mut by_layer: BTreeMap<u32, Vec<&Interaction<V>>> = BTreeMap::new();
    for &interaction in interactions {
        by_layer
            .entry(interaction.dag_layer)
            .or_default()
            .push(interaction);
    }
    by_layer
        .into_values()
        .map(|mut group| {
            group.sort_by_key(|i| i.id);
            group
        })
        .collect()
}

/// Misra–Gries color classes for one commutation group: per-color buckets
/// of borrowed interactions, plus the maximum degree Δ.
type ColorClasses<'a, V> = (Vec<Vec<&'a Interaction<V>>>, u32);

/// Misra–Gries color a 2Q commutation group; return color classes (sorted by color)
/// and Δ.
fn color_commutation_group<'a, V: VertexId>(
    interactions: &[&'a Interaction<V>],
) -> Result<ColorClasses<'a, V>, EntanglingScheduleError<V>> {
    let mut pair_to_interaction: BTreeMap<(V, V), &'a Interaction<V>> = BTreeMap::new();
    let mut vertices: BTreeSet<V> = BTreeSet::new();

    for &interaction in interactions {
        if interaction.qubits.len() != 2 {
            return Err(EntanglingScheduleError::MultiQubitCommutation(
                interaction.id,
            ));
        }
        let a = interaction.qubits[0];
        let b = interaction.qubits[1];
        vertices.insert(a);
        vertices.insert(b);
        let key = if a < b { (a, b) } else { (b, a) };
        if pair_to_interaction.insert(key, interaction).is_some() {
            return Err(EntanglingScheduleError::DuplicatePair(key.0, key.1));
        }
    }

    let vertex_index: BTreeMap<V, usize> = vertices
        .iter()
        .copied()
        .enumerate()
        .map(|(i, q)| (q, i))
        .collect();
    let n = vertex_index.len();
    let edges: Vec<(usize, usize, InteractionId)> = pair_to_interaction
        .iter()
        .map(|(&(a, b), interaction)| (vertex_index[&a], vertex_index[&b], interaction.id))
        .collect();

    let (colors, delta) = misra_gries_color(n, &edges);
    let n_colors = colors.iter().copied().max().map(|c| c + 1).unwrap_or(0);
    debug_assert!(delta == 0 || n_colors <= delta + 1);

    let id_to_interaction: BTreeMap<InteractionId, &Interaction<V>> =
        pair_to_interaction.values().map(|i| (i.id, *i)).collect();

    let mut classes: BTreeMap<u32, Vec<&Interaction<V>>> = BTreeMap::new();
    for (edge_idx, &color) in colors.iter().enumerate() {
        let id = edges[edge_idx].2;
        classes
            .entry(color)
            .or_default()
            .push(id_to_interaction[&id]);
    }
    let buckets = classes
        .into_values()
        .map(|mut group| {
            group.sort_by_key(|i| i.id);
            group
        })
        .collect();
    Ok((buckets, delta))
}

/// Misra–Gries edge coloring: returns one color per edge (same order as `edges`)
/// and the maximum degree Δ.
///
/// Uses at most Δ+1 colors on a simple graph (Vizing's theorem, constructive).
/// Case split follows Misra & Gries (IPL 1992) / Enola Sec. 3:
/// build a maximal fan, then either invert the c–d path and color with `d`, or
/// leave the path and rotate the fan prefix before coloring with `d`.
fn misra_gries_color(n: usize, edges: &[(usize, usize, InteractionId)]) -> (Vec<u32>, u32) {
    if edges.is_empty() {
        return (Vec::new(), 0);
    }

    let mut adj: Vec<BTreeMap<usize, usize>> = vec![BTreeMap::new(); n];
    let mut degree = vec![0u32; n];
    for (idx, &(u, v, _)) in edges.iter().enumerate() {
        adj[u].insert(v, idx);
        adj[v].insert(u, idx);
        degree[u] += 1;
        degree[v] += 1;
    }
    let delta = degree.into_iter().max().unwrap_or(0);
    let n_colors = (delta + 1) as usize;

    let mut edge_color: Vec<Option<u32>> = vec![None; edges.len()];
    let mut color_to_edge: Vec<Vec<Option<usize>>> = vec![vec![None; n_colors]; n];

    let mut order: Vec<usize> = (0..edges.len()).collect();
    order.sort_by_key(|&i| {
        let (u, v, id) = edges[i];
        let (a, b) = if u < v { (u, v) } else { (v, u) };
        (a, b, id)
    });

    for &edge_idx in &order {
        let (u, v, _) = edges[edge_idx];
        color_edge(u, v, edge_idx, &adj, &mut edge_color, &mut color_to_edge);
        debug_assert!(
            edge_color[edge_idx].is_some(),
            "Misra–Gries left edge {edge_idx} uncolored"
        );
    }

    validate_coloring(n, edges, &edge_color, delta);

    let colors: Vec<u32> = edge_color
        .iter()
        .map(|c| match c {
            Some(color) => *color,
            None => unreachable!("every edge colored"),
        })
        .collect();
    (colors, delta)
}

fn validate_coloring(
    n: usize,
    edges: &[(usize, usize, InteractionId)],
    edge_color: &[Option<u32>],
    delta: u32,
) {
    let mut seen: Vec<BTreeSet<u32>> = vec![BTreeSet::new(); n];
    for (idx, &(u, v, _)) in edges.iter().enumerate() {
        let c = edge_color[idx].expect("colored");
        assert!(c <= delta, "color {c} exceeds Δ+1 palette (Δ={delta})");
        assert!(seen[u].insert(c), "vertex {u} reuses color {c}");
        assert!(seen[v].insert(c), "vertex {v} reuses color {c}");
    }
}

fn free_color(color_to_edge_v: &[Option<usize>]) -> u32 {
    for (c, slot) in color_to_edge_v.iter().enumerate() {
        if slot.is_none() {
            return c as u32;
        }
    }
    unreachable!("no free color; Δ+1 palette exhausted")
}

fn color_edge(
    u: usize,
    v: usize,
    edge_uv: usize,
    adj: &[BTreeMap<usize, usize>],
    edge_color: &mut [Option<u32>],
    color_to_edge: &mut [Vec<Option<usize>>],
) {
    // Maximal fan at u starting at v (Misra & Gries / Wikipedia).
    let mut fan: Vec<usize> = vec![v];
    let mut in_fan: BTreeSet<usize> = BTreeSet::from([v]);
    loop {
        let tip = *fan.last().expect("fan non-empty");
        let free_at_tip = free_color(&color_to_edge[tip]);
        let mut next = None;
        for (&w, &eidx) in &adj[u] {
            if in_fan.contains(&w) {
                continue;
            }
            if edge_color[eidx] == Some(free_at_tip) {
                next = Some(w);
                break;
            }
        }
        match next {
            Some(w) => {
                fan.push(w);
                in_fan.insert(w);
            }
            None => break,
        }
    }

    let c = free_color(&color_to_edge[u]);
    let tip = *fan.last().expect("fan non-empty");
    let d = free_color(&color_to_edge[tip]);

    // Always invert the cd-path at u. Because c is free at u, any non-empty
    // path's first edge at u is colored d (Misra & Gries 1992).
    invert_cd_path(u, c, d, adj, edge_color, color_to_edge);

    // After inversion, some fan prefix F[1..=w] remains a fan with d free on w.
    let rotate_to = match fan
        .iter()
        .position(|&x| color_to_edge[x][d as usize].is_none())
    {
        Some(i) => i,
        None => unreachable!("inversion guarantees a fan vertex with d free"),
    };
    rotate_fan_and_color(
        u,
        edge_uv,
        &fan,
        rotate_to,
        d,
        adj,
        edge_color,
        color_to_edge,
    );
}

/// Shift colors along fan edges `u–fan[0] … u–fan[rotate_to]`, then color the
/// last with `d`. `fan[0]` is the uncolored endpoint `v`.
#[allow(clippy::too_many_arguments, clippy::needless_range_loop)]
fn rotate_fan_and_color(
    u: usize,
    edge_uv: usize,
    fan: &[usize],
    rotate_to: usize,
    d: u32,
    adj: &[BTreeMap<usize, usize>],
    edge_color: &mut [Option<u32>],
    color_to_edge: &mut [Vec<Option<usize>>],
) {
    let edge_at = |i: usize| -> usize { if i == 0 { edge_uv } else { adj[u][&fan[i]] } };

    // Snapshot colors on fan edges 1..=rotate_to (edge 0 is uncolored).
    let mut old: Vec<Option<u32>> = Vec::with_capacity(rotate_to + 1);
    old.push(None);
    for i in 1..=rotate_to {
        old.push(edge_color[edge_at(i)]);
    }

    // Clear existing colors on the prefix (skip uncolored edge 0).
    for i in 1..=rotate_to {
        let x = fan[i];
        let e = edge_at(i);
        set_edge_color(u, x, e, None, edge_color, color_to_edge);
    }

    // Shift: edge i gets former color of edge i+1; last gets d.
    for i in 0..rotate_to {
        let x = fan[i];
        let e = edge_at(i);
        let new_color = match old[i + 1] {
            Some(c) => c,
            None => unreachable!("fan edge beyond v is colored"),
        };
        set_edge_color(u, x, e, Some(new_color), edge_color, color_to_edge);
    }
    let x_last = fan[rotate_to];
    let e_last = edge_at(rotate_to);
    set_edge_color(u, x_last, e_last, Some(d), edge_color, color_to_edge);
}

fn set_edge_color(
    u: usize,
    v: usize,
    edge_idx: usize,
    color: Option<u32>,
    edge_color: &mut [Option<u32>],
    color_to_edge: &mut [Vec<Option<usize>>],
) {
    if let Some(old) = edge_color[edge_idx] {
        if color_to_edge[u][old as usize] == Some(v) {
            color_to_edge[u][old as usize] = None;
        }
        if color_to_edge[v][old as usize] == Some(u) {
            color_to_edge[v][old as usize] = None;
        }
    }
    edge_color[edge_idx] = color;
    if let Some(c) = color {
        debug_assert!(
            color_to_edge[u][c as usize].is_none(),
            "u already has color {c}"
        );
        debug_assert!(
            color_to_edge[v][c as usize].is_none(),
            "v already has color {c}"
        );
        color_to_edge[u][c as usize] = Some(v);
        color_to_edge[v][c as usize] = Some(u);
    }
}

/// Invert the maximal cd-path at `start`.
///
/// Because `c` is free at `start`, a non-empty path's first edge is colored
/// `d`, then `c`, then `d`, … (Misra & Gries 1992). Inversion frees `d` at
/// `start` (or leaves it free if the path was empty).
fn invert_cd_path(
    start: usize,
    c: u32,
    d: u32,
    adj: &[BTreeMap<usize, usize>],
    edge_color: &mut [Option<u32>],
    color_to_edge: &mut [Vec<Option<usize>>],
) {
    let mut path_edges: Vec<(usize, usize, usize)> = Vec::new(); // (u, v, eidx)
    let mut current = start;
    // First edge at start (if any) has color d.
    let mut expect = d;
    let mut other = c;
    while let Some(next) = color_to_edge[current][expect as usize] {
        let eidx = adj[current][&next];
        path_edges.push((current, next, eidx));
        current = next;
        std::mem::swap(&mut expect, &mut other);
    }

    // Clear first so mid-path swaps cannot collide in color_to_edge.
    for &(a, b, eidx) in &path_edges {
        set_edge_color(a, b, eidx, None, edge_color, color_to_edge);
    }
    // Edges were d, c, d, c, ... → invert to c, d, c, d, ...
    for (i, &(a, b, eidx)) in path_edges.iter().enumerate() {
        let new = if i % 2 == 0 { c } else { d };
        set_edge_color(a, b, eidx, Some(new), edge_color, color_to_edge);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::{
        Interaction, InteractionGraph, InteractionId, InteractionSegment, LogicalQubitId,
        SegmentKind, cubic_commutation_graph, schedule_dependency_segment,
    };
    use crate::schedule_entry::schedule_from_graph;

    fn pair(id: u32, a: u32, b: u32) -> Interaction {
        let mut qubits = vec![LogicalQubitId(a), LogicalQubitId(b)];
        qubits.sort();
        Interaction {
            id: InteractionId(id),
            qubits,
            gate_name: "CZ".into(),
            dag_layer: 0,
            on_critical_path: false,
        }
    }

    #[test]
    fn capacity_layer_count_ceil_divides() {
        assert_eq!(capacity_layer_count(0, 5), 0);
        assert_eq!(capacity_layer_count(1, 5), 1);
        assert_eq!(capacity_layer_count(5, 5), 1);
        assert_eq!(capacity_layer_count(6, 5), 2);
        assert_eq!(capacity_layer_count(10, 3), 4);
    }

    #[test]
    fn path_of_three_edges_at_most_delta_plus_one() {
        // Path on 4 vertices: 3 edges, Δ=2 → ≤ 3 colors.
        let vertices: Vec<_> = (0..4).map(LogicalQubitId).collect();
        let interactions = vec![pair(0, 0, 1), pair(1, 1, 2), pair(2, 2, 3)];
        let ids: Vec<_> = interactions.iter().map(|i| i.id).collect();
        let graph = InteractionGraph::from_interactions(
            vertices,
            interactions,
            vec![InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: ids,
            }],
            crate::graph::DEFAULT_GAMMA,
        )
        .expect("graph");
        let req = schedule_from_graph(graph).expect("stub");
        let result = schedule_entangling_layers(req, 340).expect("schedule");
        assert!(result.misra_gries_applied);
        assert!(result.max_degree <= 2);
        assert!(result.request.layers.len() <= (result.max_degree as usize) + 1);
        for layer in &result.request.layers {
            layer.validate_conflicts().expect("conflicts");
        }
    }

    #[test]
    fn cubic_n12_at_most_four_layers_pre_capacity() {
        // Chromatic bound (pre-capacity): Δ=3 ⇒ Misra–Gries ≤ 4 layers.
        // capacity=340 so capacity splitting does not inflate the count.
        let graph = cubic_commutation_graph(12).expect("cubic");
        let req = schedule_from_graph(graph).expect("stub");
        assert!(req.layout.is_none());
        let result = schedule_entangling_layers(req, 340).expect("schedule");
        assert_eq!(result.max_degree, 3);
        assert!(result.misra_gries_applied);
        assert!(
            result.request.layers.len() <= 4,
            "got {} layers",
            result.request.layers.len()
        );
        assert!(result.request.layout.is_none());
        for layer in &result.request.layers {
            layer.validate_conflicts().expect("conflicts");
        }
    }

    #[test]
    fn cubic_n8_validate_conflicts() {
        let graph = cubic_commutation_graph(8).expect("cubic");
        let req = schedule_from_graph(graph).expect("stub");
        let result = schedule_entangling_layers(req, 340).expect("schedule");
        for layer in &result.request.layers {
            layer.validate_conflicts().expect("ok");
        }
    }

    #[test]
    fn capacity_one_splits_matching() {
        // Matching of 3 disjoint edges: Δ=1 → 1 color, capacity=1 → 3 layers.
        let vertices: Vec<_> = (0..6).map(LogicalQubitId).collect();
        let interactions = vec![pair(0, 0, 1), pair(1, 2, 3), pair(2, 4, 5)];
        let ids: Vec<_> = interactions.iter().map(|i| i.id).collect();
        let graph = InteractionGraph::from_interactions(
            vertices,
            interactions,
            vec![InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: ids,
            }],
            crate::graph::DEFAULT_GAMMA,
        )
        .expect("graph");
        let req = schedule_from_graph(graph).expect("stub");
        let result = schedule_entangling_layers(req, 1).expect("schedule");
        assert_eq!(result.request.layers.len(), 3);
        assert_eq!(capacity_layer_count(3, 1), 3);
        for u in &result.utilizations {
            assert_eq!(u.entangling_pairs, 1);
            assert_eq!(u.capacity, 1);
            assert!((u.utilization - 1.0).abs() < 1e-9);
        }
    }

    #[test]
    fn dependency_dag_asap_not_edge_color() {
        // Chain of 3 dependent CZs on overlapping qubits → 3 ASAP layers.
        let mut interactions = vec![pair(0, 0, 1), pair(1, 1, 2), pair(2, 2, 3)];
        schedule_dependency_segment(&mut interactions);
        let ids: Vec<_> = interactions.iter().map(|i| i.id).collect();
        let vertices: Vec<_> = (0..4).map(LogicalQubitId).collect();
        let graph = InteractionGraph::from_interactions(
            vertices,
            interactions,
            vec![InteractionSegment {
                kind: SegmentKind::DependencyDag,
                interactions: ids,
            }],
            crate::graph::DEFAULT_GAMMA,
        )
        .expect("graph");
        let req = schedule_from_graph(graph).expect("stub");
        let result = schedule_entangling_layers(req, 340).expect("schedule");
        assert!(!result.misra_gries_applied);
        assert_eq!(result.request.layers.len(), 3);
    }

    #[test]
    fn graph_only_preserves_absent_layout() {
        let graph = cubic_commutation_graph(6).expect("cubic");
        let req = schedule_from_graph(graph).expect("stub");
        let result = schedule_entangling_layers(req, 340).expect("schedule");
        assert!(result.request.layout.is_none());
        assert!(!result.request.layers.is_empty());
    }

    #[test]
    fn rejects_zero_capacity() {
        let graph = cubic_commutation_graph(4).expect("cubic");
        let req = schedule_from_graph(graph).expect("stub");
        assert_eq!(
            schedule_entangling_layers(req, 0),
            Err(EntanglingScheduleError::InvalidCapacity(0))
        );
    }

    #[test]
    fn rejects_multi_qubit_in_commutation_group() {
        let vertices: Vec<_> = (0..3).map(LogicalQubitId).collect();
        let interactions = vec![Interaction {
            id: InteractionId(0),
            qubits: vec![LogicalQubitId(0), LogicalQubitId(1), LogicalQubitId(2)],
            gate_name: "CCZ".into(),
            dag_layer: 0,
            on_critical_path: false,
        }];
        let graph = InteractionGraph::from_interactions(
            vertices,
            interactions,
            vec![InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: vec![InteractionId(0)],
            }],
            crate::graph::DEFAULT_GAMMA,
        )
        .expect("graph");
        let req = schedule_from_graph(graph).expect("stub");
        assert!(matches!(
            schedule_entangling_layers(req, 10),
            Err(EntanglingScheduleError::MultiQubitCommutation(_))
        ));
    }

    #[test]
    fn misra_gries_respects_delta_plus_one_on_complete_graph() {
        // K_4: 6 edges, Δ=3 → ≤ 4 colors.
        let vertices: Vec<_> = (0..4).map(LogicalQubitId).collect();
        let mut interactions = Vec::new();
        let mut id = 0u32;
        for a in 0..4 {
            for b in (a + 1)..4 {
                interactions.push(pair(id, a, b));
                id += 1;
            }
        }
        let ids: Vec<_> = interactions.iter().map(|i| i.id).collect();
        let graph = InteractionGraph::from_interactions(
            vertices,
            interactions,
            vec![InteractionSegment {
                kind: SegmentKind::CommutationGroup,
                interactions: ids,
            }],
            crate::graph::DEFAULT_GAMMA,
        )
        .expect("graph");
        let req = schedule_from_graph(graph).expect("stub");
        let result = schedule_entangling_layers(req, 340).expect("schedule");
        assert_eq!(result.max_degree, 3);
        assert!(result.request.layers.len() <= 4);
    }
}
