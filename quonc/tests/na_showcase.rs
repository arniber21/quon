//! Regression tests pinning the headline schedule/report numbers cited in
//! `samples/neutral-atom/README.md` (issue #192 review). These exist so the
//! README's numbers can't silently drift out from under the prose without a
//! test failing here first.
//!
//! Also covers the review's T2 requirement: the `qft_small.qn` flat-backend
//! failure story must stay pinned to a real non-zero exit and a specific
//! error fragment, not just quoted prose that could drift.

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

fn temp_json_path(label: &str) -> PathBuf {
    // Tests in this binary run as parallel threads of one process, and several call
    // `emit_na_schedule` concurrently — the pid alone is shared, and the delete-after-use
    // below would race another thread's write/read. A per-call counter keeps paths unique.
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    std::env::temp_dir().join(format!(
        "quon-na-showcase-192-{label}-{}-{}.json",
        std::process::id(),
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    ))
}

fn emit_na_schedule(source: &PathBuf, extra_args: &[&str]) -> (std::process::Output, Value) {
    let report_path = temp_json_path("schedule");
    let _ = std::fs::remove_file(&report_path);

    let output = quonc()
        .arg(source)
        .arg("--target")
        .arg(na_target())
        .args(extra_args)
        .arg("--emit-na-schedule")
        .arg(&report_path)
        .arg("--quiet")
        .output()
        .expect("spawn quonc");

    let json = if output.status.success() {
        let text = std::fs::read_to_string(&report_path)
            .unwrap_or_else(|e| panic!("read schedule {}: {e}", report_path.display()));
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse schedule JSON: {e}\n{text}"))
    } else {
        Value::Null
    };
    let _ = std::fs::remove_file(&report_path);
    (output, json)
}

/// Headline #1 (`--na-backend zoned|flat` on `bell.qn`): both backends
/// succeed, and their schedule metrics must match the numbers quoted in
/// `samples/neutral-atom/README.md`'s "1. `--na-backend zoned|flat`" table.
#[test]
fn bell_backend_headline_numbers_match_readme() {
    let source = workspace_path("../test/na/bell.qn");

    let (zoned_out, zoned) = emit_na_schedule(&source, &["--na-backend", "zoned"]);
    assert!(
        zoned_out.status.success(),
        "zoned bell.qn should succeed: {}",
        String::from_utf8_lossy(&zoned_out.stderr)
    );
    let m = &zoned["metrics"];
    // `estimated_cycles` grew 4 -> 6 under issue #298: `H @0` used to be
    // silently dropped during extraction (the gap #298 closes); it now
    // contributes 2 extra schedule layers (a local `rz` + a global `ry`
    // raster from its Z-Y-Z decomposition). Movement-side numbers
    // (rearrangement/transfers/bottleneck) are unaffected — `H` has no site
    // placement requirement.
    assert_eq!(m["estimated_cycles"], 6, "zoned bell.qn: {zoned}");
    assert_eq!(m["rearrangement_steps"], 1, "zoned bell.qn: {zoned}");
    assert_eq!(m["trap_transfers"], 4, "zoned bell.qn: {zoned}");
    assert_eq!(m["bottleneck"], "rearrangement", "zoned bell.qn: {zoned}");

    let (flat_out, flat) = emit_na_schedule(&source, &["--na-backend", "flat"]);
    assert!(
        flat_out.status.success(),
        "flat bell.qn should succeed: {}",
        String::from_utf8_lossy(&flat_out.stderr)
    );
    let m = &flat["metrics"];
    // Same #298 story: 1 -> 3 for the same reason as the zoned case above.
    assert_eq!(m["estimated_cycles"], 3, "flat bell.qn: {flat}");
    assert_eq!(m["rearrangement_steps"], 0, "flat bell.qn: {flat}");
    assert_eq!(m["trap_transfers"], 0, "flat bell.qn: {flat}");
    assert_eq!(m["bottleneck"], "rydberg", "flat bell.qn: {flat}");
}

