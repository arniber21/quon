use frontend::lexer::SimpleSpan;
use tower_lsp::lsp_types::{Position, Range};

use crate::span::LineIndex;

pub fn offset_to_position(src: &str, offset: usize) -> Position {
    LineIndex::new(src).position(offset)
}

pub fn position_to_offset(src: &str, pos: Position) -> Option<usize> {
    let off = LineIndex::new(src).offset(pos)?;
    if off <= src.len() { Some(off) } else { None }
}

pub fn span_to_range(src: &str, span: SimpleSpan) -> Range {
    let idx = LineIndex::new(src);
    Range {
        start: idx.position(span.start),
        end: idx.position(span.end),
    }
}
