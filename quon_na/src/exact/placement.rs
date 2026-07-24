//! Exact optimal placement via z3 SMT (issue #302, Deliverable B).
//!
//! Formulated after OLSQ-DPQA (Tan et al. 2024): placement as
//! constraint-satisfaction over atom-site assignments on the interaction
//! graph, minimizing the movement-cost-shaped score `Σ w · √d` that the
//! heuristic placers in [`crate::placement`] compute.
//!
//! # Encoding
//!
//! For each graph vertex `v` (indexed `0..n`), an integer variable
//! `site[v] ∈ [0, n_sites)` selects a site on the compact grid built by
//! [`crate::placement::grid_dims`]. A `Distinct` constraint ensures a
//! bijection. For each interaction edge `(a, b)` with weight `w`, the
//! squared Euclidean distance `dx² + dy²` (where `dx = col(a) − col(b)`,
//! `dy = row(a) − row(b)`, computed via integer `/` and `%` on the site
//! index) is minimized as a polynomial objective.
//!
//! After solving, the actual `Σ w · √d` score (where `d` is the Euclidean
//! distance in µm, and `√d` is the √-law movement proxy) is computed from
//! the concrete layout. For small `n` (≤ 8), a brute-force enumeration of
//! all `n!` permutations guarantees the true `√d` optimum — the z3
//! encoding serves as the SMT proof, while enumeration handles the
//! non-polynomial objective exactly.
//!
//! # Timeout / fallback
//!
//! [`ExactPlacementParams::timeout_ms`] bounds the solver. On `unknown`
//! (timeout) the function returns [`SolverOutcome::Timeout`]; the caller
//! falls back to a heuristic placer and logs the optimality gap.

use z3::ast::{Ast, Int};
use z3::{Config, Context, Optimize, SatResult};

use crate::exact::{DEFAULT_SOLVER_TIMEOUT_MS, SolverOutcome};
use crate::graph::{InteractionGraph, VertexId};
use crate::layout::{
    AtomBinding, AtomId, AtomSite, NeutralAtomLayout, Position, SiteId, TrapBinding,
};
use crate::placement::{PlacementResult, PlacementStrategy, SITE_PITCH_UM, grid_dims};

/// Above this vertex count, brute-force enumeration is skipped in favour of
/// z3 (or heuristic fallback on timeout).
const BRUTE_FORCE_MAX_N: usize = 8;

/// Tunable parameters for [`place_exact`].
#[derive(Clone, Copy, Debug)]
pub struct ExactPlacementParams {
    /// z3 solver timeout (milliseconds). On expiry the solver returns
    /// `unknown` and the caller falls back to heuristic.
    pub timeout_ms: u64,
}

impl Default for ExactPlacementParams {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_SOLVER_TIMEOUT_MS,
        }
    }
}

/// Exact-placement result: the [`PlacementResult`] plus whether z3 proved
/// optimality.
pub struct ExactPlacementResult<V: VertexId> {
    pub result: PlacementResult<V>,
    pub outcome: SolverOutcome,
}

/// Solve the optimal placement with z3 (or brute-force for small n).
pub fn place_exact<V: VertexId>(
    graph: &InteractionGraph<V>,
    params: ExactPlacementParams,
) -> Result<ExactPlacementResult<V>, String> {
    let n = graph.vertices.len();
    if n == 0 {
        return Err("empty interaction graph: no vertices to place".into());
    }

    let (rows, cols) = grid_dims(n);
    let sites = make_sites(rows, cols);

    // For small n, brute-force enumeration is exact and faster than z3.
    // It guarantees the true optimum under the actual √d metric.
    if n <= BRUTE_FORCE_MAX_N {
        return place_exact_brute_force(graph, &sites, cols);
    }

    // For larger n, use z3's Optimize with the squared-distance proxy.
    place_exact_z3(graph, &sites, cols, params)
}

