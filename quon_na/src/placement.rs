//! Initial placement of logical qubits onto SLM sites (issue #104).
//!
//! Three heuristics map vertex ids (`LogicalQubitId` on the bare path, or
//! `AtomVertexId` on the hybrid QEC path, #318) from an
//! [`crate::graph::InteractionGraph`] onto a compact rectangular grid of
//! [`crate::layout::AtomSite`]s and fill
//! [`crate::schedule_entry::GraphScheduleRequest::layout`].
//!
//! Strategies are **inspired by** [Atomique] Sec. III-B (load-balance / spiral
//! fill; MAX-k-Cut array mapper Alg. 1), adapted to a single flat grid — not
//! reproductions. Enola's simulated-annealing placer is out of scope.
//!
//! Placement score is movement-cost-shaped:
//! `Σ_e weight(e) · √(euclidean_µm(site(a), site(b)))`, following the √-law
//! movement model in `docs/neutral_atom/architecture_model.md` §5.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::graph::{GraphError, InteractionGraph, LogicalQubitId, VertexId};
use crate::layout::{
    AtomBinding, AtomId, AtomSite, NeutralAtomLayout, Position, SiteId, TrapBinding,
};

/// SLM lattice pitch in micrometres (relative scale; cancels in strategy comparisons).
pub const SITE_PITCH_UM: f64 = 5.0;

/// Placement heuristic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlacementStrategy {
    /// Ascending qubit id → row-major sites.
    RowMajor,
    /// Weighted-degree descending → spiral from grid center (Atomique load-balance–inspired).
    DegreeBased,
    /// Agglomerative clustering on gate-frequency weights, then packed row bands
    /// (Atomique MAX-k-Cut–inspired, adapted to spatial proximity).
    InteractionClustering,
    /// SMT-optimal placement via z3 (issue #302, Deliverable B). Requires the
    /// `solver` feature; the exact encoding lives in [`crate::exact::placement`].
    /// Falls back to [`PlacementStrategy::InteractionClustering`] with a logged
    /// optimality gap when the `solver` feature is off or z3 times out.
    Exact,
}

/// Result of [`place`]: filled request plus score metadata.
///
/// Generic over the vertex label `V` (default [`LogicalQubitId`]); see
/// [`InteractionGraph`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacementResult<V = LogicalQubitId> {
    pub request: crate::schedule_entry::GraphScheduleRequest<V>,
    /// Lower is better: `Σ w · √d` over interaction edges.
    pub score: f64,
    pub strategy: PlacementStrategy,
    /// Interacting pairs whose sites share a row or column (AOD parallelism proxy).
    pub axis_aligned_pairs: u32,
}