/// T2 (issue #192 review): the flat-backend footnote story on `qft_small.qn`
/// must keep failing non-zero with the expected "fail-closed" fragment, so
/// the README's quoted error text can't silently drift.
#[test]
fn qft_small_flat_backend_fails_closed_with_expected_error() {
    let source = workspace_path("../test/na/qft_small.qn");
    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--na-backend")
        .arg("flat")
        .arg("--emit-na-schedule")
        .arg("-")
        .arg("--quiet")
        .output()
        .expect("spawn quonc");

    assert!(
        !output.status.success(),
        "qft_small.qn on the flat backend must fail closed, not succeed"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("entangling geometry violation"),
        "expected a geometry-violation error, got: {stderr}"
    );
    assert!(
        stderr.contains("fail-closed"),
        "expected the fail-closed framing quoted in the README, got: {stderr}"
    );

    // Sanity: the same program succeeds on the zoned (default) backend —
    // proves the failure above is about the flat backend specifically, not
    // a broken fixture.
    let (zoned_out, _) = emit_na_schedule(&source, &["--na-backend", "zoned"]);
    assert!(
        zoned_out.status.success(),
        "qft_small.qn must still succeed on zoned: {}",
        String::from_utf8_lossy(&zoned_out.stderr)
    );
}

/// Headline #2 (`--na-placer routing-agnostic|routing-aware` on
/// `qaoa_graph.qn` / `ising.qn`): pins the numbers quoted in the README's
/// placer-comparison table, including the N2 finding that `ising.qn`'s two
/// placer modes share identical structural metrics but different
/// `total_time_us` — not a silent fallback to the agnostic planner.
#[test]
fn placer_headline_numbers_match_readme() {
    let qaoa = workspace_path("../test/na/qaoa_graph.qn");
    let ising = workspace_path("../test/na/ising.qn");

    let (_, qaoa_agnostic) = emit_na_schedule(
        &qaoa,
        &["--na-backend", "zoned", "--na-placer", "routing-agnostic"],
    );
    let (_, qaoa_aware) = emit_na_schedule(
        &qaoa,
        &["--na-backend", "zoned", "--na-placer", "routing-aware"],
    );
    let (_, ising_agnostic) = emit_na_schedule(
        &ising,
        &["--na-backend", "zoned", "--na-placer", "routing-agnostic"],
    );
    let (_, ising_aware) = emit_na_schedule(
        &ising,
        &["--na-backend", "zoned", "--na-placer", "routing-aware"],
    );

    // Issue #298: `qaoa_graph.qn` applies `H` (hadamard_all) and `Rx`
    // (mixer_4) to every qubit — both used to be silently dropped during
    // extraction; `ising.qn` applies `Rx` (x_layer) likewise. Both now
    // contribute real schedule layers (`estimated_cycles`/`total_time_us`
    // grow), while rearrangement/transfer counts are unaffected (1-qubit
    // gates have no site placement requirement). `Rx` has no first-class
    // `LocalGateKind` (scope: `h`/`rz`/`ry`/`u3` — see
    // `quon_na::native_gate_decomp`), so it decomposes through the `u3`
    // escape hatch here even though `rx` is nominally in this target's
    // `native_gates`.
    let m = &qaoa_agnostic["metrics"];
    assert_eq!(m["estimated_cycles"], 52, "qaoa agnostic: {qaoa_agnostic}");
    assert_eq!(
        m["rearrangement_steps"], 8,
        "qaoa agnostic: {qaoa_agnostic}"
    );
    assert_eq!(m["trap_transfers"], 22, "qaoa agnostic: {qaoa_agnostic}");
    assert_eq!(m["total_time_us"], 1658, "qaoa agnostic: {qaoa_agnostic}");

    let m = &qaoa_aware["metrics"];
    assert_eq!(m["estimated_cycles"], 55, "qaoa aware: {qaoa_aware}");
    assert_eq!(m["rearrangement_steps"], 9, "qaoa aware: {qaoa_aware}");
    assert_eq!(m["trap_transfers"], 24, "qaoa aware: {qaoa_aware}");
    assert_eq!(m["total_time_us"], 1736, "qaoa aware: {qaoa_aware}");

    let agnostic_m = &ising_agnostic["metrics"];
    let aware_m = &ising_aware["metrics"];
    assert_eq!(
        agnostic_m["estimated_cycles"], 48,
        "ising agnostic: {ising_agnostic}"
    );
    assert_eq!(
        agnostic_m["rearrangement_steps"], 9,
        "ising agnostic: {ising_agnostic}"
    );
    assert_eq!(
        agnostic_m["trap_transfers"], 20,
        "ising agnostic: {ising_agnostic}"
    );
    assert_eq!(
        agnostic_m["total_time_us"], 2217,
        "ising agnostic: {ising_agnostic}"
    );
    assert_eq!(
        aware_m["estimated_cycles"], 48,
        "ising aware: {ising_aware}"
    );
    assert_eq!(
        aware_m["rearrangement_steps"], 9,
        "ising aware: {ising_aware}"
    );
    assert_eq!(aware_m["trap_transfers"], 20, "ising aware: {ising_aware}");
    assert_eq!(aware_m["total_time_us"], 2256, "ising aware: {ising_aware}");

    // N2: the structural metrics above are byte-for-byte identical between
    // the two placer modes on `ising.qn` — confirm that isn't because the
    // "aware" run silently fell back to the agnostic code path, by checking
    // the schedule metadata actually reports `routing_aware`.
    assert_eq!(
        agnostic_m["estimated_cycles"], aware_m["estimated_cycles"],
        "sanity: this test's premise is that structural metrics tie"
    );
    assert_eq!(
        ising_aware["meta"]["na_placer"], "routing_aware",
        "the aware run must report routing_aware in its schedule metadata, \
         proving it took the aware code path rather than silently falling \
         back to agnostic: {ising_aware}"
    );
    assert_ne!(
        agnostic_m["total_time_us"], aware_m["total_time_us"],
        "total_time_us should still differ even when structural metrics tie: {ising_agnostic} vs {ising_aware}"
    );
}