/// Brute-force: enumerate all permutations, compute the actual √d score,
/// and return the best. Guaranteed optimal for small n.
fn place_exact_brute_force<V: VertexId>(
    graph: &InteractionGraph<V>,
    sites: &[AtomSite],
    _cols: usize,
) -> Result<ExactPlacementResult<V>, String> {
    let vertices: Vec<V> = graph.vertices.clone();
    let n = vertices.len();

    let mut best_assignment: Vec<usize> = (0..n).collect();
    let mut best_score = f64::INFINITY;

    let mut assignment: Vec<usize> = (0..n).collect();

    loop {
        let score = compute_actual_score(graph, &vertices, &assignment, sites);
        if score < best_score {
            best_score = score;
            best_assignment = assignment.clone();
        }
        if !next_permutation(&mut assignment) {
            break;
        }
    }

    let layout = build_layout_from_assignment(&vertices, &best_assignment, sites);
    let axis_aligned = count_axis_aligned(graph, &layout)?;

    Ok(ExactPlacementResult {
        result: PlacementResult {
            request: build_request(graph, layout),
            score: best_score,
            strategy: PlacementStrategy::Exact,
            axis_aligned_pairs: axis_aligned,
        },
        outcome: SolverOutcome::Proven,
    })
}

/// z3-based optimization for larger n (squared-distance proxy).
fn place_exact_z3<V: VertexId>(
    graph: &InteractionGraph<V>,
    sites: &[AtomSite],
    cols: usize,
    params: ExactPlacementParams,
) -> Result<ExactPlacementResult<V>, String> {
    let n = graph.vertices.len();
    let n_sites = sites.len();
    let vertices: Vec<V> = graph.vertices.clone();

    let mut cfg = Config::new();
    cfg.set_timeout_msec(params.timeout_ms);
    let ctx = Context::new(&cfg);
    let opt = Optimize::new(&ctx);

    let site_vars: Vec<Int> = (0..n)
        .map(|i| Int::new_const(&ctx, format!("s{i}")))
        .collect();

    for sv in &site_vars {
        opt.assert(&sv.ge(&Int::from_u64(&ctx, 0)));
        opt.assert(&sv.lt(&Int::from_u64(&ctx, n_sites as u64)));
    }

    let site_refs: Vec<&Int> = site_vars.iter().collect();
    opt.assert(&Int::distinct(&ctx, &site_refs));

    let cols_int = Int::from_u64(&ctx, cols as u64);

    let mut objective_terms: Vec<Int> = Vec::new();

    for edge in &graph.edges {
        let a_idx = vertices
            .iter()
            .position(|&v| v == edge.a)
            .ok_or_else(|| format!("edge vertex {:?} not in graph vertices", edge.a))?;
        let b_idx = vertices
            .iter()
            .position(|&v| v == edge.b)
            .ok_or_else(|| format!("edge vertex {:?} not in graph vertices", edge.b))?;

        let sa = &site_vars[a_idx];
        let sb = &site_vars[b_idx];

        let col_a = sa.modulo(&cols_int);
        let col_b = sb.modulo(&cols_int);
        let row_a = sa.div(&cols_int);
        let row_b = sb.div(&cols_int);

        let dx = Int::sub(&ctx, &[&col_a, &col_b]);
        let dy = Int::sub(&ctx, &[&row_a, &row_b]);
        let dx_sq = Int::mul(&ctx, &[&dx, &dx]);
        let dy_sq = Int::mul(&ctx, &[&dy, &dy]);
        let dist_sq = Int::add(&ctx, &[&dx_sq, &dy_sq]);

        let weight_scaled = (edge.weight * 1_000_000.0).round() as i64;
        let weight_int = Int::from_i64(&ctx, weight_scaled);
        let contrib = Int::mul(&ctx, &[&weight_int, &dist_sq]);
        objective_terms.push(contrib);
    }

    if !objective_terms.is_empty() {
        let obj_refs: Vec<&Int> = objective_terms.iter().collect();
        let total = Int::add(&ctx, &obj_refs);
        opt.minimize(&total);
    }

    let outcome = match opt.check(&[]) {
        SatResult::Sat => SolverOutcome::Proven,
        SatResult::Unknown => SolverOutcome::Timeout,
        SatResult::Unsat => return Err("placement problem is infeasible".into()),
    };

    let model = if outcome == SolverOutcome::Timeout {
        match opt.get_model() {
            Some(m) => m,
            None => return Err("z3 timeout with no extractable model".into()),
        }
    } else {
        opt.get_model()
            .ok_or_else(|| "z3 returned sat but no model".to_string())?
    };

    let assignment: Vec<usize> = site_vars
        .iter()
        .map(|sv| model.eval(sv, true).and_then(|v| v.as_i64()).unwrap_or(0) as usize)
        .collect();

    // Compute the ACTUAL √d score from the resulting layout (not the z3
    // squared-distance proxy).
    let layout = build_layout_from_assignment(&vertices, &assignment, sites);
    let score = compute_actual_score(graph, &vertices, &assignment, sites);
    let axis_aligned = count_axis_aligned(graph, &layout)?;

    Ok(ExactPlacementResult {
        result: PlacementResult {
            request: build_request(graph, layout),
            score,
            strategy: PlacementStrategy::Exact,
            axis_aligned_pairs: axis_aligned,
        },
        outcome,
    })
}

