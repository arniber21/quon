//! CLI integration tests for `quonc` — help, flags, and clap error paths.
//! Full compile/emit behavior is covered by `smoke.rs` / `na_pipeline.rs`.

use std::path::Path;
use std::process::Command;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

#[test]
fn help_shows_usage() {
    let out = quonc().arg("--help").output().expect("spawn");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Quon quantum compiler") || stdout.contains("OpenQASM"));
    assert!(stdout.contains("emit-qasm"));
    assert!(stdout.contains("emit-na-schedule"));
    assert!(stdout.contains("emit-na-graph"));
    assert!(stdout.contains("target"));
    assert!(stdout.contains("dump-ir"));
    assert!(stdout.contains("verify-linear"));
    assert!(stdout.contains("print-target"));
    assert!(stdout.contains("list-passes"));
    assert!(stdout.contains("Examples:"));
}

#[test]
fn version_flag_works() {
    let out = quonc().arg("--version").output().expect("spawn");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("quonc"));
}

#[test]
fn missing_source_is_error() {
    let out = quonc().output().expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("required") || stderr.contains("Usage") || stderr.contains("source"),
        "stderr: {stderr}"
    );
}

#[test]
fn unknown_flag_is_error() {
    let out = quonc()
        .args(["--not-a-real-flag", "x.qn"])
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("unexpected") || stderr.contains("unrecognized"));
}

#[test]
fn short_help_matches_long() {
    let short = quonc().arg("-h").output().expect("spawn");
    let long = quonc().arg("--help").output().expect("spawn");
    assert_eq!(short.status, long.status);
    // Long help includes examples; short is a summary — both must succeed.
    assert!(short.status.success());
    assert!(long.status.success());
}

#[test]
fn help_lists_all_documented_flags() {
    let out = quonc().arg("--help").output().expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    for flag in [
        "--emit-qasm",
        "--emit-na-schedule",
        "--emit-na-graph",
        "--emit-resource-report",
        "--emit-qec-experiment",
        "--resource-report-format",
        "--target",
        "--print-target",
        "--dump-ir",
        "--verify-linear",
        "--verify-na",
        "--list-passes",
        "--metrics",
        "--metrics-json",
        "--metrics-snapshot",
        "--regression-config",
        "--watch",
        "--watch-debounce-ms",
        "--na-backend",
        "--na-placer",
        "--no-na-compact",
        "--na-placement",
        "--color",
        "--quiet",
        "--sabre-gamma",
        "--sabre-beta",
        "--sabre-lookahead",
    ] {
        assert!(stdout.contains(flag), "missing {flag} in --help");
    }
}

#[test]
fn list_passes_exits_successfully() {
    let out = quonc().arg("--list-passes").output().expect("spawn");
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("Neutral-atom path"));
    assert!(stdout.contains("extract_interaction_graph"));
    assert!(stdout.contains("OpenQASM"));
}

#[test]
fn print_target_does_not_require_source() {
    let target =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../targets/neutral_atom/generic_rna_v0.json");
    let out = quonc()
        .arg("--target")
        .arg(target)
        .arg("--print-target")
        .output()
        .expect("spawn");

    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("generic_reconfigurable_neutral_atom_v0"));
    assert!(stdout.contains("kind: neutral_atom_reconfigurable"));
    assert!(stdout.contains("max_parallel_pairs=340"));
}

#[test]
fn emit_na_flags_require_na_target() {
    let source =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../frontend/tests/fixtures/bell_state.qn");
    let out = quonc()
        .arg(&source)
        .arg("--emit-resource-report")
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("neutral_atom_reconfigurable") || stderr.contains("neutral-atom"),
        "stderr: {stderr}"
    );
}

#[test]
fn emit_qasm_rejects_na_target() {
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("../test/na/bell.qn");
    let target =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../targets/neutral_atom/generic_rna_v0.json");
    let out = quonc()
        .arg(&source)
        .arg("--target")
        .arg(target)
        .arg("--emit-qasm")
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("emit-qasm") && stderr.contains("fixed"),
        "stderr: {stderr}"
    );
}
