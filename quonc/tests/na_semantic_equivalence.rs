//! Semantic (unitary) equivalence checks for the NA single-qubit gate path
//! (issue #298 review finding #1 — CRITICAL).
//!
//! Structural checks (op presence, gate/atom counts, dialect verification)
//! cannot distinguish "every 1-qubit gate is present and correctly scoped"
//! from "every 1-qubit gate is present, but a `GlobalRy` silently also
//! rotated every other trapped atom" — both produce identical op counts and
//! both pass `--verify-na` (a structural verifier, not a semantic one). This
//! file interprets an emitted `--emit-na-schedule` JSON's `LocalGate`/
//! `GlobalRy` actions as an actual product of `backend::unitary` matrices
//! per atom and checks the result against what the source `.qn` program
//! asked for — the same class of check the reviewer asked for after finding
//! that a naive `GlobalRy` implementation corrupted bystander qubits'
//! state (every atom is bound into the trap array from schedule start via
//! `layout.initial_bindings`, so a bare raster physically hits everyone, not
//! just the atom it was decomposed for).
//!
//! Only `LocalGate`/`GlobalRy` actions are interpreted — `Entangle2`/`Move`/
//! `Transfer` are ignored, so this checks "does the schedule realize the
//! source program's local single-qubit rotations correctly, independent of
//! entangling-gate realization", which is exactly issue #298's scope.

use std::path::PathBuf;
use std::process::Command;

use backend::unitary::{Complex, M2, gate_unitary, mul2, rotation_unitary, unitary_distance2};
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

fn identity() -> M2 {
    M2([
        [Complex::new(1.0, 0.0), Complex::new(0.0, 0.0)],
        [Complex::new(0.0, 0.0), Complex::new(1.0, 0.0)],
    ])
}

/// `true` if `a` equals `b` up to an unobservable global phase.
fn close_up_to_phase(a: M2, b: M2) -> bool {
    unitary_distance2(a, b) < 1e-6
}

