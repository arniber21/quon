mod support;

use support::fixture::completion_labels;

#[test]
fn circuit_block_lists_gates() {
    let src = r#"
fn c(): Circuit<1, 1, 1, Clifford> = circuit {
  /*cursor*/
}
"#;
    let labels = completion_labels(src);
    assert!(labels.iter().any(|l| l == "H"), "labels: {labels:?}");
    assert!(labels.iter().any(|l| l == "CNOT"), "labels: {labels:?}");
}

#[test]
fn type_position_lists_qubit() {
    let src = "fn f(x: /*cursor*/): Int = x\n";
    let labels = completion_labels(src);
    assert!(labels.iter().any(|l| l == "Qubit"), "labels: {labels:?}");
}
