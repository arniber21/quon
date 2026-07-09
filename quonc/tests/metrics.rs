//! End-to-end metrics JSON tests (issue #48).

use std::path::PathBuf;
use std::process::Command;

use quon_core::CircuitMetrics;
use serde::Deserialize;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

#[derive(Debug, Deserialize)]
struct MetricsOnly {
    metrics: CircuitMetrics,
}

#[test]
fn bell_state_metrics_match_golden() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let golden_path = workspace_path("tests/fixtures/metrics/bell_state_golden.json");
    let golden_text = std::fs::read_to_string(&golden_path).expect("read golden");
    let golden: MetricsOnly = serde_json::from_str(&golden_text).expect("parse golden");

    let output = quonc()
        .arg("--metrics-json")
        .arg("-")
        .arg(&source)
        .output()
        .expect("spawn quonc");

    assert!(
        output.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    let snapshot: MetricsOnly = serde_json::from_str(&stdout).expect("parse metrics json");
    assert_eq!(snapshot.metrics, golden.metrics);
}

#[test]
fn metrics_human_line_on_stderr() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let output = quonc()
        .arg("--metrics")
        .arg(&source)
        .output()
        .expect("spawn quonc");

    assert!(output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("[quonc] depth=2 gates=2 t=0 swaps=0"));
    assert!(stderr.contains("target=generic_openqasm"));
}

#[test]
fn restrictive_target_compiles_with_metrics() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let target = workspace_path("../backend/tests/fixtures/device_5q.json");
    let output = quonc()
        .arg("--metrics-json")
        .arg("-")
        .arg("--target")
        .arg(&target)
        .arg(&source)
        .output()
        .expect("spawn quonc");

    assert!(
        output.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let snapshot: MetricsOnly = serde_json::from_str(&stdout).expect("parse metrics json");
    assert!(snapshot.metrics.gate_count > 0);
    assert!(snapshot.metrics.qubit_count >= 2);
}
