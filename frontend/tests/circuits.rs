// Circuit and Clifford-classification integration tests (issues #11, #12), driving the
// public `frontend::check_program` facade. Focus: end-to-end acceptance of the composition
// rules and span-accurate Clifford annotation errors.

use frontend::check_program;
use frontend::diagnostics::Diagnostic;

fn only_error(src: &str) -> Diagnostic {
    let mut es = check_program(src).expect_err("expected the program to be rejected");
    assert_eq!(es.len(), 1, "expected exactly one diagnostic, got {es:?}");
    es.pop().unwrap()
}

#[test]
fn bell_gate_type_checks_end_to_end() {
    let src = "fn bell(): Circuit<2, 2, 2, Clifford> = circuit { H @0 |> CNOT @(0, 1) }";
    assert!(
        check_program(src).is_ok(),
        "got {:?}",
        check_program(src).err()
    );
}

#[test]
fn symbolic_fold_depth_type_checks_end_to_end() {
    // `ising`-shaped: a fold over a circuit accumulator yields the symbolic depth `n_steps*n`.
    let src = "fn trotter(n: Nat): Circuit<n, n, 1, Universal> = circuit { for q in qubits(n) { T q } }\n\
               fn ising(n: Nat, n_steps: Int): Circuit<n, n, n_steps, Universal> =\n\
               fold(range(n_steps), identity(n), fn(acc, _) -> acc |> trotter(n))";
    assert!(
        check_program(src).is_ok(),
        "got {:?}",
        check_program(src).err()
    );
}

#[test]
fn a_t_gate_annotated_clifford_is_rejected() {
    // Acceptance criterion (#12): a Universal circuit annotated `Clifford` is rejected with a
    // span-accurate error.
    let src = "fn f(): Circuit<1, 1, 1, Clifford> = circuit { T @0 }";
    let diag = only_error(src);
    assert!(
        diag.message.contains("Clifford classification mismatch"),
        "message was: {}",
        diag.message
    );
}

#[test]
fn composition_qubit_mismatch_is_reported() {
    let src = "fn f(): Circuit<2, 3, 0, Clifford> = identity(2) |> identity(3)";
    let diag = only_error(src);
    assert!(
        diag.message.contains("matching qubit counts"),
        "message was: {}",
        diag.message
    );
}
