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

fn write_temp_source(name: &str, source: &str) -> PathBuf {
    let path = std::env::temp_dir().join(format!("quon-{name}-{}.qn", std::process::id()));
    std::fs::write(&path, source).expect("failed to write temporary Quon source");
    path
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

#[test]
fn teleport_feed_forward_emits_integer_conditions() {
    let source = write_temp_source(
        "teleport-feed-forward",
        r#"
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}

fn bell_unstate(): Circuit<2, 2, 2, Clifford> = circuit {
    CNOT @(0, 1) |> H @0
}

fn id1(): Circuit<1, 1, 1, Clifford> = circuit {
    I @0
}

fn x1(): Circuit<1, 1, 1, Clifford> = circuit {
    X @0
}

fn z1(): Circuit<1, 1, 1, Clifford> = circuit {
    Z @0
}

fn teleport_demo(): Q<Qubit> = run {
    (msg, alice, bob) <- qreg(3)
    (a, b)            <- bell_state() @ (alice, bob)
    (m2, a2)          <- bell_unstate() @ (msg, a)
    x_bit             <- measure(m2)
    z_bit             <- measure(a2)
    let b2             = (if x_bit then x1() else id1()) @ b
    let b3             = (if z_bit then z1() else id1()) @ b2
    return b3
}
"#,
    );
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
    assert!(
        qasm.contains("if (c[0] == 1) {\n  x q[2];\n}"),
        "missing X feed-forward correction:\n{qasm}"
    );
    assert!(
        qasm.contains("if (c[1] == 1) {\n  z q[2];\n}"),
        "missing Z feed-forward correction:\n{qasm}"
    );
    assert!(
        !qasm.contains("true"),
        "QASM conditions must use integer comparisons:\n{qasm}"
    );
}

#[test]
fn parameterized_teleport_fails_with_explicit_deferral() {
    let source = write_temp_source(
        "parameterized-teleport",
        &format!(
            "{}\n{}",
            include_str!("../../frontend/tests/fixtures/bell_state.qn"),
            include_str!("../../frontend/tests/fixtures/teleport.qn")
        ),
    );
    let output = quonc()
        .arg("--emit-qasm")
        .arg(&source)
        .output()
        .expect("failed to run quonc");

    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("parameterized run function `teleport` is not supported"),
        "expected explicit parameterized run deferral, got:\n{stderr}"
    );
}

#[test]
fn parameterized_bv_and_grover_fail_with_explicit_deferral() {
    for (name, fixture) in [
        (
            "bernstein-vazirani",
            include_str!("../../frontend/tests/fixtures/bernstein_vazirani.qn"),
        ),
        (
            "grover",
            include_str!("../../frontend/tests/fixtures/grover.qn"),
        ),
    ] {
        let source = write_temp_source(name, fixture);
        let output = quonc()
            .arg("--emit-qasm")
            .arg(&source)
            .output()
            .expect("failed to run quonc");

        assert!(!output.status.success(), "expected {name} to be deferred");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            stderr.contains("parameterized circuit function `hadamard_all` is not supported"),
            "expected explicit parameterized circuit deferral for {name}, got:\n{stderr}"
        );
    }
}
