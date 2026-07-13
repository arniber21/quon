//! Unit tests for LSP document formatting via embedded quonfmt.

use tower_lsp::lsp_types::Position;

#[test]
fn format_document_matches_quonfmt() {
    let src = "fn f():Int=1\n";
    let edits = quon_lsp::format::format_document(src).expect("edits");
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].range.start, Position::new(0, 0));
    assert_eq!(
        edits[0].new_text,
        quonfmt::format_str(src).expect("quonfmt")
    );
}

#[test]
fn format_document_noop_when_clean() {
    let src = "fn f(): Int = 1\n";
    let formatted = quonfmt::format_str(src).expect("format");
    assert!(quon_lsp::format::format_document(&formatted).is_none());
}

#[test]
fn format_document_skips_unparseable() {
    assert!(quon_lsp::format::format_document("fn (").is_none());
}
