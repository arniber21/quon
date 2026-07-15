//! Integration: repetition-code d=3 memory end-to-end (#248).

use std::path::PathBuf;
use std::process::Command;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn na_target() -> PathBuf {
    workspace_path("../targets/neutral_atom/generic_rna_v0.json")
}

#[test]
fn repetition_d3_memory_resource_counts() {
    let source = workspace_path("../examples/na_qec/repetition_d3_memory.qn");
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
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    // When only resource report uses `-`, it goes to stdout.
    let stdout = String::from_utf8_lossy(&output.stdout);
    let report = if stdout.contains("\"logical_qubits\"") {
        stdout
    } else {
        String::from_utf8_lossy(&output.stderr).into_owned().into()
    };

    assert!(
        report.contains("\"logical_qubits\": 1"),
        "logical_qubits: {report}"
    );
    assert!(
        report.contains("\"physical_atoms\": 5"),
        "physical_atoms (N=2d-1): {report}"
    );
    assert!(
        report.contains("\"atoms_per_logical\": 5"),
        "atoms_per_logical: {report}"
    );
    assert!(
        report.contains("\"code_family\": \"repetition_code_toy\""),
        "code_family: {report}"
    );
    assert!(report.contains("\"distance\": 3"), "distance: {report}");
    assert!(
        report.contains("\"memory_rounds\": 2"),
        "memory_rounds: {report}"
    );
    assert!(
        report.contains("\"entangle2_count\": 8"),
        "two rounds × 4 CZs: {report}"
    );
    assert!(
        report.contains("\"measurement_rounds\": 3"),
        "2 check rounds + 1 logical: {report}"
    );
    assert!(
        report.contains("\"reset_rounds\": 2"),
        "one reset layer per memory round: {report}"
    );
}

#[test]
fn bare_qubit_bell_still_uses_physical_path() {
    let source = workspace_path("../test/na/bell.qn");
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
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        text.contains("\"logical_qubits\": 2"),
        "bare path should stay 1:1 physical: {text}"
    );
    assert!(
        !text.contains("\"code_family\""),
        "bare path must not set QEC code_family: {text}"
    );
    assert!(
        !text.contains("\"memory_rounds\""),
        "bare path must not set memory_rounds: {text}"
    );
}