/// Errors from placement.
///
/// Generic over the vertex label `V` (default [`LogicalQubitId`]); the graph
/// sub-error and the missing-binding variant carry `V`.
#[derive(Debug, Error, Clone, PartialEq)]
pub enum PlacementError<V = LogicalQubitId> {
    #[error(transparent)]
    InvalidGraph(#[from] GraphError<V>),
    #[error("empty interaction graph: no vertices to place")]
    EmptyGraph,
    #[error("placement produced overlapping or incomplete site bindings")]
    InvalidBindings,
    #[error("layout is missing a binding for vertex {0:?}")]
    MissingBinding(V),
    /// Exact SMT placement failed (z3 returned infeasible or no model).
    /// Requires the `solver` feature; the caller should fall back to a
    /// heuristic strategy.
    #[error("exact placement solver failed: {0}")]
    ExactSolverFailed(String),
}

/// Place qubits onto a square-ish SLM grid and fill `req.layout`.
///
/// Leaves `req.layers` unchanged. Overwrites any existing layout.
pub fn place<V: VertexId>(
    mut req: crate::schedule_entry::GraphScheduleRequest<V>,
    strategy: PlacementStrategy,
) -> Result<PlacementResult<V>, PlacementError<V>> {
    req.graph.validate()?;
    if req.graph.vertices.is_empty() {
        return Err(PlacementError::EmptyGraph);
    }

    // Exact placement dispatches to the z3 solver (issue #302). When the
    // `solver` feature is off, fall back to InteractionClustering so the
    // build never fails — the caller's report will label it heuristic.
    #[cfg(feature = "solver")]
    if strategy == PlacementStrategy::Exact {
        let params = crate::exact::placement::ExactPlacementParams::default();
        match crate::exact::placement::place_exact(&req.graph, params) {
            Ok(exact) => {
                if exact.outcome == crate::exact::SolverOutcome::Timeout {
                    // z3 exceeded its time budget: the returned model is a
                    // best-effort guess, NOT a proven optimum. Fall back to
                    // the best heuristic and log the optimality gap so the
                    // schedule is labelled heuristic — never silently
                    // mislabelled `exact` (issue #302: "no silent
                    // heuristic-only fallback without logging").
                    let exact_score = exact.result.score;
                    let heuristic = place(req, PlacementStrategy::InteractionClustering)?;
                    eprintln!(
                        "[quon_na] exact placement solver timed out (z3 returned                          `unknown`); fell back to interaction_clustering                          (best-effort exact score={exact_score:.4},                          heuristic score={:.4})",
                        heuristic.score
                    );
                    return Ok(heuristic);
                }
                let mut result = exact.result;
                result.request.layers = std::mem::take(&mut req.layers);
                return Ok(result);
            }
            Err(msg) => {
                // z3 failed outright (infeasible / no model): fall back to
                // the best heuristic with a logged warning.
                eprintln!(
                    "[quon_na] exact placement solver failed ({msg}), \
                     falling back to interaction_clustering"
                );
                return place(req, PlacementStrategy::InteractionClustering);
            }
        }
    }
    #[cfg(not(feature = "solver"))]
    if strategy == PlacementStrategy::Exact {
        eprintln!(
            "[quon_na] exact placement requires the `solver` feature, \
             falling back to interaction_clustering"
        );
        return place(req, PlacementStrategy::InteractionClustering);
    }

    let layout = build_layout(&req.graph, strategy)?;
    validate_bindings(&req.graph, &layout)?;
    let score = placement_score(&req.graph, &layout)?;
    let axis_aligned_pairs = count_axis_aligned(&req.graph, &layout)?;
    req.layout = Some(layout);

    Ok(PlacementResult {
        request: req,
        score,
        strategy,
        axis_aligned_pairs,
    })
}

/// Score an existing layout against the graph: `Σ w · √(euclidean_µm)`.
pub fn placement_score<V: VertexId>(
    graph: &InteractionGraph<V>,
    layout: &NeutralAtomLayout,
) -> Result<f64, PlacementError<V>> {
    let positions = binding_positions(graph, layout)?;
    let mut score = 0.0;
    for edge in &graph.edges {
        let pa = positions
            .get(&edge.a)
            .ok_or(PlacementError::MissingBinding(edge.a))?;
        let pb = positions
            .get(&edge.b)
            .ok_or(PlacementError::MissingBinding(edge.b))?;
        score += edge.weight * euclidean(pa, pb).sqrt();
    }
    Ok(score)
}

fn build_layout<V: VertexId>(
    graph: &InteractionGraph<V>,
    strategy: PlacementStrategy,
) -> Result<NeutralAtomLayout, PlacementError<V>> {
    let n = graph.vertices.len();
    let (rows, cols) = grid_dims(n);
    let sites = make_sites(rows, cols);
    let site_order = match strategy {
        PlacementStrategy::RowMajor => row_major_sites(rows, cols),
        PlacementStrategy::DegreeBased => spiral_sites(rows, cols),
        PlacementStrategy::InteractionClustering | PlacementStrategy::Exact => {
            // Exact is handled in `place()` before reaching here; if we do
            // arrive, fall back to the clustering site order.
            row_major_sites(rows, cols)
        }
    };

    let qubit_order = match strategy {
        PlacementStrategy::RowMajor => {
            let mut v = graph.vertices.clone();
            v.sort();
            v
        }
        PlacementStrategy::DegreeBased => order_by_weighted_degree(graph),
        PlacementStrategy::InteractionClustering => order_by_clustering(graph, cols),
        PlacementStrategy::Exact => order_by_clustering(graph, cols),
    };

    debug_assert_eq!(qubit_order.len(), site_order.len().min(n));
    let mut initial_bindings = Vec::with_capacity(n);
    for (qubit, &site) in qubit_order.iter().zip(site_order.iter()) {
        initial_bindings.push(AtomBinding {
            atom: AtomId(qubit.index()),
            trap: TrapBinding::Slm { site },
        });
    }

    Ok(NeutralAtomLayout {
        sites,
        initial_bindings,
    })
}

/// Compact rectangle with `rows * cols >= n`.
pub fn grid_dims(n: usize) -> (usize, usize) {
    if n == 0 {
        return (0, 0);
    }
    let cols = (n as f64).sqrt().ceil() as usize;
    let cols = cols.max(1);
    let rows = n.div_ceil(cols);
    (rows, cols)
}

fn make_sites(rows: usize, cols: usize) -> Vec<AtomSite> {
    let mut sites = Vec::with_capacity(rows * cols);
    for r in 0..rows {
        for c in 0..cols {
            let id = SiteId((r * cols + c) as u32);
            sites.push(AtomSite {
                id,
                position: Position {
                    x_um: c as f64 * SITE_PITCH_UM,
                    y_um: r as f64 * SITE_PITCH_UM,
                },
            });
        }
    }
    sites
}

fn row_major_sites(rows: usize, cols: usize) -> Vec<SiteId> {
    let capacity = rows * cols;
    (0..capacity).map(|i| SiteId(i as u32)).collect()
}

/// Ulam-style spiral from the grid center; leftover cells appended row-major.
fn spiral_sites(rows: usize, cols: usize) -> Vec<SiteId> {
    if rows == 0 || cols == 0 {
        return Vec::new();
    }
    let capacity = rows * cols;
    let mut visited = vec![false; capacity];
    let mut order = Vec::with_capacity(capacity);

    let mark = |rr: isize, cc: isize, visited: &mut [bool], order: &mut Vec<SiteId>| {
        if rr >= 0 && cc >= 0 && (rr as usize) < rows && (cc as usize) < cols {
            let idx = rr as usize * cols + cc as usize;
            if !visited[idx] {
                visited[idx] = true;
                order.push(SiteId(idx as u32));
            }
        }
    };

    // Virtual coordinates may leave the rectangle; only in-bounds cells are recorded.
    let mut r = rows as isize / 2;
    let mut c = cols as isize / 2;
    let dirs = [(0isize, 1isize), (1, 0), (0, -1), (-1, 0)];
    let mut dir = 0usize;
    let mut leg_len = 1usize;

    mark(r, c, &mut visited, &mut order);

    // Grow until every cell is visited or the spiral has swept far past the grid.
    let max_legs = (rows + cols) * 2 + 4;
    let mut legs = 0usize;
    while order.len() < capacity && legs < max_legs {
        for _ in 0..2 {
            for _ in 0..leg_len {
                let (dr, dc) = dirs[dir];
                r += dr;
                c += dc;
                mark(r, c, &mut visited, &mut order);
            }
            dir = (dir + 1) % 4;
            legs += 1;
        }
        leg_len += 1;
    }

    for (i, was_visited) in visited.iter().enumerate() {
        if !was_visited {
            order.push(SiteId(i as u32));
        }
    }
    order
}

fn weighted_degrees<V: VertexId>(graph: &InteractionGraph<V>) -> BTreeMap<V, f64> {
    let mut deg: BTreeMap<V, f64> = graph.vertices.iter().map(|&v| (v, 0.0)).collect();
    for edge in &graph.edges {
        *deg.entry(edge.a).or_insert(0.0) += edge.weight;
        *deg.entry(edge.b).or_insert(0.0) += edge.weight;
    }
    deg
}

fn order_by_weighted_degree<V: VertexId>(graph: &InteractionGraph<V>) -> Vec<V> {
    let deg = weighted_degrees(graph);
    let mut verts = graph.vertices.clone();
    verts.sort_by(|a, b| {
        let da = deg.get(a).copied().unwrap_or(0.0);
        let db = deg.get(b).copied().unwrap_or(0.0);
        db.partial_cmp(&da)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(b))
    });
    verts
}

