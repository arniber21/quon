use quon_lsp::{LineIndex, diagnostic_to_lsp};
use tower_lsp::lsp_types::Diagnostic as LspDiagnostic;

fn assert_lsp_span_on(src: &str, needle: &str, diag: &LspDiagnostic) {
    let start = src.find(needle).expect("needle in source");
    let end = start + needle.len();
    let idx = LineIndex::new(src);
    let expected_start = idx.position(start);
    let expected_end = idx.position(end);
    assert_eq!(diag.range.start, expected_start);
    assert_eq!(diag.range.end, expected_end);
}

#[test]
fn ascii_byte_offset_maps_to_lsp_position() {
    let src = "fn f(x: Int): Int = x + y\n";
    let err = frontend::check_program(src).unwrap_err();
    let y_start = src.find('y').expect("y in source");
    let lsp = diagnostic_to_lsp(&err[0], src, &LineIndex::new(src));
    assert_eq!(lsp.range.start.line, 0);
    assert_eq!(lsp.range.start.character, y_start as u32);
    assert_lsp_span_on(src, "y", &lsp);
}

#[test]
fn ghost_unbound_variable_span() {
    let src = "fn f(): Int = ghost";
    let err = frontend::check_program(src).unwrap_err();
    assert!(err[0].message.contains("unbound variable"));
    let ghost_start = src.find("ghost").expect("ghost in source");
    let lsp = diagnostic_to_lsp(&err[0], src, &LineIndex::new(src));
    assert_eq!(lsp.range.start.character, ghost_start as u32);
    assert_lsp_span_on(src, "ghost", &lsp);
}

#[test]
fn unicode_in_comment_does_not_shift_ascii_diagnostic() {
    let src = "fn f(): Int = ghost -- café\n";
    let err = frontend::check_program(src).unwrap_err();
    let ghost_start = src.find("ghost").expect("ghost in source");
    let lsp = diagnostic_to_lsp(&err[0], src, &LineIndex::new(src));
    assert_eq!(lsp.range.start.character, ghost_start as u32);
    assert_lsp_span_on(src, "ghost", &lsp);
}

#[test]
fn unicode_in_string_literal_column_is_utf16() {
    let src = r#"fn f(): Int = "é" + ghost"#;
    let idx = LineIndex::new(src);
    let ghost_pos = src.find("ghost").expect("ghost");
    let pos = idx.position(ghost_pos);
    assert_eq!(pos.line, 0);
    // `"é"` is two UTF-8 bytes but one UTF-16 code unit, so the UTF-16 column is
    // smaller than the byte offset of `ghost`.
    assert!(pos.character < ghost_pos as u32);
}

#[test]
fn unicode_in_string_literal_diagnostic_span() {
    let src = r#"fn f(): Int = "é" + ghost"#;
    let err = frontend::check_program(src).unwrap_err();
    let idx = LineIndex::new(src);
    let lsp = diagnostic_to_lsp(&err[0], src, &idx);
    let span = err[0].span;
    assert_eq!(lsp.range.start, idx.position(span.start));
    assert_eq!(lsp.range.end, idx.position(span.end));
    let ghost_byte = src.find("ghost").expect("ghost");
    assert!(idx.position(ghost_byte).character < ghost_byte as u32);
}

#[test]
fn crlf_line_endings_map_correctly() {
    let src = "fn f(): Int = ghost\r\n";
    let err = frontend::check_program(src).unwrap_err();
    let ghost_start = src.find("ghost").expect("ghost");
    let lsp = diagnostic_to_lsp(&err[0], src, &LineIndex::new(src));
    assert_eq!(lsp.range.start.line, 0);
    assert_eq!(lsp.range.start.character, ghost_start as u32);
    assert_lsp_span_on(src, "ghost", &lsp);
}

#[test]
fn tab_counts_as_one_utf16_code_unit() {
    let src = "fn f(): Int = \tghost";
    let err = frontend::check_program(src).unwrap_err();
    let ghost_start = src.find("ghost").expect("ghost");
    let lsp = diagnostic_to_lsp(&err[0], src, &LineIndex::new(src));
    assert_eq!(lsp.range.start.character, ghost_start as u32);
    assert_lsp_span_on(src, "ghost", &lsp);
}

#[test]
fn position_offset_round_trip_ascii() {
    let src = "fn f(x: Int): Int = x + y\n";
    let idx = LineIndex::new(src);
    let offset = src.find('y').expect("y");
    let pos = idx.position(offset);
    assert_eq!(idx.offset(pos), Some(offset));
}

#[test]
fn invalid_position_returns_none() {
    let src = "fn f(): Int = 1";
    let idx = LineIndex::new(src);
    assert!(
        idx.offset(tower_lsp::lsp_types::Position {
            line: 99,
            character: 0,
        })
        .is_none()
    );
}

#[test]
fn empty_span_maps_to_start_of_file() {
    let src = "fn f(): Int = 1";
    let idx = LineIndex::new(src);
    let pos = idx.position(0);
    assert_eq!(pos.line, 0);
    assert_eq!(pos.character, 0);
}

#[test]
fn span_past_eof_is_clamped() {
    let src = "fn f(): Int = 1";
    let idx = LineIndex::new(src);
    let pos = idx.position(src.len() + 100);
    assert_eq!(pos.line, 0);
}
