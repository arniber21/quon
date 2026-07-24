//! OpenQASM 2/3 ingestion for the neutral-atom pipeline (#304, NA-scoped
//! slice of #197).
//!
//! Acceptance: a `.qasm` counterpart of `test/na/ising_n42.qn` enters the
//! existing NA pipeline at the interaction-graph seam and reproduces the
//! same pre-flight invariants — `entangle2_count == 82`, `rydberg_stages ==
//! 4` — as the `.qn` fixture (see `quonc/tests/rap_table_i.rs`). The QASM
//! circuit is a structural twin (two Trotter steps of a 42-qubit chain,
//! even/odd matchings per step); it carries no 1-qubit gates, so the
//! 82/4 counts are identical to the `.qn` fixture's interaction-graph-only
//! slice (the `.qn` fixture's `Rx` layer is invisible to 2q extraction).
//!
//! Also covers that unsupported QASM constructs fail with actionable,
//! line-tagged errors rather than silently dropping gates (#304 step 6).

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
    workspace_path("../targets/neutral_atom/rap_table_i.json")
}

fn qasm_fixture() -> PathBuf {
    workspace_path("../test/na/ising_n42.qasm")
}

fn qn_fixture() -> PathBuf {
    workspace_path("../test/na/ising_n42.qn")
}

fn u64_field(report: &Value, field: &str) -> u64 {
    report[field]
        .as_u64()
        .unwrap_or_else(|| panic!("resource report missing u64 field `{field}`: {report}"))
}

/// Run `quonc <src> --target <target> --emit-resource-report -` and parse the
/// JSON report from stdout. Fails (with stderr) on a non-zero exit.
fn resource_report(src: &std::path::Path) -> Value {
    let output = quonc()
        .arg(src)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-resource-report")
        .arg("-")
        .arg("--quiet")
        .output()
        .expect("spawn quonc");
    assert!(
        output.status.success(),
        "quonc failed for {} ({}); stderr: {}",
        src.display(),
        src.file_name().unwrap().to_string_lossy(),
        String::from_utf8_lossy(&output.stderr),
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout).unwrap_or_else(|e| {
        panic!(
            "parse resource report JSON for {}: {e}\n{stdout}",
            src.display()
        )
    })
}

/// The QASM fixture reproduces the `.qn` fixture's pre-flight invariants
/// ([RAP] Table I ising n=42): 82 two-qubit gates over 4 entangling layers.
#[test]
fn ising_n42_qasm_matches_qn_preflight_invariants() {
    let qasm = resource_report(&qasm_fixture());
    assert_eq!(
        u64_field(&qasm, "entangle2_count"),
        82,
        "ising_n42.qasm must compile to exactly 82 Entangle2 actions; got: {qasm}"
    );
    assert_eq!(
        u64_field(&qasm, "rydberg_stages"),
        4,
        "ising_n42.qasm must schedule into exactly 4 rydberg stages; got: {qasm}"
    );
    assert_eq!(u64_field(&qasm, "logical_qubits"), 42);

    // The `.qn` fixture must agree on these placer-independent pre-flight
    // counts — guarding against a regression that only realigns one side of
    // the twin-fixture seam.
    let qn = resource_report(&qn_fixture());
    assert_eq!(
        u64_field(&qn, "entangle2_count"),
        82,
        "ising_n42.qn baseline drifted; got: {qn}"
    );
    assert_eq!(
        u64_field(&qn, "rydberg_stages"),
        4,
        "ising_n42.qn baseline drifted; got: {qn}"
    );
}