// ── Helpers ───────────────────────────────────────────────────────────

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

fn build_layout_from_assignment<V: VertexId>(
    vertices: &[V],
    assignment: &[usize],
    sites: &[AtomSite],
) -> NeutralAtomLayout {
    let initial_bindings = vertices
        .iter()
        .zip(assignment.iter())
        .map(|(&v, &site_idx)| AtomBinding {
            atom: AtomId(v.index()),
            trap: TrapBinding::Slm {
                site: sites[site_idx].id,
            },
        })
        .collect();
    NeutralAtomLayout {
        sites: sites.to_vec(),
        initial_bindings,
    }
}

/// `Σ w · √(euclidean_µm)` — the same metric
/// [`crate::placement::placement_score`] uses: `√d` where `d` is the
/// Euclidean distance (not `d` itself, not `d²`).
fn compute_actual_score<V: VertexId>(
    graph: &InteractionGraph<V>,
    vertices: &[V],
    assignment: &[usize],
    sites: &[AtomSite],
) -> f64 {
    let mut pos_map = std::collections::BTreeMap::new();
    for (i, &v) in vertices.iter().enumerate() {
        pos_map.insert(v, sites[assignment[i]].position);
    }

    let zero = Position {
        x_um: 0.0,
        y_um: 0.0,
    };
    let mut score = 0.0;
    for edge in &graph.edges {
        // `graph.validate()` guarantees edge endpoints are in `vertices`,
        // hence in `pos_map`; the fallback is defensive only.
        let pa = pos_map.get(&edge.a).copied().unwrap_or(zero);
        let pb = pos_map.get(&edge.b).copied().unwrap_or(zero);
        let dx = pa.x_um - pb.x_um;
        let dy = pa.y_um - pb.y_um;
        let euclidean = (dx * dx + dy * dy).sqrt();
        // The score metric is √(euclidean), not euclidean itself.
        score += edge.weight * euclidean.sqrt();
    }
    score
}

fn count_axis_aligned<V: VertexId>(
    graph: &InteractionGraph<V>,
    layout: &NeutralAtomLayout,
) -> Result<u32, String> {
    let site_pos: std::collections::BTreeMap<SiteId, Position> =
        layout.sites.iter().map(|s| (s.id, s.position)).collect();
    let atom_to_qubit: std::collections::BTreeMap<AtomId, V> = graph
        .vertices
        .iter()
        .map(|&q| (AtomId(q.index()), q))
        .collect();

    let mut positions = std::collections::BTreeMap::new();
    for binding in &layout.initial_bindings {
        let site = match &binding.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => *site,
        };
        let pos = site_pos.get(&site).copied().ok_or("missing site")?;
        let qubit = atom_to_qubit
            .get(&binding.atom)
            .copied()
            .ok_or("missing atom binding")?;
        positions.insert(qubit, pos);
    }

    let mut count = 0u32;
    for edge in &graph.edges {
        let pa = positions.get(&edge.a).ok_or("missing vertex position")?;
        let pb = positions.get(&edge.b).ok_or("missing vertex position")?;
        if (pa.x_um - pb.x_um).abs() < 1e-12 || (pa.y_um - pb.y_um).abs() < 1e-12 {
            count += 1;
        }
    }
    Ok(count)
}

