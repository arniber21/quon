use quonlint::span::LineIndex;

#[test]
fn byte_span_to_line_col() {
    let src = "fn f() = 1\nfn g() = 2\n";
    let idx = LineIndex::new(src);
    let (line, col) = idx.line_col(0);
    assert_eq!(line, 0);
    assert_eq!(col, 0);

    let (line, col) = idx.line_col(src.find('g').unwrap());
    assert_eq!(line, 1);
    assert_eq!(col, 3);
}

#[test]
fn lsp_range_from_span() {
    let src = "abc\ndef\n";
    let idx = LineIndex::new(src);
    let range = idx.range((4..7).into());
    assert_eq!(range.start.line, 1);
    assert_eq!(range.start.character, 0);
}
