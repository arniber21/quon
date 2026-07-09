//! CLI integration tests for `quonc` — help, flags, and clap error paths.
//! Full compile/emit behavior is covered by `smoke.rs`; these tests exercise
//! only the clap surface.

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
    assert!(stdout.contains("Quon quantum compiler"));
    assert!(stdout.contains("emit-qasm"));
    assert!(stdout.contains("target"));
    assert!(stdout.contains("dump-ir"));
    assert!(stdout.contains("verify-linear"));
    assert!(stdout.contains("print-target"));
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
        stderr.contains("required") || stderr.contains("USAGE") || stderr.contains("source"),
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
    assert_eq!(
        String::from_utf8_lossy(&short.stdout),
        String::from_utf8_lossy(&long.stdout)
    );
}

#[test]
fn help_lists_all_documented_flags() {
    let out = quonc().arg("--help").output().expect("spawn");
    let stdout = String::from_utf8_lossy(&out.stdout);
    for flag in [
        "--emit-qasm",
        "--target",
        "--print-target",
        "--dump-ir",
        "--verify-linear",
        "--metrics",
        "--metrics-json",
        "--metrics-snapshot",
        "--regression-config",
        "--watch",
        "--watch-debounce-ms",
    ] {
        assert!(stdout.contains(flag), "missing {flag} in --help");
    }
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
