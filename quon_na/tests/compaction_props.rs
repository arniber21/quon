//! Property tests for schedule compaction (#108).

use proptest::prelude::*;
use quon_na::{
    CompactionOptions, asap_schedule_layers, compact_schedule, cubic_commutation_graph,
    erdos_renyi_commutation_graph, infer_atom_dependencies, schedule_entangling_layers,
    schedule_from_graph,
};

fn er_strategy() -> impl Strategy<Value = (u32, Vec<(u32, u32)>)> {
    (2u32..=24).prop_flat_map(|n| {
        prop::collection::vec((0..n, 0..n), 0..(n as usize * 2)).prop_map(move |raw| {
            let edges: Vec<(u32, u32)> = raw
                .into_iter()
                .filter(|(a, b)| a != b)
                .map(|(a, b)| if a < b { (a, b) } else { (b, a) })
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect();
            (n, edges)
        })
    })
}

proptest! {
    #[test]
    fn compaction_preserves_conflicts_and_beats_asap(
        (n, edges) in er_strategy()
    ) {
        let graph = erdos_renyi_commutation_graph(n, &edges).expect("valid ER graph");
        let req = schedule_from_graph(graph).expect("stub");
        let scheduled = schedule_entangling_layers(req, 340).expect("schedule");
        if scheduled.request.layers.is_empty() {
            return Ok(());
        }
        let deps = infer_atom_dependencies(&scheduled.request.layers);
        let asap = asap_schedule_layers(scheduled.request.clone(), &deps).expect("asap");
        let opts = CompactionOptions {
            greedy: true,
            ..Default::default()
        };
        let compacted = compact_schedule(scheduled.request, &deps, &opts).expect("compact");
        for layer in &compacted.request.layers {
            layer.validate_conflicts().expect("conflicts");
            layer.validate_occupancy().expect("occupancy");
        }
        prop_assert!(
            compacted.compacted_makespan_cycles <= asap.asap_makespan_cycles,
            "compacted {} > asap {}",
            compacted.compacted_makespan_cycles,
            asap.asap_makespan_cycles
        );
    }

    #[test]
    fn compaction_deterministic((n, edges) in er_strategy()) {
        let graph = erdos_renyi_commutation_graph(n, &edges).expect("valid ER graph");
        let req = schedule_from_graph(graph).expect("stub");
        let scheduled = schedule_entangling_layers(req, 340).expect("schedule");
        if scheduled.request.layers.is_empty() {
            return Ok(());
        }
        let deps = infer_atom_dependencies(&scheduled.request.layers);
        let opts = CompactionOptions {
            greedy: true,
            ..Default::default()
        };
        let a = compact_schedule(scheduled.request.clone(), &deps, &opts).expect("a");
        let b = compact_schedule(scheduled.request, &deps, &opts).expect("b");
        prop_assert_eq!(a.request.layers, b.request.layers);
        prop_assert_eq!(a.compacted_makespan_cycles, b.compacted_makespan_cycles);
    }

    #[test]
    fn cubic_chain_like_makespan_stable(n in (4u32..=16).prop_filter("even", |n| n % 2 == 0)) {
        let graph = cubic_commutation_graph(n).expect("cubic");
        let req = schedule_from_graph(graph).expect("stub");
        let scheduled = schedule_entangling_layers(req, 1).expect("schedule");
        let deps = infer_atom_dependencies(&scheduled.request.layers);
        let asap = asap_schedule_layers(scheduled.request.clone(), &deps).expect("asap");
        let opts = CompactionOptions {
            greedy: true,
            ..Default::default()
        };
        let compacted = compact_schedule(scheduled.request, &deps, &opts).expect("compact");
        // Capacity-1 coloring serializes pairs that share vertices; greedy cannot
        // beat exclusive-cycle when AtomHazard chains force total order locally.
        prop_assert!(compacted.compacted_makespan_cycles <= asap.asap_makespan_cycles);
        for layer in &compacted.request.layers {
            layer.validate_conflicts().expect("ok");
        }
    }
}
