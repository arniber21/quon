//! Integration: surface-code d=3 memory end-to-end (#249).

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
fn surface_d3_memory_resource_counts() {
    let source = workspace_path("../examples/na_qec/surface_d3_memory.qn");
    let report_path =
        std::env::temp_dir().join(format!("quon-qec-249-report-{}.json", std::process::id()));
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
    assert_eq!(report["physical_atoms"], 17);
    assert_eq!(report["atoms_per_logical"], 17);
    assert_eq!(report["code_family"], "surface_code_like");
    assert_eq!(report["distance"], 3);
    assert_eq!(report["memory_rounds"], 2);
    assert_eq!(
        report["entangle2_count"], 48,
        "two rounds × 24 CNOTs: {report_text}"
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
