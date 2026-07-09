//! Property tests for Misra–Gries entangling-layer scheduling (#105).

use std::collections::BTreeSet;

use proptest::prelude::*;
use quon_na::{
    cubic_commutation_graph, erdos_renyi_commutation_graph, schedule_entangling_layers,
    schedule_from_graph,
};

fn er_strategy() -> impl Strategy<Value = (u32, Vec<(u32, u32)>)> {
    (2u32..=32).prop_flat_map(|n| {
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
    fn entangling_schedule_erdos_renyi_conflict_free((n, edges) in er_strategy()) {
        let graph = erdos_renyi_commutation_graph(n, &edges).expect("valid ER graph");
        let has_edges = !graph.interactions.is_empty();
        let req = schedule_from_graph(graph).expect("stub");
        let result = schedule_entangling_layers(req, 340).expect("schedule");
        if has_edges {
            assert!(!result.request.layers.is_empty());
            assert!(result.misra_gries_applied);
            assert!(result.request.layers.len() <= (result.max_degree as usize) + 1);
        } else {
            assert!(result.request.layers.is_empty());
        }
        for layer in &result.request.layers {
            layer.validate_conflicts().expect("conflicts");
        }
        assert!(result.request.layout.is_none());
    }
}

proptest! {
    #[test]
    fn cubic_commutation_at_most_four_layers(n in prop::sample::select(vec![4u32, 6, 8, 10, 12])) {
        let graph = cubic_commutation_graph(n).expect("cubic");
        let req = schedule_from_graph(graph).expect("stub");
        let result = schedule_entangling_layers(req, 340).expect("schedule");
        assert_eq!(result.max_degree, 3);
        assert!(result.misra_gries_applied);
        assert!(
            result.request.layers.len() <= 4,
            "n={n} got {} layers",
            result.request.layers.len()
        );
        for layer in &result.request.layers {
            layer.validate_conflicts().expect("conflicts");
        }
    }
}
