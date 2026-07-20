//! End-to-end neutral-atom pipeline tests for issue #112.

use std::path::PathBuf;
use std::process::Command;

use quon_na::{
    PlacementStrategy, PlacerMode, cubic_commutation_graph, erdos_renyi_commutation_graph,
};
use quonc::compile::schedule_raw_graph;
use quonc::na_target::NaBackendKind;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn na_target() -> PathBuf {
    workspace_path("../targets/neutral_atom/generic_rna_v0.json")
}

/// Same shape as `na_target()` but `native_gates` lacks `ry` (and `rx`), so no
/// non-diagonal single-qubit gate has a native rz/ry/rz-basis realization —
/// forces the `u3(theta, phi, lambda)` escape hatch (issue #298).
fn na_target_no_ry() -> PathBuf {
    workspace_path("tests/fixtures/na_target_no_ry.json")
}

#[test]
fn bell_emits_na_schedule_and_resource_report() {
    let source = workspace_path("../test/na/bell.qn");
    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-na-schedule")
        .arg("-")
        .arg("--emit-resource-report")
        .arg("-")
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    // Schedule JSON envelope on stdout; resource report on stderr when both use `-`.
    assert!(
        stdout.contains("\"kind\": \"na_schedule_view\""),
        "missing schedule envelope: {stdout}"
    );
    assert!(
        stdout.contains("\"schema_version\": 1"),
        "missing schema_version: {stdout}"
    );
    assert!(stdout.contains("\"cycle\""), "missing schedule: {stdout}");
    assert!(
        stdout.contains("\"Entangle2\""),
        "missing Entangle2 in schedule: {stdout}"
    );
    assert!(
        stdout.contains("\"zones\""),
        "missing zones in schedule envelope: {stdout}"
    );
    assert!(
        stderr.contains("\"estimated_cycles\""),
        "missing report on stderr: {stderr}"
    );
    assert!(
        stderr.contains("\"entangle2_count\""),
        "missing entangle2_count: {stderr}"
    );
    // Extract must not double-count leftover quantum.circ.func callees.
    assert!(
        stderr.contains("\"logical_qubits\": 2"),
        "expected 2 logical qubits, got: {stderr}"
    );
    assert!(
        stderr.contains("\"entangle2_count\": 1"),
        "expected one Entangle2, got: {stderr}"
    );
    let report: serde_json::Value =
        serde_json::from_str(&stderr).expect("resource report on stderr is JSON");
    let budget = report
        .get("error_budget")
        .expect("resource report must include error_budget when target has error_model");
    let stages = report["rydberg_stages"].as_u64().expect("rydberg_stages") as f64;
    let rydberg = budget["rydberg"].as_f64().expect("error_budget.rydberg");
    // generic_rna_v0.json error_model.rydberg = 0.002
    assert!(
        (rydberg - 0.002 * stages).abs() < 1e-12,
        "rydberg contribution {rydberg} != 0.002 × {stages}"
    );
}

/// ADR-0017: `--emit-resource-report` on NeutralAtom hard-fails without `error_model`.
#[test]
fn emit_resource_report_fails_without_error_model() {
    let source = workspace_path("../test/na/bell.qn");
    let mut target: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(na_target()).expect("read target"))
            .expect("parse target");
    target
        .as_object_mut()
        .expect("object")
        .remove("error_model");
    let path = std::env::temp_dir().join(format!(
        "quon-na-no-error-model-{}.json",
        std::process::id()
    ));
    std::fs::write(&path, target.to_string()).expect("write temp target");

    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(&path)
        .arg("--emit-resource-report")
        .arg("-")
        .arg("--quiet")
        .output()
        .expect("spawn");
    let _ = std::fs::remove_file(&path);

    assert!(
        !output.status.success(),
        "expected failure without error_model; stdout={}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("missing error_model"),
        "stderr should be MissingErrorModel diagnostic: {stderr}"
    );
    assert!(
        stderr.contains("--emit-resource-report"),
        "stderr should name the emit surface: {stderr}"
    );
}

#[test]
fn bell_emits_interaction_graph_dot() {
    let source = workspace_path("../test/na/bell.qn");
    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-na-graph")
        .arg("-")
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("graph InteractionGraph"),
        "missing DOT header: {stdout}"
    );
    assert!(stdout.contains("q0"), "missing qubit node: {stdout}");
    assert!(stdout.contains("--"), "missing edge: {stdout}");
}

#[test]
fn resource_report_markdown_via_extension() {
    let source = workspace_path("../test/na/bell.qn");
    let out_path = std::env::temp_dir().join(format!("quon-na-report-{}.md", std::process::id()));
    let _ = std::fs::remove_file(&out_path);
    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-resource-report")
        .arg(&out_path)
        .arg("--quiet")
        .output()
        .expect("spawn");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let md = std::fs::read_to_string(&out_path).expect("read md");
    assert!(md.contains("# Neutral-atom analytic resource report"));
    assert!(md.contains("Estimated cycles"));
    assert!(
        md.contains("## Physical error budget"),
        "markdown report must include error budget when target has error_model"
    );
    assert!(md.contains("Contribution (rate × count)"));
    let _ = std::fs::remove_file(&out_path);
}

