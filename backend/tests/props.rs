// Property-based and differential tests for the backend crate — issue #3.
//
// - Property tests assert the Floyd-Warshall invariants (identity, symmetry,
//   triangle inequality, adjacency) over random graphs.
// - Differential tests compare our Floyd-Warshall distance matrix against an
//   independent shortest-path implementation (petgraph's Dijkstra).
// - A round-trip property checks descriptor -> JSON -> domain preserves the
//   topology and native gate set.

use std::collections::BTreeSet;

use backend::gates::STD_GATES;
use backend::target::{ConnectivityGraph, UNREACHABLE};
use petgraph::algo::dijkstra;
use petgraph::graph::{NodeIndex, UnGraph};
use proptest::prelude::*;

/// Independent all-pairs shortest paths via petgraph's Dijkstra (unit weights).
/// A different algorithm and codebase from our Floyd-Warshall, so agreement is
/// a strong correctness signal.
fn oracle_distances(n: usize, edges: &[(usize, usize)]) -> Vec<Vec<usize>> {
    let mut g = UnGraph::<(), ()>::new_undirected();
    let nodes: Vec<NodeIndex> = (0..n).map(|_| g.add_node(())).collect();
    for &(u, v) in edges {
        g.add_edge(nodes[u], nodes[v], ());
    }
    let mut dist = vec![vec![UNREACHABLE; n]; n];
    for i in 0..n {
        let shortest = dijkstra(&g, nodes[i], None, |_| 1usize);
        for (j, row_entry) in dist[i].iter_mut().enumerate() {
            if let Some(d) = shortest.get(&nodes[j]) {
                *row_entry = *d;
            }
        }
    }
    dist
}

/// Random simple undirected graph: `2..=10` qubits and a deduplicated,
/// self-loop-free edge set.
fn graph_strategy() -> impl Strategy<Value = (usize, Vec<(usize, usize)>)> {
    (2usize..=10).prop_flat_map(|n| {
        prop::collection::vec((0..n, 0..n), 0..(n * 2)).prop_map(move |raw| {
            let edges: Vec<(usize, usize)> = raw
                .into_iter()
                .filter(|(a, b)| a != b)
                .map(|(a, b)| if a < b { (a, b) } else { (b, a) })
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            (n, edges)
        })
    })
}

proptest! {
    /// Our Floyd-Warshall matches the independent Dijkstra oracle exactly.
    #[test]
    fn floyd_warshall_matches_petgraph((n, edges) in graph_strategy()) {
        let graph = ConnectivityGraph::try_from_edges(n, edges.clone()).unwrap();
        let oracle = oracle_distances(n, &edges);
        for (i, oracle_row) in oracle.iter().enumerate() {
            for (j, &oracle_d) in oracle_row.iter().enumerate() {
                prop_assert_eq!(graph.dist(i, j), oracle_d, "dist({}, {})", i, j);
            }
        }
    }

    /// Metric invariants of a unit-weight undirected shortest-path matrix.
    #[test]
    fn distance_matrix_invariants((n, edges) in graph_strategy()) {
        let graph = ConnectivityGraph::try_from_edges(n, edges.clone()).unwrap();
        let edge_set: BTreeSet<(usize, usize)> = edges.iter().copied().collect();

        for i in 0..n {
            prop_assert_eq!(graph.dist(i, i), 0);
            for j in 0..n {
                // Symmetry.
                prop_assert_eq!(graph.dist(i, j), graph.dist(j, i));
                // Direct neighbours are exactly the edges, at distance 1.
                let key = if i < j { (i, j) } else { (j, i) };
                if i != j && edge_set.contains(&key) {
                    prop_assert_eq!(graph.dist(i, j), 1);
                }
            }
        }

        // Triangle inequality over finite distances.
        for i in 0..n {
            for k in 0..n {
                for j in 0..n {
                    let (ik, kj, ij) = (graph.dist(i, k), graph.dist(k, j), graph.dist(i, j));
                    if ik < UNREACHABLE && kj < UNREACHABLE {
                        prop_assert!(ij <= ik + kj);
                    }
                }
            }
        }
    }

    /// A descriptor built from a random graph survives the JSON round trip with
    /// its topology and native gate set intact.
    #[test]
    fn json_round_trip_preserves_topology_and_gates(
        (n, edges) in graph_strategy(),
        gate_mask in prop::collection::vec(any::<bool>(), STD_GATES.len()),
    ) {
        let gates: Vec<&str> = STD_GATES
            .iter()
            .zip(&gate_mask)
            .filter_map(|((name, _), &keep)| keep.then_some(*name))
            .collect();

        let gates_json = gates
            .iter()
            .map(|g| format!("\"{g}\""))
            .collect::<Vec<_>>()
            .join(",");
        let edges_json = edges
            .iter()
            .map(|(a, b)| format!("[{a},{b}]"))
            .collect::<Vec<_>>()
            .join(",");
        let src = format!(
            r#"{{"id":"g","num_qubits":{n},"topology":{{"edges":[{edges_json}]}},
               "native_gates":[{gates_json}],"meas_latency_us":0.0,
               "supports_mid_circuit_meas":false,"supports_feed_forward":false}}"#
        );

        let target = backend::json::from_str(&src).unwrap();
        let reloaded = backend::json::from_str(
            &serde_json::to_string(&target.to_descriptor()).unwrap(),
        )
        .unwrap();

        prop_assert_eq!(&reloaded.topology.edges, &target.topology.edges);
        let mut got: Vec<&str> = reloaded.native_gates.iter().map(|g| g.name.as_str()).collect();
        got.sort_unstable();
        let mut want = gates.clone();
        want.sort_unstable();
        prop_assert_eq!(got, want);
    }
}
