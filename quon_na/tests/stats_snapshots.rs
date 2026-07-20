//! Insta regression snapshots + shape assertions for `NaStats`
//! (`--emit-na-stats`, issue #307).
//!
//! Wall-clock timings are not reproducible across machines, so this file
//! follows the guidance from the issue: assert timing fields are *present*
//! (Option-typed fields `Some`/`None` per backend) and non-negative (trivial
//! for `u64`, but the presence check is the real content), then snapshot a
//! *redacted* copy of the JSON where every `stage_timings_us` value is
//! replaced with a placeholder. That pins field presence / naming (the
//! "shape") without pinning volatile numbers. The deterministic
//! `config` / `search` sections snapshot their real values.
//!
//! Update goldens after an intentional shape change:
//! ```bash
//! INSTA_UPDATE=1 cargo test -p quon_na --test stats_snapshots --no-default-features
//! ```

use std::path::PathBuf;

use quon_na::{
    NaBackendKind, NaScheduleOptions, NaStats, PlacementStrategy, PlacerMode,
    cubic_commutation_graph, na_stats_to_json, run_from_graph,
};
use serde_json::Value;

fn na_target() -> backend::NeutralAtomTarget {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../targets/neutral_atom/generic_rna_v0.json");
    backend::json::load(&path)
        .expect("load NA target")
        .neutral_atom_target()
        .expect("neutral atom")
        .clone()
}

/// Replace every `stage_timings_us` leaf value with a fixed placeholder so
/// the snapshot pins field presence/shape, not machine-dependent wall times.
fn redact_timings(json: &str) -> String {
    let mut value: Value = serde_json::from_str(json).expect("parse NaStats JSON");
    if let Some(obj) = value
        .get_mut("stage_timings_us")
        .and_then(Value::as_object_mut)
    {
        for v in obj.values_mut() {
            *v = Value::String("<redacted_us>".to_string());
        }
    }
    serde_json::to_string_pretty(&value).expect("reserialize redacted NaStats JSON")
}

/// Assert every `u64` (always non-negative) timing field is well-formed and
/// present exactly where the backend/config implies it should be, then
/// snapshot the redacted shape.
fn assert_and_snapshot(
    name: &str,
    stats: &NaStats,
    expect_zoned_schedule: bool,
    expect_placement_movement: bool,
    expect_compaction: bool,
) {
    let timings = &stats.stage_timings_us;
    // Raw-graph entry point (`run_from_graph`, not `run_from_module`): no
    // MLIR extraction happened, so this is always absent here.
    assert!(
        timings.extract_us.is_none(),
        "run_from_graph must not set extract_us"
    );
    assert_eq!(timings.zoned_schedule_us.is_some(), expect_zoned_schedule);
    assert_eq!(timings.placement_us.is_some(), expect_placement_movement);
    assert_eq!(timings.movement_us.is_some(), expect_placement_movement);
    assert_eq!(timings.compaction_us.is_some(), expect_compaction);
    // total_us covers at least the two backend-agnostic stages that always
    // run; every summand is a u64 so this is also an implicit non-negativity
    // check on the individual stage fields.
    assert!(timings.total_us >= timings.schedule_from_graph_us + timings.entangling_layers_us);

    assert_eq!(stats.kind, quon_na::NA_STATS_KIND);
    assert_eq!(stats.schema_version, quon_na::NA_STATS_SCHEMA_VERSION);
    assert!(!stats.version.quon_na_version.is_empty());
    // Built via run_from_graph directly (not quonc), so no target id overlay.
    assert!(stats.version.target_id.is_none());

    let json = na_stats_to_json(stats).expect("serialize NaStats");
    insta::assert_snapshot!(name, redact_timings(&json));
}

#[test]
fn zoned_routing_agnostic_stats_shape() {
    let na = na_target();
    let cubic = cubic_commutation_graph(4).expect("cubic");
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
    let stats = artifacts
        .stats
        .expect("run_from_graph always populates stats");

    assert_eq!(stats.config.backend, NaBackendKind::Zoned);
    assert_eq!(stats.config.placer_mode, Some(PlacerMode::RoutingAgnostic));
    assert_eq!(stats.config.placement_strategy, None);
    assert_eq!(stats.search.aware_search_completed_layers, Some(0));
    assert_eq!(stats.search.aware_search_node_expansions, Some(0));
    assert_eq!(
        stats.search.aware_search_node_budget,
        Some(quon_na::AWARE_NODE_BUDGET as u64)
    );

    assert_and_snapshot("zoned_routing_agnostic", &stats, true, false, true);
}

