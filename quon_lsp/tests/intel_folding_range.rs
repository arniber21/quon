mod support;

use support::fixture::folding_range_count;

#[test]
fn folds_multiline_circuit_block() {
    let src = r#"
fn bell(): Circuit<2, 2, 1, Clifford> = circuit {
  H @ 0
  CNOT @ (0, 1)
}
"#;
    let n = folding_range_count(src);
    assert!(n >= 1, "expected at least one fold for circuit/fn, got {n}");
}

#[test]
fn folds_match_borrow_and_run() {
    let src = r#"
fn qft(n: Nat): Circuit<n, n, 1, Clifford> = match n {
  0 => circuit { H @ 0 }
  _ => circuit { H @ 0 }
}

fn f(): Q<Int> = run {
  borrow a: Qubit in {
    return 0
  }
}
"#;
    let n = folding_range_count(src);
    assert!(
        n >= 2,
        "expected folds for match/run/borrow regions, got {n}"
    );
}

#[test]
fn folds_for_loop_block() {
    let src = r#"
fn hadamard_all(n: Nat): Circuit<n, n, 1, Clifford> =
  for q in qubits(n) {
    H @ q
  }
"#;
    let n = folding_range_count(src);
    assert!(n >= 1, "expected fold for for-loop, got {n}");
}

#[test]
fn single_line_fn_has_no_fold() {
    let src = "fn f(): Int = 1\n";
    assert_eq!(folding_range_count(src), 0);
}
