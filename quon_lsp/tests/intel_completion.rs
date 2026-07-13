mod support;

use support::fixture::{completion_items, completion_labels, signature_help_at_marker};
use tower_lsp::lsp_types::Documentation;

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

#[test]
fn completion_includes_leading_docs() {
    let src = r#"
-- Prepare a Bell pair
fn bell_state(): Int = 1

fn use_bell(): Int = /*cursor*/bell_state()
"#;
    let items = completion_items(src);
    let labels: Vec<_> = items.iter().map(|i| i.label.as_str()).collect();
    let bell = items
        .iter()
        .find(|i| i.label == "bell_state")
        .unwrap_or_else(|| panic!("bell_state completion; labels={labels:?}"));
    match &bell.documentation {
        Some(Documentation::MarkupContent(m)) => {
            assert!(m.value.contains("Prepare a Bell pair"), "docs: {}", m.value);
        }
        other => panic!("expected markup docs, got {other:?}"),
    }
}

#[test]
fn after_at_offers_gates_not_types() {
    let src = r#"
fn c(): Circuit<1, 1, 1, Clifford> = circuit {
  H @/*cursor*/
}
"#;
    let labels = completion_labels(src);
    assert!(labels.iter().any(|l| l == "H"), "gates: {labels:?}");
    assert!(labels.iter().any(|l| l == "CNOT"), "gates: {labels:?}");
    assert!(
        labels.iter().any(|l| l == "apply"),
        "applyables: {labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l == "Qubit"),
        "types should not appear after @: {labels:?}"
    );
    assert!(
        !labels.iter().any(|l| l == "let"),
        "keywords should not appear after @: {labels:?}"
    );
}

#[test]
fn type_position_offers_types_not_gates() {
    let src = "fn f(x: /*cursor*/): Int = 0\n";
    let labels = completion_labels(src);
    assert!(labels.iter().any(|l| l == "Qubit"), "types: {labels:?}");
    assert!(labels.iter().any(|l| l == "Int"), "types: {labels:?}");
    assert!(
        !labels.iter().any(|l| l == "H" || l == "CNOT"),
        "gates should not appear in type position: {labels:?}"
    );
}

#[test]
fn expression_offers_locals_fns_and_builtins() {
    let src = r#"
fn helper(): Int = 1
fn f(): Int = let x = 1 in (/*cursor*/ x)
"#;
    let labels = completion_labels(src);
    assert!(labels.iter().any(|l| l == "x"), "local: {labels:?}");
    assert!(labels.iter().any(|l| l == "helper"), "fn: {labels:?}");
    assert!(
        labels.iter().any(|l| l == "map"),
        "classical builtin: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "measure"),
        "quantum builtin: {labels:?}"
    );
}

#[test]
fn snippets_include_circuit_run_borrow() {
    let src = "/*cursor*/\n";
    let labels = completion_labels(src);
    for snip in ["circuit", "run", "borrow", "fn"] {
        assert!(
            labels.iter().any(|l| l == snip),
            "missing snippet {snip}: {labels:?}"
        );
    }
}

#[test]
fn completion_items_have_detail_or_docs() {
    let src = r#"
fn c(): Circuit<1, 1, 1, Clifford> = circuit {
  /*cursor*/
}
"#;
    let items = completion_items(src);
    let h = items.iter().find(|i| i.label == "H").expect("H completion");
    assert!(
        h.detail.is_some() || h.documentation.is_some(),
        "H should carry detail/docs: {h:?}"
    );
}

#[test]
fn signature_help_at_map_call() {
    let src = "fn f(xs: List<Int>): List<Int> = map(fn(x: Int) -> x, /*cursor*/xs)\n";
    let help = signature_help_at_marker(src).expect("signature help");
    let sig = &help.signatures[0];
    assert!(
        sig.label.contains("map"),
        "label should mention map: {}",
        sig.label
    );
    assert_eq!(help.active_parameter, Some(1));
}

#[test]
fn signature_help_at_gate_app() {
    let src = r#"
fn c(): Circuit<1, 1, 1, Clifford> = circuit {
  H @/*cursor*/0
}
"#;
    let help = signature_help_at_marker(src).expect("signature help");
    let sig = &help.signatures[0];
    assert!(sig.label.contains('H'), "label: {}", sig.label);
    assert!(sig.label.contains('@'), "label: {}", sig.label);
}