#[test]
fn zoned_routing_aware_stats_shape() {
    let na = na_target();
    let cubic = cubic_commutation_graph(4).expect("cubic");
    let artifacts = run_from_graph(
        cubic,
        &na,
        NaScheduleOptions {
            backend: NaBackendKind::Zoned,
            placer: PlacerMode::RoutingAware,
            compact: true,
            placement: PlacementStrategy::RowMajor,
            dump_ir: false,
        },
        None,
    )
    .expect("schedule");
    let stats = artifacts
        .stats
        .expect("run_from_graph always populates stats");

    assert_eq!(stats.config.backend, NaBackendKind::Zoned);
    assert_eq!(stats.config.placer_mode, Some(PlacerMode::RoutingAware));
    // Routing-aware search should actually run and expand at least one node
    // on a small cubic commutation graph (non-trivial layers).
    let completed = stats
        .search
        .aware_search_completed_layers
        .expect("zoned backend always sets this");
    let budget_exceeded = stats.search.aware_search_budget_exceeded_layers.unwrap();
    let no_legal = stats
        .search
        .aware_search_no_legal_assignment_layers
        .unwrap();
    assert!(completed + budget_exceeded + no_legal > 0);
    assert!(stats.search.aware_search_node_expansions.unwrap() > 0);

    assert_and_snapshot("zoned_routing_aware_config_only", &stats, true, false, true);
}

#[test]
fn flat_aod_stats_shape() {
    let na = na_target();
    let cubic = cubic_commutation_graph(4).expect("cubic");
    let artifacts = run_from_graph(
        cubic,
        &na,
        NaScheduleOptions {
            backend: NaBackendKind::FlatAod,
            placer: PlacerMode::RoutingAgnostic,
            compact: true,
            placement: PlacementStrategy::RowMajor,
            dump_ir: false,
        },
        None,
    )
    .expect("schedule");
    let stats = artifacts
        .stats
        .expect("run_from_graph always populates stats");

    assert_eq!(stats.config.backend, NaBackendKind::FlatAod);
    assert_eq!(stats.config.placer_mode, None);
    assert_eq!(
        stats.config.placement_strategy,
        Some(PlacementStrategy::RowMajor)
    );
    // Aware-search concept doesn't apply to flat AOD.
    assert_eq!(stats.search.aware_search_completed_layers, None);
    assert_eq!(stats.search.aware_search_node_expansions, None);
    assert_eq!(stats.search.aware_search_node_budget, None);

    assert_and_snapshot("flat_aod", &stats, false, true, true);
}

#[test]
fn no_compact_flag_reflects_requested_not_applied() {
    let na = na_target();
    let cubic = cubic_commutation_graph(4).expect("cubic");
    let artifacts = run_from_graph(
        cubic,
        &na,
        NaScheduleOptions {
            backend: NaBackendKind::Zoned,
            placer: PlacerMode::RoutingAgnostic,
            compact: false,
            placement: PlacementStrategy::RowMajor,
            dump_ir: false,
        },
        None,
    )
    .expect("schedule");
    let stats = artifacts
        .stats
        .expect("run_from_graph always populates stats");

    assert!(!stats.config.compaction.requested);
    assert!(!stats.config.compaction.applied);
    assert!(stats.stage_timings_us.compaction_us.is_none());
}

/// A stats JSON with only the fields present in schema v1 must still
/// deserialize (the `#[serde(default)]` field-evolution policy, issue #307).
#[test]
fn na_stats_deserializes_minimal_json() {
    let minimal = r#"{}"#;
    let stats: NaStats = serde_json::from_str(minimal).expect("minimal NaStats must deserialize");
    assert_eq!(stats.kind, quon_na::NA_STATS_KIND);
    assert_eq!(stats.schema_version, quon_na::NA_STATS_SCHEMA_VERSION);
    assert_eq!(stats.config.backend, NaBackendKind::Zoned);
    assert_eq!(stats.stage_timings_us.total_us, 0);
    assert!(stats.search.aware_search_completed_layers.is_none());
}