/// Headline #3 (dynamic circuit / mid-circuit measurement): pins the real
/// interleaved measure/entangle numbers the README cites for
/// `examples/na_qec/repetition_d3_memory.qn`, and asserts the schedule
/// genuinely interleaves measurement between entangling layers rather than
/// only measuring at the end — the concrete claim issue #192's review asked
/// this pack to either prove or retract.
#[test]
fn repetition_d3_memory_schedule_is_genuinely_mid_circuit() {
    let source = workspace_path("../examples/na_qec/repetition_d3_memory.qn");
    let (output, schedule) = emit_na_schedule(&source, &["--verify-na"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let m = &schedule["metrics"];
    assert_eq!(m["rydberg_stages"], 6, "{schedule}");
    assert_eq!(m["measurement_rounds"], 3, "{schedule}");
    assert_eq!(m["reset_rounds"], 2, "{schedule}");
    assert_eq!(m["estimated_cycles"], 37, "{schedule}");

    let layers = schedule["layers"]
        .as_array()
        .unwrap_or_else(|| panic!("schedule has no layers array: {schedule}"));
    let mut measure_layer_indices = Vec::new();
    let mut entangle_layer_indices = Vec::new();
    for (i, layer) in layers.iter().enumerate() {
        let actions = layer["actions"].as_array().cloned().unwrap_or_default();
        let has_measure = actions.iter().any(|a| a.get("Measure").is_some());
        let has_entangle = actions
            .iter()
            .any(|a| a.get("Entangle2").is_some() || a.get("EntangleN").is_some());
        if has_measure {
            measure_layer_indices.push(i);
        }
        if has_entangle {
            entangle_layer_indices.push(i);
        }
    }

    assert_eq!(
        measure_layer_indices.len(),
        3,
        "expected 3 layers containing a Measure action: {measure_layer_indices:?}"
    );
    let first_measure = measure_layer_indices[0];
    let last_measure = *measure_layer_indices.last().unwrap();

    // The defining "genuinely mid-circuit" claim: at least one entangling
    // layer must come strictly *after* the first measurement layer (and
    // before the last one) — i.e. the schedule doesn't just measure
    // everything once at the very end.
    let entangle_after_first_measure = entangle_layer_indices
        .iter()
        .any(|&i| i > first_measure && i < last_measure);
    assert!(
        entangle_after_first_measure,
        "expected at least one Entangle2/EntangleN layer strictly between the \
         first and last Measure layers, proving genuine mid-circuit \
         measurement rather than terminal-only measurement: measures at \
         {measure_layer_indices:?}, entangles at {entangle_layer_indices:?}"
    );
}