/// Agglomerative clustering: assign degree-descending qubits to the cluster that
/// maximizes intra-cluster weight, then pack clusters into row-major bands.
fn order_by_clustering<V: VertexId>(graph: &InteractionGraph<V>, cols: usize) -> Vec<V> {
    let n = graph.vertices.len();
    if n == 0 {
        return Vec::new();
    }
    let k = cols.max(2).min(n);
    let deg = weighted_degrees(graph);
    let deg_order = order_by_weighted_degree(graph);

    // Adjacency for weight lookups.
    let mut adj: BTreeMap<(V, V), f64> = BTreeMap::new();
    for edge in &graph.edges {
        adj.insert((edge.a, edge.b), edge.weight);
        adj.insert((edge.b, edge.a), edge.weight);
    }
    let weight = |a: V, b: V| -> f64 { adj.get(&(a, b)).copied().unwrap_or(0.0) };

    let mut clusters: Vec<Vec<V>> = vec![Vec::new(); k];

    for (i, &q) in deg_order.iter().enumerate() {
        if i < k {
            // Seed: one qubit per cluster for balance.
            clusters[i].push(q);
            continue;
        }
        let mut best_c = 0usize;
        let mut best_score = f64::NEG_INFINITY;
        for (c_idx, members) in clusters.iter().enumerate() {
            let intra: f64 = members.iter().map(|&m| weight(q, m)).sum();
            // Prefer higher intra weight; break ties toward smaller clusters.
            let score = intra - 1e-9 * (members.len() as f64);
            if score > best_score {
                best_score = score;
                best_c = c_idx;
            }
        }
        clusters[best_c].push(q);
    }

    // Order clusters by total internal weight descending.
    let mut cluster_idxs: Vec<usize> = (0..k).collect();
    cluster_idxs.sort_by(|&a, &b| {
        let wa = cluster_internal_weight(&clusters[a], &weight);
        let wb = cluster_internal_weight(&clusters[b], &weight);
        wb.partial_cmp(&wa)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.cmp(&b))
    });

    let mut ordered = Vec::with_capacity(n);
    for c_idx in cluster_idxs {
        let mut members = clusters[c_idx].clone();
        members.sort_by(|a, b| {
            let da = deg.get(a).copied().unwrap_or(0.0);
            let db = deg.get(b).copied().unwrap_or(0.0);
            db.partial_cmp(&da)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.cmp(b))
        });
        ordered.extend(members);
    }
    ordered
}

