use frontend::analysis::{DocumentAnalysis, SymbolKind};
use frontend::types::Ty;
use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, Position, Range};

use crate::convert::offset_to_position;

/// Inferred-type inlays on `let` / notable bindings; circuit types already include dimensions.
pub fn inlay_hints(analysis: &DocumentAnalysis, range: Range) -> Option<Vec<InlayHint>> {
    let mut hints = Vec::new();
    for sym in &analysis.symbols.symbols {
        if !matches!(
            sym.kind,
            SymbolKind::LocalBinding | SymbolKind::LinearBinding | SymbolKind::Parameter
        ) {
            continue;
        }
        if sym.name_span.start == sym.name_span.end {
            continue;
        }
        if sym.name.starts_with('$') || sym.name == "_" {
            continue;
        }
        let Some(ty) = analysis.annotations.get(sym.name_span).or(sym.ty.as_ref()) else {
            continue;
        };
        if !ty_ready_for_hint(ty) {
            continue;
        }
        let hint_pos = offset_to_position(&analysis.src, sym.name_span.end);
        if !position_in_range(hint_pos, range) {
            continue;
        }
        // Avoid duplicating an already-written `: Type` annotation after the name.
        if has_explicit_type_annotation(&analysis.src, sym.name_span.end) {
            continue;
        }
        hints.push(InlayHint {
            position: hint_pos,
            label: InlayHintLabel::String(format!(": {ty}")),
            kind: Some(InlayHintKind::TYPE),
            text_edits: None,
            tooltip: circuit_tooltip(ty),
            padding_left: Some(false),
            padding_right: Some(false),
            data: None,
        });
    }
    hints.sort_by_key(|h| (h.position.line, h.position.character));
    if hints.is_empty() { None } else { Some(hints) }
}

fn ty_ready_for_hint(ty: &Ty) -> bool {
    !matches!(ty, Ty::Meta(_)) && !ty.to_string().contains('?')
}

fn has_explicit_type_annotation(src: &str, after_name: usize) -> bool {
    src.get(after_name..)
        .map(|rest| rest.trim_start().starts_with(':'))
        .unwrap_or(false)
}

fn circuit_tooltip(ty: &Ty) -> Option<tower_lsp::lsp_types::InlayHintTooltip> {
    match ty {
        Ty::Circuit { .. } => Some(tower_lsp::lsp_types::InlayHintTooltip::String(
            ty.to_string(),
        )),
        _ => None,
    }
}

fn position_in_range(pos: Position, range: Range) -> bool {
    if pos.line < range.start.line || pos.line > range.end.line {
        return false;
    }
    if pos.line == range.start.line && pos.character < range.start.character {
        return false;
    }
    if pos.line == range.end.line && pos.character > range.end.character {
        return false;
    }
    true
}

/// Full-document range helper for tests.
pub fn full_document_range(src: &str) -> Range {
    let end = offset_to_position(src, src.len());
    Range {
        start: Position {
            line: 0,
            character: 0,
        },
        end,
    }
}
