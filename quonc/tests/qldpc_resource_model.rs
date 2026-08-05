//! End-to-end `--qldpc-graph` / `--qldpc-net-rate` tests (issue #478).
//!
//! The qLDPC resource model skips the compile pipeline, so these tests do not
//! pass a source file or a backend target — they exercise the standalone
//! resource-estimate path directly.

use std::path::PathBuf;

use serde_json::Value;
use std::process::Command;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Graph mode: load the toy [[5,1,3]] parity-check graph and emit JSON to
/// stdout. Asserts the qLDPC-specific estimate fields are present and correct.
#[test]
fn graph_mode_emits_estimate_json() {
    let graph = workspace_path("../examples/na_qec/qldpc_5qubit.json");
    let out = quonc()
        .arg("--qldpc-graph")
        .arg(&graph)
        .arg("--emit-qldpc-report")
        .arg("-")
        .output()
        .expect("run quonc");
    assert!(
        out.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["evidence_kind"], "analytic");
    assert_eq!(v["mode"], "parity_check_graph");
    let est = &v["estimate"];
    assert_eq!(est["n_data"], 5);
    assert_eq!(est["n_checks"], 4);
    assert_eq!(est["distance"], 3);
    assert_eq!(est["max_check_weight"], 5);
    assert_eq!(est["peak_atoms"], 9);
    assert_eq!(est["edge_count"], 20);
    assert_eq!(est["measurement_rounds"], 1);
}

/// Graph mode with `--qldpc-rounds 3` scales the measurement-rounds field.
#[test]
fn graph_mode_rounds_scales_measurement_rounds() {
    let graph = workspace_path("../examples/na_qec/qldpc_5qubit.json");
    let out = quonc()
        .arg("--qldpc-graph")
        .arg(&graph)
        .arg("--qldpc-rounds")
        .arg("3")
        .arg("--emit-qldpc-report")
        .arg("-")
        .output()
        .expect("run quonc");
    assert!(
        out.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["estimate"]["measurement_rounds"], 3);
}

/// Net-rate sizing mode: `1/24` with 12 logical qubits mirrors the
/// [[144,12,12]]-style architecture-model example (288 physical atoms).
#[test]
fn net_rate_mode_emits_sizing_json() {
    let out = quonc()
        .arg("--qldpc-net-rate")
        .arg("1/24")
        .arg("--qldpc-logical-qubits")
        .arg("12")
        .arg("--emit-qldpc-report")
        .arg("-")
        .output()
        .expect("run quonc");
    assert!(
        out.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Value = serde_json::from_slice(&out.stdout).expect("valid JSON");
    assert_eq!(v["mode"], "net_rate_sizing");
    assert_eq!(v["code_family"], "high_rate_qldpc_like");
    assert_eq!(v["net_rate"]["numerator"], 1);
    assert_eq!(v["net_rate"]["denominator"], 24);
    assert_eq!(v["atoms_per_logical"], 24);
    assert_eq!(v["logical_qubits"], 12);
    assert_eq!(v["physical_atoms"], 288);
}

/// Markdown output: `.md` path extension switches to the Markdown format.
#[test]
fn net_rate_mode_markdown_output() {
    let tmp = tempfile::NamedTempFile::with_suffix(".md").expect("temp file");
    let path = tmp.path();
    let out = quonc()
        .arg("--qldpc-net-rate")
        .arg("1/24")
        .arg("--emit-qldpc-report")
        .arg(path)
        .output()
        .expect("run quonc");
    assert!(
        out.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let body = std::fs::read_to_string(path).expect("read report");
    assert!(body.contains("high_rate_qldpc_like"));
    assert!(body.contains("Atoms per logical | 24"));
}

/// `--emit-qldpc-report` is required; omitting it fails with an actionable
/// error.
#[test]
fn graph_mode_requires_emit_flag() {
    let graph = workspace_path("../examples/na_qec/qldpc_5qubit.json");
    let out = quonc()
        .arg("--qldpc-graph")
        .arg(&graph)
        .output()
        .expect("run quonc");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("--emit-qldpc-report"),
        "error should mention --emit-qldpc-report: {err}"
    );
}

/// `--qldpc-graph` and `--qldpc-net-rate` are mutually exclusive.
#[test]
fn graph_and_net_rate_are_mutually_exclusive() {
    let graph = workspace_path("../examples/na_qec/qldpc_5qubit.json");
    let out = quonc()
        .arg("--qldpc-graph")
        .arg(&graph)
        .arg("--qldpc-net-rate")
        .arg("1/24")
        .arg("--emit-qldpc-report")
        .arg("-")
        .output()
        .expect("run quonc");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("mutually exclusive"),
        "error should mention mutual exclusivity: {err}"
    );
}

/// Invalid net-rate format fails with an actionable error.
#[test]
fn net_rate_rejects_bad_format() {
    let out = quonc()
        .arg("--qldpc-net-rate")
        .arg("not-a-rate")
        .arg("--emit-qldpc-report")
        .arg("-")
        .output()
        .expect("run quonc");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("--qldpc-net-rate"),
        "error should mention --qldpc-net-rate: {err}"
    );
}

/// Missing graph file fails with an actionable error.
#[test]
fn graph_mode_rejects_missing_file() {
    let out = quonc()
        .arg("--qldpc-graph")
        .arg("/nonexistent/qldpc.json")
        .arg("--emit-qldpc-report")
        .arg("-")
        .output()
        .expect("run quonc");
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("read qLDPC graph"),
        "error should mention read failure: {err}"
    );
}
