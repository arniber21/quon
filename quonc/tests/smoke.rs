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
    assert!(
        stdout.contains("Quon quantum compiler") || stdout.contains("OpenQASM"),
        "unexpected help: {stdout}"
    );
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
/// a hard error, never silently-emitted invalid QASM. `device_no_entangler.json`
/// has no two-qubit native gate at all (no `cx`/`cz`/`swap`), so the Bell
/// `CNOT` has no decomposition path — unlike `device_5q.json`, whose `{rz, sx}`
/// pair lets native gate decomposition (#24) synthesize `H` successfully, so it
/// no longer demonstrates a hard emission failure.
#[test]
fn non_native_gate_fails_loudly() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let target = workspace_path("../backend/tests/fixtures/device_no_entangler.json");
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

/// End-to-end acceptance for the M1 pipeline-wiring milestone (issue #1): on a
/// topology-constrained target, the driver must reach the *actually executed*
/// circuit (not just dead, pre-inlining `quantum.circ.func` definitions — see
/// `native_gate_decomp::decompose_block`'s doc comment) and route it so that
/// only the target's native gates are emitted. `device_linear_chain.json` (a
/// bare 0-1-2-3 chain, no shortcuts) forces real SWAP insertion for BV's
/// `CNOT @(0, 3)`. This exact output is a regression lock for three
/// correctness bugs found wiring this milestone (all in `sabre_routing.rs`):
/// a bystander qubit displaced by an unrelated swap kept a stale SSA operand
/// on its next gate and on the block terminator, and `wires[logical]` was
/// never updated to a gate's own result — see git history on this test for
/// the full analysis. Golden updated for issue #181 (β · critical_path_delta
/// + lookahead) which changes SWAP choice vs the α/γ-only cost. Verified
/// against Qiskit Aer to recover the BV secret (1,1,0) unchanged from the
/// unrouted `generic_openqasm` result.
#[test]
fn bernstein_vazirani_routes_and_emits_only_native_gates_on_linear_chain() {
    let source = workspace_path("../test/verify/bernstein_vazirani.qn");
    let target = workspace_path("../backend/tests/fixtures/device_linear_chain.json");
    let output = quonc()
        .arg("--emit-qasm")
        .arg("--target")
        .arg(&target)
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
qubit[4] q;
bit[4] c;
x q[3];
rz(3.141592653589793) q[0];
sx q[0];
rz(4.71238898038469) q[0];
sx q[0];
rz(3.141592653589793) q[0];
rz(3.141592653589793) q[1];
sx q[1];
rz(4.71238898038469) q[1];
sx q[1];
rz(3.141592653589793) q[1];
rz(3.141592653589793) q[3];
sx q[3];
rz(4.71238898038469) q[3];
sx q[3];
rz(3.141592653589793) q[3];
cx q[2], q[3];
cx q[3], q[2];
cx q[2], q[3];
cx q[1], q[2];
cx q[2], q[1];
cx q[1], q[2];
cx q[0], q[1];
cx q[2], q[1];
rz(3.141592653589793) q[0];
sx q[0];
rz(4.71238898038469) q[0];
sx q[0];
rz(3.141592653589793) q[0];
rz(3.141592653589793) q[2];
sx q[2];
rz(4.71238898038469) q[2];
sx q[2];
rz(3.141592653589793) q[2];
c[0] = measure q[0];
c[1] = measure q[2];
c[2] = measure q[3];
c[3] = measure q[1];
";
    assert_eq!(qasm, expected);
    for gate in ["h", "cnot", "swap", "id"] {
        assert!(
            !qasm.to_lowercase().contains(gate),
            "only device_linear_chain's native gates (cx, rz, sx, x) may appear:\n{qasm}"
        );
    }
}

/// Since the measurement deferral pass (#22) is now wired into the default
/// pipeline (issue #1 MVP milestone M1), a mid-circuit measurement whose bit
/// feeds exactly one terminal `if` is rewritten into a terminal measurement
/// plus a controlled correction — no literal `if` survives in the emitted
/// QASM. This supersedes the pass's pre-wiring behaviour (raw feed-forward
/// `if` blocks), which is still exercised directly against the MLIR passes in
/// `mlir_bridge/tests/measurement_deferral.rs`.
#[test]
fn teleport_feed_forward_is_deferred_by_default() {
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
    let expected = "\
OPENQASM 3.0;
include \"stdgates.inc\";
qubit[3] q;
bit[2] c;
h q[1];
cx q[1], q[2];
cx q[0], q[1];
h q[0];
cx q[0], q[2];
c[0] = measure q[0];
cz q[1], q[2];
c[1] = measure q[1];
";
    assert_eq!(
        qasm, expected,
        "deferred X correction (cx q[0], q[2]) and Z correction (cz q[1], q[2]) \
         should replace the literal feed-forward `if` blocks"
    );
    assert!(
        !qasm.contains("if ("),
        "no literal feed-forward `if` should survive default measurement deferral:\n{qasm}"
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

/// The *reference* (general, not call-site-monomorphized) `grover.qn` /
/// `bernstein_vazirani.qn` fixtures each take `oracle: Oracle<n>` — a
/// circuit-*valued* entry-point parameter. Parametric circuit elaboration
/// (issue #1 MVP milestone M2, `frontend/src/elaborate.rs`) specializes
/// `Nat`/`Int`/`Float`-parameterized circuit *functions* called with concrete
/// arguments (see `bernstein_vazirani_routes_and_emits_only_native_gates_on_linear_chain`
/// and `test/verify/grover.qn` for exactly that) — a higher-order circuit
/// parameter is a different, out-of-scope feature (a real oracle must be
/// inlined into a concrete callee by the caller, as `test/verify/*.qn` do).
/// So each fixture now clears the *circuit-function* parametric barrier that
/// used to block it first, and instead fails one layer deeper, on its
/// parametric *entry point* — `ParametricRunFn`, `frontend/src/lower.rs`.
#[test]
fn parametric_entry_point_with_circuit_valued_param_fails_with_explicit_deferral() {
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
            stderr.contains("parameterized run function") && stderr.contains("is not supported"),
            "expected explicit parametric-entry-point deferral for {name}, got:\n{stderr}"
        );
    }
}
