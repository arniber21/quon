//! End-to-end `--emit-na-stats` tests (issue #307).
//!
//! Wall-clock timing values are machine-dependent, so these assert JSON
//! shape / field presence / config echo — never exact timing numbers.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn na_target() -> PathBuf {
    workspace_path("../targets/neutral_atom/generic_rna_v0.json")
}

fn run_emit_na_stats(source: &PathBuf, extra_args: &[&str]) -> (std::process::Output, Value) {
    let output = quonc()
        .arg(source)
        .arg("--target")
        .arg(na_target())
        .args(extra_args)
        .arg("--emit-na-stats")
        .arg("-")
        .arg("--quiet")
        .output()
        .expect("spawn quonc");
    let json = if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        serde_json::from_str(&stdout)
            .unwrap_or_else(|e| panic!("parse na-stats JSON: {e}\n{stdout}"))
    } else {
        Value::Null
    };
    (output, json)
}

/// Zoned (default) backend: `zoned_schedule_us` present, `placement_us` /
/// `movement_us` absent, search diagnostics populated (`Some(0)` under the
/// default routing-agnostic placer).
#[test]
fn zoned_backend_emits_full_shape() {
    let source = workspace_path("../test/na/bell.qn");
    let (output, stats) = run_emit_na_stats(&source, &[]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(stats["kind"], "na_compiler_stats");
    assert_eq!(stats["schema_version"], 1);
    assert_eq!(
        stats["version"]["target_id"], "generic_reconfigurable_neutral_atom_v0",
        "quonc must overlay the loaded target id: {stats}"
    );
    assert!(
        stats["version"]["quon_na_version"].is_string(),
        "missing quon_na_version: {stats}"
    );

    assert_eq!(stats["config"]["backend"], "zoned");
    assert_eq!(stats["config"]["placer_mode"], "routing_agnostic");
    assert!(
        stats["config"].get("placement_strategy").is_none(),
        "zoned backend must not echo a flat-AOD placement_strategy: {stats}"
    );
    assert_eq!(stats["config"]["compaction"]["requested"], true);
    assert_eq!(stats["config"]["compaction"]["applied"], true);
    assert_eq!(
        stats["config"]["objective"], "time",
        "default objective: {stats}"
    );

    let timings = &stats["stage_timings_us"];
    assert!(
        timings.get("extract_us").is_some(),
        "quonc always extracts from MLIR: {stats}"
    );
    assert!(timings["schedule_from_graph_us"].is_u64());
    assert!(timings["entangling_layers_us"].is_u64());
    assert!(
        timings.get("zoned_schedule_us").is_some(),
        "zoned backend must time schedule_zoned: {stats}"
    );
    assert!(
        timings.get("placement_us").is_none() && timings.get("movement_us").is_none(),
        "zoned backend must not have flat-AOD stage timings: {stats}"
    );
    assert!(timings["total_us"].is_u64());

    let search = &stats["search"];
    assert_eq!(search["aware_search_completed_layers"], 0);
    assert_eq!(search["aware_search_node_expansions"], 0);
    assert_eq!(search["aware_search_node_budget"], 100_000);
}

/// Flat-AOD backend: `placement_us` / `movement_us` present, `zoned_schedule_us`
/// absent, search diagnostics entirely absent (concept doesn't apply).
#[test]
fn flat_aod_backend_emits_full_shape() {
    let source = workspace_path("../test/na/bell.qn");
    let (output, stats) = run_emit_na_stats(&source, &["--na-backend", "flat"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    assert_eq!(stats["config"]["backend"], "flat_aod");
    assert_eq!(stats["config"]["placement_strategy"], "row_major");
    assert!(
        stats["config"].get("placer_mode").is_none(),
        "flat backend must not echo a zoned placer_mode: {stats}"
    );

    let timings = &stats["stage_timings_us"];
    assert!(timings.get("zoned_schedule_us").is_none(), "{stats}");
    assert!(timings.get("placement_us").is_some(), "{stats}");
    assert!(timings.get("movement_us").is_some(), "{stats}");

    let search = &stats["search"];
    assert!(
        search.as_object().is_some_and(serde_json::Map::is_empty),
        "flat backend must omit every aware-search field: {stats}"
    );
}

/// Routing-aware placer surfaces non-trivial search diagnostics.
#[test]
fn routing_aware_placer_populates_search_diagnostics() {
    let source = workspace_path("../test/na/qft_small.qn");
    let (output, stats) = run_emit_na_stats(&source, &["--na-placer", "routing-aware"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stats["config"]["placer_mode"], "routing_aware");
    let search = &stats["search"];
    let completed = search["aware_search_completed_layers"].as_u64().unwrap();
    let budget_exceeded = search["aware_search_budget_exceeded_layers"]
        .as_u64()
        .unwrap();
    let no_legal = search["aware_search_no_legal_assignment_layers"]
        .as_u64()
        .unwrap();
    assert!(
        completed + budget_exceeded + no_legal > 0,
        "routing-aware placer on a non-trivial circuit should run the search at least once: {stats}"
    );
    assert!(
        search["aware_search_node_expansions"].as_u64().unwrap() > 0,
        "{stats}"
    );
}

/// `--no-na-compact` must be echoed as requested-but-not-applied, with
/// `compaction_us` absent.
#[test]
fn no_na_compact_reflected_in_config_echo() {
    let source = workspace_path("../test/na/bell.qn");
    let (output, stats) = run_emit_na_stats(&source, &["--no-na-compact"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(stats["config"]["compaction"]["requested"], false);
    assert_eq!(stats["config"]["compaction"]["applied"], false);
    assert!(
        stats["stage_timings_us"].get("compaction_us").is_none(),
        "{stats}"
    );
}

/// `--emit-na-stats` requires a neutral-atom target, matching every other
/// NA-only emit flag.
#[test]
fn requires_neutral_atom_target() {
    let source = workspace_path("../test/na/bell.qn");
    let output = quonc()
        .arg(&source)
        .arg("--emit-na-stats")
        .arg("-")
        .output()
        .expect("spawn quonc");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--emit-na-stats"),
        "error should name the flag: {stderr}"
    );
}

/// QEC-backed programs now populate NaStats via the shared `plan_backend`
/// stage (#317), so `--emit-na-stats` succeeds and emits valid stats JSON.
#[test]
fn qec_backed_program_emits_stats() {
    let source = workspace_path("../examples/na_qec/repetition_d3_memory.qn");
    let (output, stats) = run_emit_na_stats(&source, &[]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Stage timings are populated — the QEC path now instruments per-phase
    // entangle + plan_backend timings (#317 / #307).
    assert_eq!(stats["kind"], "na_compiler_stats");
    assert!(
        stats["stage_timings_us"]["total_us"].as_u64().unwrap_or(0) > 0,
        "total_us must be > 0 for QEC-backed compile"
    );
    // Zoned backend (default) should have zoned_schedule_us populated.
    assert!(
        stats["stage_timings_us"]["zoned_schedule_us"].is_number(),
        "zoned_schedule_us must be present for zoned backend"
    );
    assert_eq!(stats["config"]["backend"], "zoned");
}

/// When both `--emit-na-schedule -` and `--emit-na-stats -` target stdout,
/// the schedule (primary debug artifact registered first) keeps stdout and
/// stats is redirected to stderr — same convention as every other `-`-target
/// emit pair in `quonc`.
#[test]
fn stdout_arbitration_with_schedule() {
    let source = workspace_path("../test/na/bell.qn");
    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-na-schedule")
        .arg("-")
        .arg("--emit-na-stats")
        .arg("-")
        .arg("--quiet")
        .output()
        .expect("spawn quonc");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stdout.contains("\"kind\": \"na_schedule_view\""),
        "schedule should own stdout: {stdout}"
    );
    assert!(
        stderr.contains("\"kind\": \"na_compiler_stats\""),
        "stats should fall back to stderr: {stderr}"
    );
}
