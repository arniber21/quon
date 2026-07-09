//! Property tests for `schedule_from_graph` (issue #103).
//!
//! Generators emit valid interaction graphs (Erdős–Rényi-ish and small 3-regular)
//! and assert the stub request succeeds with empty layers/layout.
//! Literature benches use n ≈ 30–90+; CI keeps n ≤ 64 / cubic ≤ 12 for speed.

use std::collections::BTreeSet;

use proptest::prelude::*;
use quon_na::{cubic_commutation_graph, erdos_renyi_commutation_graph, schedule_from_graph};

fn er_strategy() -> impl Strategy<Value = (u32, Vec<(u32, u32)>)> {
    (2u32..=64).prop_flat_map(|n| {
        prop::collection::vec((0..n, 0..n), 0..(n as usize * 2)).prop_map(move |raw| {
            let edges: Vec<(u32, u32)> = raw
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
    #[test]
    fn schedule_from_graph_accepts_erdos_renyi((n, edges) in er_strategy()) {
        let graph = erdos_renyi_commutation_graph(n, &edges).expect("valid ER graph");
        let req = schedule_from_graph(graph.clone()).expect("schedule stub");
        assert_eq!(req.graph, graph);
        assert!(req.layers.is_empty());
        assert!(req.layout.is_none());
    }
}

proptest! {
    #[test]
    fn schedule_from_graph_accepts_cubic(n in prop::sample::select(vec![4u32, 6, 8, 10, 12])) {
        let graph = cubic_commutation_graph(n).expect("cubic");
        let req = schedule_from_graph(graph.clone()).expect("schedule stub");
        assert_eq!(req.graph, graph);
        assert!(req.layers.is_empty());
        assert!(req.layout.is_none());
        assert_eq!(req.graph.edges.len() * 2, (n as usize) * 3);
    }
}