fn cluster_internal_weight<V: VertexId>(members: &[V], weight: &dyn Fn(V, V) -> f64) -> f64 {
    let mut total = 0.0;
    for i in 0..members.len() {
        for j in (i + 1)..members.len() {
            total += weight(members[i], members[j]);
        }
    }
    total
}

fn validate_bindings<V: VertexId>(
    graph: &InteractionGraph<V>,
    layout: &NeutralAtomLayout,
) -> Result<(), PlacementError<V>> {
    let mut sites_used = BTreeSet::new();
    let mut atoms_used = BTreeSet::new();
    for binding in &layout.initial_bindings {
        let site = match &binding.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => *site,
        };
        if !sites_used.insert(site) || !atoms_used.insert(binding.atom) {
            return Err(PlacementError::InvalidBindings);
        }
    }
    if layout.initial_bindings.len() != graph.vertices.len() {
        return Err(PlacementError::InvalidBindings);
    }
    let vertex_atoms: BTreeSet<AtomId> = graph.vertices.iter().map(|q| AtomId(q.index())).collect();
    if atoms_used != vertex_atoms {
        return Err(PlacementError::InvalidBindings);
    }
    Ok(())
}

fn binding_positions<V: VertexId>(
    graph: &InteractionGraph<V>,
    layout: &NeutralAtomLayout,
) -> Result<BTreeMap<V, Position>, PlacementError<V>> {
    let site_pos: BTreeMap<SiteId, Position> =
        layout.sites.iter().map(|s| (s.id, s.position)).collect();
    let atom_to_qubit: BTreeMap<AtomId, V> = graph
        .vertices
        .iter()
        .map(|&q| (AtomId(q.index()), q))
        .collect();

    let mut positions = BTreeMap::new();
    for binding in &layout.initial_bindings {
        let qubit = atom_to_qubit
            .get(&binding.atom)
            .copied()
            .ok_or(PlacementError::InvalidBindings)?;
        let site = match &binding.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => *site,
        };
        let pos = site_pos
            .get(&site)
            .copied()
            .ok_or(PlacementError::InvalidBindings)?;
        positions.insert(qubit, pos);
    }
    for &v in &graph.vertices {
        if !positions.contains_key(&v) {
            return Err(PlacementError::MissingBinding(v));
        }
    }
    Ok(positions)
}

fn count_axis_aligned<V: VertexId>(
    graph: &InteractionGraph<V>,
    layout: &NeutralAtomLayout,
) -> Result<u32, PlacementError<V>> {
    let positions = binding_positions(graph, layout)?;
    let mut count = 0u32;
    for edge in &graph.edges {
        let pa = positions
            .get(&edge.a)
            .ok_or(PlacementError::MissingBinding(edge.a))?;
        let pb = positions
            .get(&edge.b)
            .ok_or(PlacementError::MissingBinding(edge.b))?;
        if (pa.x_um - pb.x_um).abs() < 1e-12 || (pa.y_um - pb.y_um).abs() < 1e-12 {
            count += 1;
        }
    }
    Ok(count)
}

fn euclidean(a: &Position, b: &Position) -> f64 {
    let dx = a.x_um - b.x_um;
    let dy = a.y_um - b.y_um;
    (dx * dx + dy * dy).sqrt()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grid_dims_covers_n() {
        for n in 1..=64 {
            let (r, c) = grid_dims(n);
            assert!(r * c >= n, "n={n} -> {r}x{c}");
        }
    }

    #[test]
    fn spiral_covers_all_sites_uniquely() {
        for &(rows, cols) in &[(1, 1), (2, 2), (3, 3), (3, 4), (4, 3), (5, 2)] {
            let order = spiral_sites(rows, cols);
            assert_eq!(order.len(), rows * cols);
            let set: BTreeSet<_> = order.iter().copied().collect();
            assert_eq!(set.len(), rows * cols);
        }
    }
}
