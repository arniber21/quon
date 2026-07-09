use frontend::analysis::{DocumentAnalysis, format_hover, resolve_at};
use tower_lsp::lsp_types::{Hover, HoverContents, MarkupContent, MarkupKind, Position};

use crate::convert::position_to_offset;

pub fn hover_at(analysis: &DocumentAnalysis, position: Position) -> Option<Hover> {
    let offset = position_to_offset(&analysis.src, position)?;
    let query = resolve_at(analysis, offset)?;
    let markdown = format_hover(&query, analysis);
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: markdown,
        }),
        range: None,
    })
}
