//! Integration: `--verify-na` CLI and QEC auto-verify (ADR-0021 / #256).

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
fn qec_repetition_auto_verifies_on_emit() {
    let source = workspace_path("../examples/na_qec/repetition_d3_memory.qn");
    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-na-mlir")
        .arg("-")
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "QEC emit must auto-verify; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("quantum.na.schedule"),
        "missing schedule: {stdout}"
    );
}

#[test]
fn physical_na_skips_verify_without_flag() {
    let source = workspace_path("../test/na/bell.qn");
    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-na-mlir")
        .arg("-")
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "physical NA without --verify-na must succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn physical_na_verifies_with_flag() {
    let source = workspace_path("../test/na/bell.qn");
    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-na-mlir")
        .arg("-")
        .arg("--verify-na")
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "physical NA with --verify-na must succeed; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn standalone_mlir_rejects_round_barrier_violation() {
    let mlir = workspace_path("../test/lit/na/verify_round_barrier_same_cycle.mlir");
    let output = quonc()
        .arg(&mlir)
        .arg("--verify-na")
        .output()
        .expect("spawn");

    assert!(
        !output.status.success(),
        "expected verification failure; stdout: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("round-barrier") || stderr.contains("QEC round"),
        "stderr: {stderr}"
    );
}

#[test]
fn standalone_mlir_accepts_legal_schedule() {
    let mlir = workspace_path("../test/lit/na/verify_ok_roundtrip.mlir");
    let output = quonc()
        .arg(&mlir)
        .arg("--verify-na")
        .arg("--quiet")
        .output()
        .expect("spawn");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