/// The `.qasm` extension selects the ingestion path even without
/// `--from-qasm`; the explicit flag forces it for a non-`.qasm` source.
#[test]
fn from_qasm_flag_forces_ingestion_of_non_qasm_source() {
    // Same circuit content as the fixture, fed via stdin-path-less temp file
    // with a `.txt` extension so only `--from-qasm` can select the path.
    let tmp = tempfile::Builder::new()
        .suffix(".txt")
        .tempfile()
        .expect("create temp file");
    std::fs::write(tmp.path(), "OPENQASM 2.0;\nqreg q[2];\ncz q[0],q[1];\n")
        .expect("write temp circuit");
    let output = quonc()
        .arg(tmp.path())
        .arg("--from-qasm")
        .arg("--target")
        .arg(na_target())
        .arg("--emit-resource-report")
        .arg("-")
        .arg("--quiet")
        .output()
        .expect("spawn quonc");
    assert!(
        output.status.success(),
        "--from-qasm should ingest a non-.qasm source; stderr: {}",
        String::from_utf8_lossy(&output.stderr),
    );
    let report: Value =
        serde_json::from_str(&String::from_utf8_lossy(&output.stdout)).expect("parse report");
    assert_eq!(u64_field(&report, "entangle2_count"), 1);
    assert_eq!(u64_field(&report, "rydberg_stages"), 1);
    assert_eq!(u64_field(&report, "logical_qubits"), 2);
}

/// A non-`.qasm` source without `--from-qasm` is NOT routed through the
/// QASM parser — it falls through to the `.qn` frontend and fails with a
/// Quon parse error (not a QASM error). Confirms the extension/flag gate is
/// real, not a silent always-on path.
#[test]
fn non_qasm_source_without_flag_is_not_ingested_as_qasm() {
    let tmp = tempfile::Builder::new()
        .suffix(".txt")
        .tempfile()
        .expect("create temp file");
    std::fs::write(tmp.path(), "qreg q[2];\ncz q[0],q[1];\n").expect("write temp circuit");
    let output = quonc()
        .arg(tmp.path())
        .arg("--target")
        .arg(na_target())
        .arg("--emit-resource-report")
        .arg("-")
        .output()
        .expect("spawn quonc");
    assert!(
        !output.status.success(),
        "non-.qasm source without --from-qasm should fail (.qn frontend), not silently ingest as QASM"
    );
}

/// QASM ingestion requires a neutral-atom target — a fixed target is an
/// actionable, named error, not a silent fallback to the OpenQASM emit path.
#[test]
fn qasm_ingestion_requires_neutral_atom_target() {
    let output = quonc()
        .arg(qasm_fixture())
        .arg("--emit-qasm")
        .arg("--quiet")
        .output()
        .expect("spawn quonc");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("neutral-atom") || stderr.contains("neutral_atom"),
        "fixed-target QASM ingestion should name the neutral-atom requirement; stderr: {stderr}"
    );
}

/// Unsupported QASM constructs (gate definitions, classical control flow)
/// fail with an actionable, line-tagged error mentioning the construct —
/// never a silent drop (#304 step 6).
#[test]
fn unsupported_gate_definition_is_actionable() {
    let tmp = tempfile::Builder::new()
        .suffix(".qasm")
        .tempfile()
        .expect("create temp file");
    std::fs::write(
        tmp.path(),
        "OPENQASM 2.0;\nqreg q[2];\ngate mygate a { x a; }\n",
    )
    .expect("write temp circuit");
    let output = quonc()
        .arg(tmp.path())
        .arg("--target")
        .arg(na_target())
        .arg("--emit-resource-report")
        .arg("-")
        .output()
        .expect("spawn quonc");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unsupported") && stderr.contains("gate definition"),
        "gate definitions should be rejected as actionable unsupported constructs; stderr: {stderr}"
    );
}

/// An out-of-range qubit index is an actionable error naming the register,
/// the index, and the size — not a silent clamp or drop.
#[test]
fn out_of_range_qubit_is_actionable() {
    let tmp = tempfile::Builder::new()
        .suffix(".qasm")
        .tempfile()
        .expect("create temp file");
    std::fs::write(tmp.path(), "qreg q[2];\ncx q[0],q[5];\n").expect("write temp circuit");
    let output = quonc()
        .arg(tmp.path())
        .arg("--target")
        .arg(na_target())
        .arg("--emit-resource-report")
        .arg("-")
        .output()
        .expect("spawn quonc");
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("out of range") && stderr.contains("line "),
        "out-of-range qubit should be an actionable line-tagged error; stderr: {stderr}"
    );
}
