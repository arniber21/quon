//! End-to-end backend tests (issues #27, #29).
//!
//! These check the **tree**: compiling real source through frontend lowering →
//! monadic lowering → `reify` must produce an exact `quon_core::qasm::Program`,
//! asserted by object comparison. Codegen — turning a `Program` into OpenQASM
//! text — is tested separately as exact-string assertions in `quon_core::qasm`.
//! Together they pin both halves of emission independently.

use backend::generic_openqasm;
use mlir_bridge::emit::openqasm3;
use mlir_bridge::passes::monadic_lowering;
use quon_core::qasm::{Expr, OneQubitGate, Program, QasmError, QasmGate, Stmt, TwoQubitGate};

/// Compile a Quon source string to the reified QASM syntax tree.
fn reify(src: &str) -> Program {
    let context = melior::Context::new();
    let module = frontend::lower::lower_program(&context, src).expect("lower program");
    monadic_lowering::run_on_module(&context, &module).expect("monadic lowering");
    openqasm3::reify(&module, &generic_openqasm::target(8)).expect("reify")
}

const BELL: &str = r#"
fn bell_state(): Circuit<2, 2, 2, Clifford> = circuit {
    H @0 |> CNOT @(0, 1)
}

fn main(): Q<(Bit, Bit)> = run {
    (q0, q1) <- bell_state() @ qreg(2)
    b0       <- measure(q0)
    b1       <- measure(q1)
    return (b0, b1)
}
"#;

fn qubit(program: &Program, i: usize) -> quon_core::qasm::QubitId {
    let Some(id) = program.qubit(i) else {
        unreachable!("test qubit index {i} exceeds program bound")
    };
    id
}

fn bit(program: &Program, i: usize) -> quon_core::qasm::BitId {
    let Some(id) = program.bit(i) else {
        unreachable!("test bit index {i} exceeds program bound")
    };
    id
}

#[test]
fn bell_state_reifies_to_expected_tree() -> Result<(), QasmError> {
    let mut expected = Program::new(2, 2);
    expected.push_gate(QasmGate::One(OneQubitGate::H, qubit(&expected, 0)))?;
    expected.push_gate(QasmGate::Two(
        TwoQubitGate::Cx,
        qubit(&expected, 0),
        qubit(&expected, 1),
    ))?;
    expected.push_measure(qubit(&expected, 0), bit(&expected, 0))?;
    expected.push_measure(qubit(&expected, 1), bit(&expected, 1))?;

    assert_eq!(reify(BELL), expected);
    Ok(())
}

const TELEPORT: &str = r#"
fn prep(): Circuit<3, 3, 3, Clifford> = circuit { X @0 |> H @1 |> CNOT @(1, 2) }
fn bell_basis(): Circuit<2, 2, 2, Clifford> = circuit { CNOT @(0, 1) |> H @0 }
fn pauli_x(): Circuit<1, 1, 1, Clifford> = circuit { X @0 }
fn pauli_z(): Circuit<1, 1, 1, Clifford> = circuit { Z @0 }
fn id_one(): Circuit<1, 1, 1, Clifford> = circuit { I @0 }

fn main(): Q<Bit> = run {
    (msg, alice, bob) <- prep() @ qreg(3)
    (m2, a2)          <- bell_basis() @ (msg, alice)
    x_bit             <- measure(m2)
    z_bit             <- measure(a2)
    b2                <- (if z_bit then pauli_x() else id_one()) @ bob
    b3                <- (if x_bit then pauli_z() else id_one()) @ b2
    result            <- measure(b3)
    return result
}
"#;

#[test]
fn teleport_reifies_to_expected_feed_forward_tree() -> Result<(), QasmError> {
    let mut expected = Program::new(3, 3);
    // prep: message |1>, Bell pair on (alice, bob).
    expected.push_gate(QasmGate::One(OneQubitGate::X, qubit(&expected, 0)))?;
    expected.push_gate(QasmGate::One(OneQubitGate::H, qubit(&expected, 1)))?;
    expected.push_gate(QasmGate::Two(
        TwoQubitGate::Cx,
        qubit(&expected, 1),
        qubit(&expected, 2),
    ))?;
    // bell_basis on (msg, alice).
    expected.push_gate(QasmGate::Two(
        TwoQubitGate::Cx,
        qubit(&expected, 0),
        qubit(&expected, 1),
    ))?;
    expected.push_gate(QasmGate::One(OneQubitGate::H, qubit(&expected, 0)))?;
    // Bell measurement: msg -> c[0], alice -> c[1].
    expected.push_measure(qubit(&expected, 0), bit(&expected, 0))?;
    expected.push_measure(qubit(&expected, 1), bit(&expected, 1))?;
    // Feed-forward corrections on bob: X if Alice's bit, Z if the message's.
    expected.push_if(
        Expr::bit_is_set(bit(&expected, 1)),
        vec![Stmt::Gate(QasmGate::One(
            OneQubitGate::X,
            qubit(&expected, 2),
        ))],
        vec![],
    )?;
    expected.push_if(
        Expr::bit_is_set(bit(&expected, 0)),
        vec![Stmt::Gate(QasmGate::One(
            OneQubitGate::Z,
            qubit(&expected, 2),
        ))],
        vec![],
    )?;
    expected.push_measure(qubit(&expected, 2), bit(&expected, 2))?;

    assert_eq!(reify(TELEPORT), expected);
    Ok(())
}

const BERNSTEIN_VAZIRANI: &str = r#"
fn bv_oracle_s110(): Circuit<4, 4, 10, Clifford> = circuit {
    X @3
    |> H @0 |> H @1 |> H @2 |> H @3
    |> CNOT @(0, 3) |> CNOT @(1, 3)
    |> H @0 |> H @1 |> H @2
}

fn main(): Q<(Bit, Bit, Bit, Bit)> = run {
    (q0, q1, q2, anc) <- bv_oracle_s110() @ qreg(4)
    b0  <- measure(q0)
    b1  <- measure(q1)
    b2  <- measure(q2)
    anc_bit <- measure(anc)
    return (b0, b1, b2, anc_bit)
}
"#;

#[test]
fn bernstein_vazirani_reifies_to_expected_tree() -> Result<(), QasmError> {
    let one = |g, q| Stmt::Gate(QasmGate::One(g, q));
    let cx = |a, b| Stmt::Gate(QasmGate::Two(TwoQubitGate::Cx, a, b));

    let mut expected = Program::new(4, 4);
    expected.extend(vec![
        one(OneQubitGate::X, qubit(&expected, 3)),
        one(OneQubitGate::H, qubit(&expected, 0)),
        one(OneQubitGate::H, qubit(&expected, 1)),
        one(OneQubitGate::H, qubit(&expected, 2)),
        one(OneQubitGate::H, qubit(&expected, 3)),
        // Oracle for secret 110: CNOT(q0, anc), CNOT(q1, anc).
        cx(qubit(&expected, 0), qubit(&expected, 3)),
        cx(qubit(&expected, 1), qubit(&expected, 3)),
        one(OneQubitGate::H, qubit(&expected, 0)),
        one(OneQubitGate::H, qubit(&expected, 1)),
        one(OneQubitGate::H, qubit(&expected, 2)),
    ])?;
    expected.push_measure(qubit(&expected, 0), bit(&expected, 0))?;
    expected.push_measure(qubit(&expected, 1), bit(&expected, 1))?;
    expected.push_measure(qubit(&expected, 2), bit(&expected, 2))?;
    expected.push_measure(qubit(&expected, 3), bit(&expected, 3))?;

    assert_eq!(reify(BERNSTEIN_VAZIRANI), expected);
    Ok(())
}
