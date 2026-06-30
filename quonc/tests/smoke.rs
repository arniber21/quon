use std::path::PathBuf;
use std::process::Command;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

/// Resolve a path relative to the `quonc` crate root, so tests can reach the
/// shared `.qn` and device-descriptor fixtures that live in sibling crates.
fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

#[test]
fn quonc_help_exits_successfully() {
    let output = quonc().arg("--help").output().expect("failed to run quonc");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Quon quantum compiler"));
}

/// End-to-end driver acceptance for issue #27: the `bell_state` fixture compiles
/// source → quantum.circ → quantum.dynamic → OpenQASM 3.0 and emits exactly the
/// expected document on the default `generic_openqasm` target.
#[test]
fn bell_state_emits_expected_openqasm3() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let output = quonc()
        .arg("--emit-qasm")
        .arg(&source)
        .output()
        .expect("failed to run quonc");

    assert!(
        output.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let qasm = String::from_utf8_lossy(&output.stdout);
    let expected = "\
OPENQASM 3.0;
include \"stdgates.inc\";
qubit[2] q;
bit[2] c;
h q[0];
cx q[0], q[1];
c[0] = measure q[0];
c[1] = measure q[1];
";
    assert_eq!(qasm, expected);
}

/// Issue #27 acceptance: a gate that is not native to the selected target must be
/// a hard error, never silently-emitted invalid QASM. `device_5q.json`'s native
/// set is `{cx, rz, sx, x}` — it has no `h`, so the Bell `H` cannot resolve.
#[test]
fn non_native_gate_fails_loudly() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let target = workspace_path("../backend/tests/fixtures/device_5q.json");
    let output = quonc()
        .arg("--emit-qasm")
        .arg("--target")
        .arg(&target)
        .arg(&source)
        .output()
        .expect("failed to run quonc");

    assert!(
        !output.status.success(),
        "expected failure on a non-native gate, but quonc succeeded with:\n{}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("not native"),
        "expected a non-native-gate error, got:\n{stderr}"
    );
    // The failure must be reported, not a partial QASM document on stdout.
    assert!(
        String::from_utf8_lossy(&output.stdout).is_empty(),
        "no QASM should be emitted when emission fails"
    );
}
