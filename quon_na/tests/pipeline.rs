//! Smoke tests: NA schedule pipeline is callable without `quonc` (#210).

use std::path::PathBuf;

use quon_na::{
    NaBackendKind, NaScheduleOptions, PlacementStrategy, PlacerMode, cubic_commutation_graph,
    run_from_graph,
};

fn na_target() -> backend::NeutralAtomTarget {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../targets/neutral_atom/generic_rna_v0.json");
    backend::json::load(&path)
        .expect("load NA target")
        .neutral_atom_target()
        .expect("neutral atom")
        .clone()
}

#[test]
fn run_from_graph_schedules_cubic_without_quonc() {
    let na = na_target();
    let cubic = cubic_commutation_graph(12).expect("cubic");
    let artifacts = run_from_graph(
        cubic,
        &na,
        NaScheduleOptions {
            backend: NaBackendKind::Zoned,
            placer: PlacerMode::RoutingAgnostic,
            compact: true,
            placement: PlacementStrategy::RowMajor,
            dump_ir: false,
        },
        None,
    )
    .expect("schedule");
    assert!(!artifacts.layers.is_empty());
    assert!(artifacts.resource_report.estimated_cycles > 0);
    assert!(artifacts.logical_qubits > 0);
}
