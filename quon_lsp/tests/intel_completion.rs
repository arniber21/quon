mod support;

use frontend::analyze;
use quon_lsp::intel::completions_at;
use support::fixture::{completion_labels, position_after_marker, src_without_marker};
use tower_lsp::lsp_types::{CompletionResponse, Documentation};

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
    let clean = src_without_marker(src);
    let pos = position_after_marker(src);
    let result = analyze(&clean);
    assert!(
        result.intelligence.diagnostics.is_empty(),
        "{:?}",
        result.intelligence.diagnostics
    );
    let resp = completions_at(&result.intelligence, pos).expect("completions");
    let CompletionResponse::Array(items) = resp else {
        panic!("expected array");
    };
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
