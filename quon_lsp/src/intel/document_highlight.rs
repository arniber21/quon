use frontend::analysis::{DocumentAnalysis, OccurrenceKind, occurrences_of, resolve_at};
use tower_lsp::lsp_types::{DocumentHighlight, DocumentHighlightKind, Position};

use crate::convert::{position_to_offset, span_to_range};

/// Highlight read/write occurrences of the symbol under `position` in this file.
pub fn document_highlight_at(
    analysis: &DocumentAnalysis,
    position: Position,
) -> Option<Vec<DocumentHighlight>> {
    let offset = position_to_offset(&analysis.src, position)?;
    let query = resolve_at(analysis, offset)?;
    let occs = occurrences_of(analysis, &query.target);
    if occs.is_empty() {
        return None;
    }
    Some(
        occs.into_iter()
            .map(|(span, kind)| DocumentHighlight {
                range: span_to_range(&analysis.src, span),
                kind: Some(match kind {
                    OccurrenceKind::Read => DocumentHighlightKind::READ,
                    OccurrenceKind::Write => DocumentHighlightKind::WRITE,
                }),
            })
            .collect(),
    )
}
