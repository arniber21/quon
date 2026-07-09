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
fn completion_excludes_other_fn_bindings() {
    let src = "fn f(): Int = let x = 1 in x\nfn g(): Int = /*cursor*/0\n";
    let labels = completion_labels(src);
    assert!(
        !labels.iter().any(|l| l == "x"),
        "x out of scope: {labels:?}"
    );
}
