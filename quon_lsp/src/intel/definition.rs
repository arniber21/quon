use frontend::analysis::{DocumentAnalysis, ResolvedTarget, resolve_at};
use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Position, Url};

use crate::convert::{position_to_offset, span_to_range};

pub fn definition_at(
    analysis: &DocumentAnalysis,
    uri: &Url,
    position: Position,
) -> Option<GotoDefinitionResponse> {
    let offset = position_to_offset(&analysis.src, position)?;
    let query = resolve_at(analysis, offset)?;
    let span = match &query.target {
        ResolvedTarget::Symbol(id) => analysis.symbols.get(*id)?.name_span,
        ResolvedTarget::TypeAlias(id) => analysis.symbols.get(*id)?.name_span,
        ResolvedTarget::Builtin(_)
        | ResolvedTarget::Gate(_)
        | ResolvedTarget::QuantumBuiltin(_) => {
            return None;
        }
    };
    if span.start == span.end {
        return None;
    }
    Some(GotoDefinitionResponse::Scalar(Location {
        uri: uri.clone(),
        range: span_to_range(&analysis.src, span),
    }))
}