/// Emit the `--emit-na-schedule` JSON for `source` on the default (zoned)
/// backend and return it parsed.
fn emit_schedule(source: &PathBuf) -> Value {
    let output = quonc()
        .arg(source)
        .arg("--target")
        .arg(na_target())
        .arg("--emit-na-schedule")
        .arg("-")
        .arg("--verify-na")
        .arg("--quiet")
        .output()
        .expect("spawn");
    assert!(
        output.status.success(),
        "compile failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("parse schedule JSON")
}

/// Walk `schedule`'s layers in cycle order and compute each atom's
/// cumulative net unitary from `LocalGate`/`GlobalRy` actions only.
fn net_unitaries(schedule: &Value, num_atoms: usize) -> Vec<M2> {
    let mut net = vec![identity(); num_atoms];
    let layers = schedule["layers"].as_array().expect("layers array");
    for layer in layers {
        let actions = layer["actions"].as_array().expect("actions array");
        for action in actions {
            if let Some(local) = action.get("LocalGate") {
                let atom = local["atom"].as_u64().expect("atom") as usize;
                let gate = &local["gate"];
                let u = local_gate_unitary(gate);
                net[atom] = mul2(u, net[atom]);
            } else if let Some(global) = action.get("GlobalRy") {
                let theta = global["theta_rad"].as_f64().expect("theta_rad");
                let u = rotation_unitary("ry", theta).expect("ry unitary");
                for slot in net.iter_mut() {
                    *slot = mul2(u, *slot);
                }
            }
        }
    }
    net
}

fn local_gate_unitary(gate: &Value) -> M2 {
    if let Some(theta) = gate.get("rz").and_then(Value::as_f64) {
        return rotation_unitary("rz", theta).expect("rz unitary");
    }
    if let Some(u3) = gate.get("u3") {
        let theta = u3["theta"].as_f64().expect("theta");
        let phi = u3["phi"].as_f64().expect("phi");
        let lambda = u3["lambda"].as_f64().expect("lambda");
        let rz_phi = rotation_unitary("rz", phi).expect("rz");
        let ry_theta = rotation_unitary("ry", theta).expect("ry");
        let rz_lambda = rotation_unitary("rz", lambda).expect("rz");
        return mul2(rz_phi, mul2(ry_theta, rz_lambda));
    }
    if gate.as_str() == Some("h") {
        return gate_unitary("H").expect("H unitary");
    }
    panic!("unrecognized LocalGate kind in schedule JSON: {gate}");
}

/// Minimal repro from the issue #298 review: two independent qubits, no
/// entangling gate between them, each with its own 1-qubit rotation.
/// Pre-fix, both atoms' net rotation was a garbled composition of *both*
/// gates (each `GlobalRy` hit both atoms); each must now realize exactly
/// its own source gate and nothing else.
#[test]
fn h_and_ry_each_atom_realizes_only_its_own_gate() {
    let source = workspace_path("../test/na/h_and_ry.qn");
    let schedule = emit_schedule(&source);
    let net = net_unitaries(&schedule, 2);

    let expected_h = gate_unitary("H").expect("H unitary");
    let expected_ry = rotation_unitary("ry", std::f64::consts::FRAC_PI_4).expect("ry unitary");

    assert!(
        close_up_to_phase(net[0], expected_h),
        "atom0 should realize exactly H, got {:?}",
        net[0]
    );
    assert!(
        close_up_to_phase(net[1], expected_ry),
        "atom1 should realize exactly Ry(pi/4), got {:?}",
        net[1]
    );
    // The corruption this test guards against was symmetric: if either atom
    // absorbed the *other* atom's rotation on top of its own, it would no
    // longer match its own intended gate alone.
    assert!(
        !close_up_to_phase(net[0], expected_ry),
        "atom0 must not also carry atom1's Ry(pi/4)"
    );
    assert!(
        !close_up_to_phase(net[1], expected_h),
        "atom1 must not also carry atom0's H"
    );
}

/// `test/lit/na/parallel_gates_na_mlir.qn`: `H @0 |> H @2 |> CNOT @(0,1) |>
/// CNOT @(2,3)`. Two atoms receive `H`; the other two receive no 1-qubit
/// gate at all in the source program and must net to identity. (Extraction
/// numbers atoms by first-appearance-as-a-gate-operand, not by the source
/// program's `@N` index, so this checks the *set* of outcomes rather than
/// assuming a specific atom-id <-> `@N` correspondence — see
/// `quon_na::extract`'s module doc comment.)
#[test]
fn parallel_gates_bystanders_net_to_identity_not_h() {
    let source = workspace_path("../test/lit/na/parallel_gates_na_mlir.qn");
    let schedule = emit_schedule(&source);
    let net = net_unitaries(&schedule, 4);

    let expected_h = gate_unitary("H").expect("H unitary");
    let mut h_count = 0;
    let mut identity_count = 0;
    for (atom, u) in net.iter().enumerate() {
        let is_h = close_up_to_phase(*u, expected_h);
        let is_identity = close_up_to_phase(*u, identity());
        assert!(
            is_h || is_identity,
            "atom{atom} net unitary should be exactly H or exactly identity \
             (bystander corruption would produce neither): {u:?}"
        );
        h_count += is_h as u32;
        identity_count += is_identity as u32;
    }
    assert_eq!(
        h_count, 2,
        "exactly 2 atoms received H in the source program"
    );
    assert_eq!(
        identity_count, 2,
        "exactly 2 atoms received no 1-qubit gate and must net to identity"
    );
}

/// `test/na/rotations.qn`: `H @0 |> Rz(t1) @1 |> CNOT @(0,1) |> Ry(t2) @2 |>
/// CNOT @(1,2) |> Rz(t3) @0`. Every qubit's first appearance as a gate
/// operand matches its source `@N` index here (no reordering — see the
/// `parallel_gates` test above for a case where that assumption breaks), so
/// atom ids are asserted directly against each qubit's exact expected
/// composition.
#[test]
fn rotations_qn_each_atom_realizes_exactly_its_own_local_gates() {
    let source = workspace_path("../test/na/rotations.qn");
    let schedule = emit_schedule(&source);
    let net = net_unitaries(&schedule, 3);

    let rz1 = std::f64::consts::FRAC_PI_8;
    let ry2 = std::f64::consts::FRAC_PI_4;
    let rz3 = std::f64::consts::FRAC_PI_2;

    // atom0: H first, then Rz(rz3) later — net = Rz(rz3) . H
    let h = gate_unitary("H").expect("H unitary");
    let rz3_u = rotation_unitary("rz", rz3).expect("rz unitary");
    let expected0 = mul2(rz3_u, h);
    assert!(
        close_up_to_phase(net[0], expected0),
        "atom0 should realize Rz({rz3}) . H, got {:?}",
        net[0]
    );

    // atom1: only Rz(rz1)
    let expected1 = rotation_unitary("rz", rz1).expect("rz unitary");
    assert!(
        close_up_to_phase(net[1], expected1),
        "atom1 should realize exactly Rz({rz1}), got {:?}",
        net[1]
    );

    // atom2: only Ry(ry2)
    let expected2 = rotation_unitary("ry", ry2).expect("ry unitary");
    assert!(
        close_up_to_phase(net[2], expected2),
        "atom2 should realize exactly Ry({ry2}), got {:?}",
        net[2]
    );
}

/// `bell.qn` on the `na_target_no_ry` fixture takes the `u3` escape hatch
/// (native set lacks `ry`); its single atom must still realize exactly `H`.
#[test]
fn u3_escape_hatch_realizes_the_source_gate() {
    let source = workspace_path("../test/na/bell.qn");
    let output = quonc()
        .arg(&source)
        .arg("--target")
        .arg(workspace_path("tests/fixtures/na_target_no_ry.json"))
        .arg("--emit-na-schedule")
        .arg("-")
        .arg("--verify-na")
        .arg("--quiet")
        .output()
        .expect("spawn");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let schedule: Value = serde_json::from_slice(&output.stdout).expect("parse schedule JSON");
    let net = net_unitaries(&schedule, 2);
    let expected_h = gate_unitary("H").expect("H unitary");
    assert!(
        close_up_to_phase(net[0], expected_h),
        "atom0 (u3 escape hatch) should realize exactly H, got {:?}",
        net[0]
    );
}
