//! LSP `textDocument/formatting` via embedded [`quonfmt`].
//!
//! # Comment-stripping hazard
//!
//! **`quonfmt` v1 strips line (`--`) and block (`{- -}`) comments.** Formatting a
//! buffer that still contains comments will silently delete them. Prefer an
//! explicit Format Document action over format-on-save until comment preservation
//! lands.
//!
//! # Double-format hazard
//!
//! Editors that already shell out to `quonfmt` (VS Code extension provider,
//! Zed external formatter, Neovim conform.nvim) must **not** also enable LSP
//! document formatting for the same buffer. Applying both yields redundant work
//! and can race on save. Use either the LSP provider **or** the external
//! formatter — never both.

use tower_lsp::lsp_types::{Position, Range, TextEdit};

use crate::span::LineIndex;

/// Format `src` with `quonfmt`. Returns a single full-document [`TextEdit`] when
/// the formatted text differs, `Ok(None)` when already formatted or on parse
/// failure (so the editor keeps the buffer unchanged).
pub fn format_document(src: &str) -> Option<Vec<TextEdit>> {
    let formatted = match quonfmt::format_str(src) {
        Ok(s) => s,
        Err(err) => {
            tracing::debug!(%err, "quonfmt refused document (parse error or not applicable)");
            return None;
        }
    };
    if formatted == src {
        return None;
    }
    Some(vec![TextEdit {
        range: full_document_range(src),
        new_text: formatted,
    }])
}

fn full_document_range(src: &str) -> Range {
    let idx = LineIndex::new(src);
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end: idx.position(src.len()),
    }
}
