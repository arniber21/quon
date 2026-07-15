//! Integration: repetition-code d=3 memory end-to-end (#248).

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

#[test]
fn repetition_d3_memory_resource_counts() {
    let source = workspace_path("../examples/na_qec/repetition_d3_memory.qn");
    let report_path =
        std::env::temp_dir().join(format!("quon-qec-248-report-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&report_path);

    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-resource-report")
        .arg(&report_path)
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let report_text = std::fs::read_to_string(&report_path)
        .unwrap_or_else(|e| panic!("read report {}: {e}", report_path.display()));
    let report: Value = serde_json::from_str(&report_text)
        .unwrap_or_else(|e| panic!("parse JSON: {e}\n{report_text}"));

    assert_eq!(report["logical_qubits"], 1);
    assert_eq!(report["physical_atoms"], 5);
    assert_eq!(report["atoms_per_logical"], 5);
    assert_eq!(report["code_family"], "repetition_code_toy");
    assert_eq!(report["distance"], 3);
    assert_eq!(report["memory_rounds"], 2);
    assert_eq!(
        report["entangle2_count"], 8,
        "two rounds × 4 CNOTs: {report_text}"
    );
    assert_eq!(
        report["measurement_rounds"], 3,
        "2 check rounds + 1 logical: {report_text}"
    );
    assert_eq!(
        report["reset_rounds"], 2,
        "one reset layer per memory round: {report_text}"
    );

    let _ = std::fs::remove_file(&report_path);
}

#[test]
fn bare_qubit_bell_still_uses_physical_path() {
    let source = workspace_path("../test/na/bell.qn");
    let report_path =
        std::env::temp_dir().join(format!("quon-qec-248-bare-{}.json", std::process::id()));
    let _ = std::fs::remove_file(&report_path);

    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-resource-report")
        .arg(&report_path)
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report_text = std::fs::read_to_string(&report_path)
        .unwrap_or_else(|e| panic!("read report {}: {e}", report_path.display()));
    let report: Value = serde_json::from_str(&report_text)
        .unwrap_or_else(|e| panic!("parse JSON: {e}\n{report_text}"));

    assert_eq!(report["logical_qubits"], 2);
    assert!(
        report.get("code_family").is_none_or(|v| v.is_null()),
        "bare path must not set QEC code_family: {report_text}"
    );
    assert!(
        report.get("memory_rounds").is_none_or(|v| v.is_null()),
        "bare path must not set memory_rounds: {report_text}"
    );

    let _ = std::fs::remove_file(&report_path);
}