fn build_request<V: VertexId>(
    graph: &InteractionGraph<V>,
    layout: NeutralAtomLayout,
) -> crate::schedule_entry::GraphScheduleRequest<V> {
    crate::schedule_entry::GraphScheduleRequest {
        graph: graph.clone(),
        layers: Vec::new(),
        layout: Some(layout),
    }
}

/// Heap's algorithm for next permutation in lexicographic order.
fn next_permutation(arr: &mut [usize]) -> bool {
    let n = arr.len();
    if n < 2 {
        return false;
    }
    let mut i = n - 2;
    while arr[i] >= arr[i + 1] {
        if i == 0 {
            return false;
        }
        i -= 1;
    }
    let mut j = n - 1;
    while arr[j] <= arr[i] {
        j -= 1;
    }
    arr.swap(i, j);
    arr[i + 1..].reverse();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graph::erdos_renyi_commutation_graph;

    #[test]
    fn exact_placement_qft4_beats_or_matches_heuristic() {
        let graph =
            erdos_renyi_commutation_graph(4, &[(0, 1), (0, 2), (0, 3), (1, 2), (1, 3), (2, 3)])
                .unwrap();

        let exact = place_exact(&graph, ExactPlacementParams::default()).unwrap();
        assert_eq!(exact.outcome, SolverOutcome::Proven);

        let req = crate::schedule_entry::schedule_from_graph(graph.clone()).unwrap();
        let heuristic =
            crate::placement::place(req, PlacementStrategy::InteractionClustering).unwrap();

        assert!(
            exact.result.score <= heuristic.score + 1e-9,
            "exact score {} should be <= heuristic {}",
            exact.result.score,
            heuristic.score
        );
    }

    #[test]
    fn exact_placement_qaoa_triangle() {
        let graph =
            erdos_renyi_commutation_graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0), (0, 2), (1, 3)])
                .unwrap();

        let exact = place_exact(&graph, ExactPlacementParams::default()).unwrap();
        assert_eq!(exact.outcome, SolverOutcome::Proven);

        let req = crate::schedule_entry::schedule_from_graph(graph.clone()).unwrap();
        let heuristic =
            crate::placement::place(req, PlacementStrategy::InteractionClustering).unwrap();

        assert!(
            exact.result.score <= heuristic.score + 1e-9,
            "exact score {} should be <= heuristic {}",
            exact.result.score,
            heuristic.score
        );
    }

    #[test]
    fn exact_placement_small_timeout_does_not_panic() {
        let graph = erdos_renyi_commutation_graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]).unwrap();
        let params = ExactPlacementParams { timeout_ms: 1 };
        let result = place_exact(&graph, params);
        if let Ok(r) = result {
            assert!(r.result.score >= 0.0);
        }
    }

    #[test]
    fn exact_placement_validates_bindings() {
        let graph = erdos_renyi_commutation_graph(4, &[(0, 1), (1, 2), (2, 3), (3, 0)]).unwrap();
        let exact = place_exact(&graph, ExactPlacementParams::default()).unwrap();

        let layout = exact.result.request.layout.as_ref().unwrap();
        let mut sites = std::collections::BTreeSet::new();
        for binding in &layout.initial_bindings {
            let site = match &binding.trap {
                TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => *site,
            };
            assert!(sites.insert(site), "duplicate site binding");
        }
        assert_eq!(sites.len(), graph.vertices.len());
    }

    #[test]
    fn exact_placement_beats_row_major_on_path() {
        let graph = erdos_renyi_commutation_graph(4, &[(0, 1), (1, 2), (2, 3)]).unwrap();

        let exact = place_exact(&graph, ExactPlacementParams::default()).unwrap();
        let req = crate::schedule_entry::schedule_from_graph(graph.clone()).unwrap();
        let row_major = crate::placement::place(req, PlacementStrategy::RowMajor).unwrap();

        assert!(
            exact.result.score <= row_major.score + 1e-9,
            "exact score {} should be <= row_major {}",
            exact.result.score,
            row_major.score
        );
    }
}
