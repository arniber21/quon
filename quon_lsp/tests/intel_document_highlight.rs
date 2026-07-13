mod support;

use tower_lsp::lsp_types::DocumentHighlightKind;

use support::fixture::highlights_at_marker;

#[test]
fn highlight_marks_read_and_write() {
    let src = "fn f(): Int = let x = 1 in /*cursor*/x\n";
    let highlights = highlights_at_marker(src).expect("highlights");
    assert_eq!(
        highlights.len(),
        2,
        "expected write + read, got {highlights:?}"
    );
    let kinds: Vec<_> = highlights.iter().map(|h| h.kind).collect();
    assert!(
        kinds.contains(&Some(DocumentHighlightKind::WRITE)),
        "missing WRITE: {kinds:?}"
    );
    assert!(
        kinds.contains(&Some(DocumentHighlightKind::READ)),
        "missing READ: {kinds:?}"
    );
}

#[test]
fn highlight_function_occurrences() {
    let src = "fn helper(): Int = 1\nfn f(): Int = /*cursor*/helper()\n";
    let highlights = highlights_at_marker(src).expect("highlights");
    assert_eq!(
        highlights.len(),
        2,
        "expected def + call, got {highlights:?}"
    );
}

#[test]
fn builtin_has_no_highlight() {
    let src = "fn f(): Int = /*cursor*/range(3)\n";
    assert!(highlights_at_marker(src).is_none());
}
