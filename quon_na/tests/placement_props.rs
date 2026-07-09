//! Property tests for placement (issue #104).

use std::collections::BTreeSet;

use proptest::prelude::*;
use quon_na::{
    PlacementStrategy, TrapBinding, cubic_commutation_graph, erdos_renyi_commutation_graph, place,
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

fn assert_valid_placement(result: &quon_na::PlacementResult) {
    let layout = result.request.layout.as_ref().expect("layout");
    let mut sites = BTreeSet::new();
    for b in &layout.initial_bindings {
        let site = match &b.trap {
            TrapBinding::Slm { site } | TrapBinding::Aod { site, .. } => *site,
        };
        assert!(sites.insert(site));
    }
    assert_eq!(
        layout.initial_bindings.len(),
        result.request.graph.vertices.len()
    );
    assert!(result.score.is_finite());
    assert!(result.score >= 0.0);
}

proptest! {
    #[test]
    fn all_strategies_no_overlap_erdos_renyi((n, edges) in er_strategy()) {
        let graph = erdos_renyi_commutation_graph(n, &edges).expect("er");
        for strategy in [
            PlacementStrategy::RowMajor,
            PlacementStrategy::DegreeBased,
            PlacementStrategy::InteractionClustering,
        ] {
            let result = place(schedule_from_graph(graph.clone()).unwrap(), strategy).unwrap();
            assert_valid_placement(&result);
        }
    }
}

proptest! {
    #[test]
    fn all_strategies_no_overlap_cubic(n in prop::sample::select(vec![4u32, 6, 8, 10, 12])) {
        let graph = cubic_commutation_graph(n).expect("cubic");
        for strategy in [
            PlacementStrategy::RowMajor,
            PlacementStrategy::DegreeBased,
            PlacementStrategy::InteractionClustering,
        ] {
            let result = place(schedule_from_graph(graph.clone()).unwrap(), strategy).unwrap();
            assert_valid_placement(&result);
        }
    }
}
