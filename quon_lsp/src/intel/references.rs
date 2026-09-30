use frontend::analysis::{
    DocumentAnalysis, OccurrenceKind, occurrences_of, resolve_at, target_symbol_id,
};
use tower_lsp::lsp_types::{Location, Position, Url};

use crate::convert::{position_to_offset, span_to_range};
use crate::intel::definition::name_at;
use crate::workspace::{
    WorkspaceIndex, cross_file_occurrences, export_ids, export_target, is_workspace_export,
};

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
    references_in_workspace(
        analysis,
        uri,
        position,
        include_declaration,
        &WorkspaceIndex::default(),
    )
}

/// Find references, including same-directory top-level `fn` / `type` names.
pub fn references_in_workspace(
    analysis: &DocumentAnalysis,
    uri: &Url,
    position: Position,
    include_declaration: bool,
    index: &WorkspaceIndex,
) -> Option<Vec<Location>> {
    let offset = position_to_offset(&analysis.src, position)?;
    let export_name = resolve_at(analysis, offset).and_then(|query| {
        let id = target_symbol_id(&query.target)?;
        let sym = analysis.symbols.get(id)?;
        if is_workspace_export(&analysis.symbols, sym) {
            Some(sym.name.clone())
        } else {
            None
        }
    });
    if export_name.is_none() && resolve_at(analysis, offset).is_some() {
        return in_file_locations(analysis, uri, offset, include_declaration);
    }
    let unresolved = export_name.is_none() && resolve_at(analysis, offset).is_none();
    let name = match export_name {
        Some(name) => name,
        None => name_at(analysis, offset)?.0,
    };
    if index.definition_locations(uri, &name).is_empty() && unresolved {
        return None;
    }
    let mut locs = file_locations(analysis, uri, &name, include_declaration);
    for (other_uri, other) in index.other_files(uri) {
        locs.extend(file_locations(other, other_uri, &name, include_declaration));
    }
    if locs.is_empty() { None } else { Some(locs) }
}

fn in_file_locations(
    analysis: &DocumentAnalysis,
    uri: &Url,
    offset: usize,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    let query = resolve_at(analysis, offset)?;
    let occs = occurrences_of(analysis, &query.target);
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

fn file_locations(
    analysis: &DocumentAnalysis,
    uri: &Url,
    name: &str,
    include_declaration: bool,
) -> Vec<Location> {
    let occs = if export_ids(analysis, name).is_empty() {
        cross_file_occurrences(analysis, name)
    } else {
        let mut occs = Vec::new();
        for id in export_ids(analysis, name) {
            occs.extend(occurrences_of(analysis, &export_target(analysis, id)));
        }
        occs
    };
    occs.into_iter()
        .filter(|(_, kind)| include_declaration || *kind == OccurrenceKind::Read)
        .map(|(span, _)| Location {
            uri: uri.clone(),
            range: span_to_range(&analysis.src, span),
        })
        .collect()
}