#[test]
fn na_fixtures_produce_nonempty_schedules() {
    for name in [
        "bell.qn",
        "qft_small.qn",
        "qaoa_graph.qn",
        "ising.qn",
        "syndrome_round_toy.qn",
        "rotations.qn",
    ] {
        let source = workspace_path(&format!("../test/na/{name}"));
        let output = quonc()
            .arg(&source)
            .arg("--target")
            .arg(na_target())
            .arg("--emit-resource-report")
            .arg("-")
            .arg("--quiet")
            .output()
            .expect("spawn");
        assert!(
            output.status.success(),
            "{name} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("\"estimated_cycles\""),
            "{name}: missing cycles in {stdout}"
        );
        // Parse estimated_cycles > 0
        let cycles = stdout
            .lines()
            .find(|l| l.contains("estimated_cycles"))
            .and_then(|l| {
                l.split(':')
                    .nth(1)
                    .map(|s| s.trim().trim_end_matches(',').parse::<u64>().ok())
            })
            .flatten()
            .unwrap_or(0);
        assert!(cycles > 0, "{name}: estimated_cycles was 0");
    }
}

#[test]
fn openqasm_path_unaffected_by_na_wiring() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let output = quonc()
        .arg("--emit-qasm")
        .arg(&source)
        .output()
        .expect("spawn");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let qasm = String::from_utf8_lossy(&output.stdout);
    assert!(qasm.contains("OPENQASM 3.0"));
    assert!(qasm.contains("cx q[0], q[1]"));
}

#[test]
fn raw_graph_stress_cubic_and_erdos_renyi() {
    let target = backend::json::load(&na_target()).expect("load NA target");
    let na = target
        .neutral_atom_target()
        .expect("neutral atom target")
        .clone();

    // 3-regular (cubic) graph — Enola/OLSQ-DPQA style; Δ=3 ⇒ ≤4 MG layers.
    let cubic = cubic_commutation_graph(20).expect("cubic");
    let (req, report) = schedule_raw_graph(
        cubic,
        &na,
        NaBackendKind::Zoned,
        PlacerMode::RoutingAgnostic,
        true,
        PlacementStrategy::RowMajor,
    )
    .expect("cubic schedule");
    assert!(!req.layers.is_empty());
    assert!(report.estimated_cycles > 0);
    assert!(report.entangle2_count > 0);

    // Larger ER-style random graph stress (debug entry point).
    let mut edges = Vec::new();
    let n = 40u32;
    let mut state = 0xC0FFEE_u64;
    for a in 0..n {
        for b in (a + 1)..n {
            state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
            if (state >> 33) % 100 < 8 {
                edges.push((a, b));
            }
        }
    }
    let er = erdos_renyi_commutation_graph(n, &edges).expect("er");
    let (req, report) = schedule_raw_graph(
        er,
        &na,
        NaBackendKind::Zoned,
        PlacerMode::RoutingAgnostic,
        true,
        PlacementStrategy::RowMajor,
    )
    .expect("er schedule");
    assert!(!req.layers.is_empty());
    assert!(report.estimated_cycles > 0);
}

// --- Issue #298: local rz / global ry / u3 end to end ---

#[test]
fn rotations_qn_preserves_local_rz_and_global_ry_end_to_end() {
    let source = workspace_path("../test/na/rotations.qn");
    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-na-schedule")
        .arg("-")
        .arg("--emit-resource-report")
        .arg("-")
        .arg("--verify-na")
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Schedule preserves both entangling gates (CNOT @(0,1), CNOT @(1,2))...
    assert!(
        stdout.contains("\"Entangle2\""),
        "missing Entangle2 in schedule: {stdout}"
    );
    // ...and every 1-qubit gate: `H @0` and `Ry(...) @2` decompose through a
    // global `ry` raster (`H` also contributes a local `rz`); `Rz(...) @1`
    // and `Rz(...) @0` are local `rz`s.
    assert!(
        stdout.contains("\"LocalGate\""),
        "missing LocalGate in schedule: {stdout}"
    );
    assert!(
        stdout.contains("\"GlobalRy\""),
        "missing GlobalRy in schedule: {stdout}"
    );
    assert!(
        stdout.contains("\"rz\""),
        "missing rz gate kind in schedule: {stdout}"
    );

    // Resource report gains #298 single-qubit gate counts/time, consistent
    // with the circuit: `H`'s zero-angle Euler component is dropped by the
    // existing (unmodified) `backend::decompose` ZYZ math, so `H` (1) +
    // `Rz(..) @1` (1) + `Rz(..) @0` (1) = 3 local rz`s; `H` (1) +
    // `Ry(..) @2` (1) = 2 global ry rasters; no u3 escape needed on this
    // target (native_gates includes `ry`).
    assert!(
        stderr.contains("\"local_rz_count\": 3"),
        "expected 3 local rz gates, got: {stderr}"
    );
    assert!(
        stderr.contains("\"global_ry_count\": 2"),
        "expected 2 global ry rasters, got: {stderr}"
    );
    assert!(
        stderr.contains("\"local_u3_count\": 0"),
        "expected no u3 escapes on a target with native ry, got: {stderr}"
    );
}

#[test]
fn bell_on_target_without_native_ry_falls_back_to_u3_with_warning() {
    // `bell.qn` applies `H @0` — non-diagonal, so with no `ry` (or `rx`+`sx`)
    // in the target's native set, `backend::decompose`'s ZYZ math has no
    // realizable native form; the NA-path decomposer must escape to
    // `u3(theta, phi, lambda)` with a warning (mirrors qmap's
    // `warnUnsupportedGates`), not silently drop the gate or hard-fail.
    let source = workspace_path("../test/na/bell.qn");
    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target_no_ry())
        .arg("--emit-na-schedule")
        .arg("-")
        .arg("--emit-resource-report")
        .arg("-")
        .arg("--verify-na")
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert!(
        stdout.contains("\"u3\""),
        "missing u3 escape-hatch gate in schedule: {stdout}"
    );
    assert!(
        stderr.contains("\"local_u3_count\": 1"),
        "expected one u3 escape, got: {stderr}"
    );
    assert!(
        stderr.contains("native-gate-decomp") && stderr.contains("u3"),
        "expected an unsupported-gate warning on stderr, got: {stderr}"
    );
}
