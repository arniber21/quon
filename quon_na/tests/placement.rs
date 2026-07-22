//! Placement passes (issue #104): overlap-free layouts and score comparisons.

use std::collections::BTreeSet;

use quon_na::{
    AtomId, DEFAULT_GAMMA, Interaction, InteractionGraph, InteractionId, InteractionSegment,
    LogicalQubitId, PlacementStrategy, SegmentKind, TrapBinding, cubic_commutation_graph,
    erdos_renyi_commutation_graph, place, schedule_from_graph,
};

fn cz(id: u32, a: u32, b: u32) -> Interaction {
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

fn assert_no_overlap(result: &quon_na::PlacementResult) {
    let layout = result.request.layout.as_ref().expect("layout filled");
    let mut sites = BTreeSet::new();
    let mut atoms = BTreeSet::new();
    for b in &layout.initial_bindings {
        let site = match &b.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => *site,
        };
        assert!(sites.insert(site), "site overlap {:?}", site);
        assert!(atoms.insert(b.atom), "atom overlap {:?}", b.atom);
    }
    assert_eq!(
        layout.initial_bindings.len(),
        result.request.graph.vertices.len()
    );
    let expected: BTreeSet<_> = result
        .request
        .graph
        .vertices
        .iter()
        .map(|q| AtomId(q.0))
        .collect();
    assert_eq!(atoms, expected);
    assert!(result.score.is_finite());
    assert!(result.score >= 0.0);
}

fn two_clique_bridge_graph() -> InteractionGraph {
    // Two K4 cliques on even/odd ids so row-major interleaves them; clustering
    // should pack each clique contiguously and beat the baseline score.
    // Clique A: {0,2,4,6}, Clique B: {1,3,5,7}, bridge 6–7.
    let mut interactions = Vec::new();
    let mut id = 0u32;
    for &(a, b) in &[
        (0, 2),
        (0, 4),
        (0, 6),
        (2, 4),
        (2, 6),
        (4, 6),
        (1, 3),
        (1, 5),
        (1, 7),
        (3, 5),
        (3, 7),
        (5, 7),
        (6, 7),
    ] {
        interactions.push(cz(id, a, b));
        id += 1;
    }
    let ids: Vec<_> = (0..id).map(InteractionId).collect();
    InteractionGraph::from_interactions(
        (0..8).map(LogicalQubitId).collect(),
        interactions,
        vec![InteractionSegment {
            kind: SegmentKind::CommutationGroup,
            interactions: ids,
        }],
        DEFAULT_GAMMA,
    )
    .expect("two-clique graph")
}

#[test]
fn row_major_no_overlap() {
    let g = cubic_commutation_graph(8).unwrap();
    let req = schedule_from_graph(g).unwrap();
    let result = place(req, PlacementStrategy::RowMajor).unwrap();
    assert_eq!(result.strategy, PlacementStrategy::RowMajor);
    assert_no_overlap(&result);
}

#[test]
fn degree_based_no_overlap() {
    let g = cubic_commutation_graph(8).unwrap();
    let req = schedule_from_graph(g).unwrap();
    let result = place(req, PlacementStrategy::DegreeBased).unwrap();
    assert_no_overlap(&result);
}

#[test]
fn clustering_no_overlap() {
    let g = cubic_commutation_graph(8).unwrap();
    let req = schedule_from_graph(g).unwrap();
    let result = place(req, PlacementStrategy::InteractionClustering).unwrap();
    assert_no_overlap(&result);
}

#[test]
fn all_strategies_on_benchmarks_without_overlap() {
    let benchmarks: Vec<InteractionGraph> = vec![
        cubic_commutation_graph(4).unwrap(),
        cubic_commutation_graph(6).unwrap(),
        cubic_commutation_graph(8).unwrap(),
        cubic_commutation_graph(12).unwrap(),
        erdos_renyi_commutation_graph(10, &[(0, 1), (1, 2), (2, 3), (3, 4), (4, 0), (1, 4)])
            .unwrap(),
        two_clique_bridge_graph(),
    ];
    let strategies = [
        PlacementStrategy::RowMajor,
        PlacementStrategy::DegreeBased,
        PlacementStrategy::InteractionClustering,
    ];
    for graph in benchmarks {
        for strategy in strategies {
            let req = schedule_from_graph(graph.clone()).unwrap();
            let result = place(req, strategy).unwrap();
            assert_no_overlap(&result);
            assert!(result.request.layers.is_empty());
        }
    }
}

#[test]
fn clustering_improves_score_over_row_major() {
    let graph = two_clique_bridge_graph();
    let row = place(
        schedule_from_graph(graph.clone()).unwrap(),
        PlacementStrategy::RowMajor,
    )
    .unwrap();
    let cluster = place(
        schedule_from_graph(graph).unwrap(),
        PlacementStrategy::InteractionClustering,
    )
    .unwrap();
    assert_no_overlap(&row);
    assert_no_overlap(&cluster);
    assert!(
        cluster.score < row.score,
        "clustering score {} should beat row-major {}",
        cluster.score,
        row.score
    );
    eprintln!(
        "score evidence: row-major={:.6} clustering={:.6} improvement={:.6}",
        row.score,
        cluster.score,
        row.score - cluster.score
    );
}

#[test]
fn place_preserves_graph_and_layers() {
    let graph = cubic_commutation_graph(6).unwrap();
    let req = schedule_from_graph(graph.clone()).unwrap();
    let result = place(req, PlacementStrategy::DegreeBased).unwrap();
    assert_eq!(result.request.graph, graph);
    assert!(result.request.layers.is_empty());
    assert!(result.request.layout.is_some());
}

#[test]
fn place_rejects_empty_graph() {
    let graph: InteractionGraph =
        InteractionGraph::from_interactions(vec![], vec![], vec![], DEFAULT_GAMMA).unwrap();
    let req = schedule_from_graph(graph).unwrap();
    let err = place(req, PlacementStrategy::RowMajor).unwrap_err();
    assert!(matches!(err, quon_na::PlacementError::EmptyGraph));
}

#[test]
fn noncontiguous_qubit_ids_place_cleanly() {
    let graph = InteractionGraph::from_interactions(
        vec![LogicalQubitId(2), LogicalQubitId(5), LogicalQubitId(9)],
        vec![cz(0, 2, 5), cz(1, 5, 9)],
        vec![InteractionSegment {
            kind: SegmentKind::CommutationGroup,
            interactions: vec![InteractionId(0), InteractionId(1)],
        }],
        DEFAULT_GAMMA,
    )
    .unwrap();
    let result = place(
        schedule_from_graph(graph).unwrap(),
        PlacementStrategy::RowMajor,
    )
    .unwrap();
    assert_no_overlap(&result);
    let atoms: BTreeSet<_> = result
        .request
        .layout
        .as_ref()
        .unwrap()
        .initial_bindings
        .iter()
        .map(|b| b.atom.0)
        .collect();
    assert_eq!(atoms, BTreeSet::from([2, 5, 9]));
}
