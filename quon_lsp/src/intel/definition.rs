use frontend::analysis::{DocumentAnalysis, NodeAt, ResolvedTarget, node_at_offset, resolve_at};
use frontend::lexer::SimpleSpan;
use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Position, Url};

use crate::convert::{position_to_offset, span_to_range};
use crate::workspace::{WorkspaceIndex, is_workspace_export};

pub fn definition_at(
    analysis: &DocumentAnalysis,
    uri: &Url,
    position: Position,
) -> Option<GotoDefinitionResponse> {
    definition_in_workspace(analysis, uri, position, &WorkspaceIndex::default())
}

/// Go-to-definition, including same-directory top-level `fn` / `type` names.
pub fn definition_in_workspace(
    analysis: &DocumentAnalysis,
    uri: &Url,
    position: Position,
    index: &WorkspaceIndex,
) -> Option<GotoDefinitionResponse> {
    let offset = position_to_offset(&analysis.src, position)?;
    if let Some(query) = resolve_at(analysis, offset) {
        match &query.target {
            ResolvedTarget::Symbol(id) | ResolvedTarget::TypeAlias(id) => {
                let sym = analysis.symbols.get(*id)?;
                let name_span = sym.name_span;
                if is_workspace_export(&analysis.symbols, sym) {
                    let locs = index.definition_locations(uri, &sym.name);
                    if let Some(response) = response_from(locs) {
                        return Some(response);
                    }
                    return scalar_location(uri, &analysis.src, name_span);
                }
                if name_span.start == name_span.end {
                    return None;
                }
                scalar_location(uri, &analysis.src, name_span)
            }
            ResolvedTarget::Builtin(_)
            | ResolvedTarget::Gate(_)
            | ResolvedTarget::QuantumBuiltin(_) => None,
        }
    } else {
        let (name, _) = name_at(analysis, offset)?;
        response_from(index.definition_locations(uri, &name))
    }
}

fn response_from(locs: Vec<Location>) -> Option<GotoDefinitionResponse> {
    match locs.len() {
        0 => None,
        1 => locs.into_iter().next().map(GotoDefinitionResponse::Scalar),
        _ => Some(GotoDefinitionResponse::Array(locs)),
    }
}

fn scalar_location(uri: &Url, src: &str, span: SimpleSpan) -> Option<GotoDefinitionResponse> {
    if span.start == span.end {
        return None;
    }
    Some(GotoDefinitionResponse::Scalar(Location {
        uri: uri.clone(),
        range: span_to_range(src, span),
    }))
}

pub fn name_at(analysis: &DocumentAnalysis, offset: usize) -> Option<(String, SimpleSpan)> {
    match node_at_offset(&analysis.decls, offset)? {
        NodeAt::Name(name, span) => Some((name.to_string(), span)),
        _ => None,
    }
}
