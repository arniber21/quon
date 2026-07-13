mod support;

use support::fixture::hover_markdown;

#[test]
fn hover_gate_shows_circuit() {
    let src = r#"
fn bell(): Circuit<2, 2, 1, Clifford> = circuit {
  H/*cursor*/ @ 0
  CNOT @ (0, 1)
}
"#;
    let md = hover_markdown(src).expect("hover");
    assert!(md.contains("Circuit"), "expected Circuit in hover: {md}");
}

#[test]
fn hover_local_shows_inferred_type() {
    let src = "fn f(): Int = let x = 1 in /*cursor*/x\n";
    let md = hover_markdown(src).expect("hover");
    assert!(md.contains("Int"), "hover should include Int: {md}");
}

#[test]
fn hover_fn_use_shows_leading_docs() {
    let src = r#"
-- Prepare a Bell pair on two qubits
fn bell_state(): Int = 1

fn use_bell(): Int = /*cursor*/bell_state()
"#;
    let md = hover_markdown(src).expect("hover");
    assert!(
        md.contains("Prepare a Bell pair on two qubits"),
        "expected docs in hover: {md}"
    );
    assert!(md.contains("Int"), "expected type in hover: {md}");
}
