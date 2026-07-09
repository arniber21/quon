use line_index::{LineCol, LineIndex as RawLineIndex, TextSize, WideEncoding, WideLineCol};
use tower_lsp::lsp_types::Position;

/// UTF-16-aware line/column indexing for LSP position ↔ byte offset mapping.
#[derive(Debug, Clone)]
pub struct LineIndex {
    inner: RawLineIndex,
}

impl LineIndex {
    pub fn new(text: &str) -> Self {
        Self {
            inner: RawLineIndex::new(text),
        }
    }

    /// LSP Position → byte offset (for incremental edit application).
    pub fn offset(&self, pos: Position) -> usize {
        let wide = WideLineCol {
            line: pos.line,
            col: pos.character,
        };
        let utf8 = self
            .inner
            .to_utf8(WideEncoding::Utf16, wide)
            .and_then(|lc| self.inner.offset(lc));
        utf8.map(usize::from).unwrap_or(0)
    }

    /// Byte offset → LSP Position (for diagnostic mapping).
    pub fn position(&self, offset: usize) -> Position {
        let offset = offset.min(u32::MAX as usize) as u32;
        let lc = self
            .inner
            .try_line_col(TextSize::from(offset))
            .unwrap_or(LineCol { line: 0, col: 0 });
        let wide = self
            .inner
            .to_wide(WideEncoding::Utf16, lc)
            .unwrap_or(WideLineCol {
                line: lc.line,
                col: lc.col,
            });
        Position {
            line: wide.line,
            character: wide.col,
        }
    }
}
