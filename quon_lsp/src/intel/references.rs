use frontend::analysis::{DocumentAnalysis, OccurrenceKind, occurrences_of, resolve_at};
use tower_lsp::lsp_types::{Location, Position, Url};

use crate::convert::{position_to_offset, span_to_range};

/// In-file find-all-references for the symbol under `position`.
///
/// Builtins / gates return `None`. When `include_declaration` is false, the
/// definition (write) span is omitted.
pub fn references_at(
    analysis: &DocumentAnalysis,
    uri: &Url,
    position: Position,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    let offset = position_to_offset(&analysis.src, position)?;
    let query = resolve_at(analysis, offset)?;
    let occs = occurrences_of(analysis, &query.target);
    if occs.is_empty() {
        return None;
    }
    let locs: Vec<Location> = occs
        .into_iter()
        .filter(|(_, kind)| include_declaration || *kind == OccurrenceKind::Read)
        .map(|(span, _)| Location {
            uri: uri.clone(),
            range: span_to_range(&analysis.src, span),
        })
        .collect();
    if locs.is_empty() { None } else { Some(locs) }
}
