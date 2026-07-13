mod support;

use support::fixture::inlay_hint_labels;

#[test]
fn inlay_shows_inferred_let_type() {
    let src = "fn f(): Int = let x = 1 in x\n";
    let labels = inlay_hint_labels(src);
    assert!(
        labels.iter().any(|l| l.contains("Int")),
        "expected Int inlay on let x, got {labels:?}"
    );
}

#[test]
fn inlay_skips_explicit_param_annotation() {
    let src = "fn f(x: Int): Int = x\n";
    let labels = inlay_hint_labels(src);
    assert!(
        labels.is_empty(),
        "should not duplicate `: Int` on param, got {labels:?}"
    );
}

#[test]
fn inlay_circuit_type_includes_dimensions() {
    let src = "fn f(): Circuit<1, 1, 1, Clifford> = let c = (circuit { H @ 0 } : Circuit<1, 1, 1, Clifford>) in c\n";
    let labels = inlay_hint_labels(src);
    assert!(
        labels
            .iter()
            .any(|l| l.contains("Circuit") && l.contains('1')),
        "expected circuit dimension hint, got {labels:?}"
    );
}
