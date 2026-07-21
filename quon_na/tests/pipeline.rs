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
            ..Default::default()
        },
        None,
    )
    .expect("schedule");
    assert!(!artifacts.layers.is_empty());
    assert!(artifacts.resource_report.estimated_cycles > 0);
    assert!(artifacts.logical_qubits > 0);
}

/// ADR-0017: `run_from_graph` attaches analytic `error_budget` when the target
/// carries `error_model` (independent of quonc re-attach on emit).
#[test]
fn run_from_graph_attaches_error_budget_when_target_has_model() {
    let na = na_target();
    let model = na
        .error_model
        .as_ref()
        .expect("generic_rna_v0.json must include error_model");
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
            ..Default::default()
        },
        None,
    )
    .expect("schedule");
    let budget = artifacts
        .resource_report
        .error_budget
        .expect("pipeline must attach error_budget when target has error_model");
    let expected_rydberg = model.rydberg * artifacts.resource_report.rydberg_stages as f64;
    assert!(
        (budget.rydberg - expected_rydberg).abs() < 1e-12,
        "rydberg contribution {} != rate × stages {}",
        budget.rydberg,
        expected_rydberg
    );
}
