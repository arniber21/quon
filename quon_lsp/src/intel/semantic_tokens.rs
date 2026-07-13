//! Semantic tokens (`full` + `range`) with a small modifier legend.
//!
//! # Token modifiers
//!
//! Legend order (bitset indices):
//! - `0` — `definition`: identifier at its binding / declaration name span
//! - `1` — `readonly`: classical / reusable names (non-linear bindings, gates,
//!   builtins, functions). Intentionally **absent** on linear resources
//!   (`Qubit`, `QReg`, …) so editors can treat missing `readonly` as a
//!   linearity cue without inventing a custom modifier.

use frontend::analysis::{DocumentAnalysis, SymbolKind, resolve_at};
use frontend::lexer::{Token, lex};
use tower_lsp::lsp_types::{
    Position, Range, SemanticToken, SemanticTokens, SemanticTokensRangeResult, SemanticTokensResult,
};

const TOKEN_TYPES: &[&str] = &[
    "keyword",
    "type",
    "function",
    "variable",
    "parameter",
    "number",
    "operator",
    "namespace",
];

const TOKEN_MODIFIERS: &[&str] = &["definition", "readonly"];

const MOD_DEFINITION: u32 = 1 << 0;
const MOD_READONLY: u32 = 1 << 1;

pub fn semantic_tokens_legend() -> tower_lsp::lsp_types::SemanticTokensLegend {
    tower_lsp::lsp_types::SemanticTokensLegend {
        token_types: TOKEN_TYPES.iter().map(|s| (*s).into()).collect(),
        token_modifiers: TOKEN_MODIFIERS.iter().map(|s| (*s).into()).collect(),
    }
}

pub fn semantic_tokens_full(
    analysis: &DocumentAnalysis,
    _position: Position,
) -> Option<SemanticTokensResult> {
    Some(SemanticTokensResult::Tokens(encode_tokens(analysis, None)?))
}

pub fn semantic_tokens_range(
    analysis: &DocumentAnalysis,
    range: Range,
) -> Option<SemanticTokensRangeResult> {
    Some(SemanticTokensRangeResult::Tokens(encode_tokens(
        analysis,
        Some(range),
    )?))
}

fn encode_tokens(analysis: &DocumentAnalysis, range: Option<Range>) -> Option<SemanticTokens> {
    let tokens = lex(&analysis.src).ok()?;
    let mut data = Vec::new();
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;

    for (tok, span) in tokens {
        if matches!(tok, Token::Eof | Token::Newline) {
            continue;
        }
        let line = line_of_offset(&analysis.src, span.start) as u32;
        let char = char_in_line(&analysis.src, span.start) as u32;
        let len = (span.end - span.start) as u32;

        if let Some(r) = range.as_ref()
            && !token_intersects_range(line, char, len, r)
        {
            continue;
        }

        let token_type = classify_token(&tok, span, analysis);
        let token_modifiers_bitset = modifiers_for(&tok, span, analysis);

        let delta_line = line.saturating_sub(prev_line);
        let delta_start = if delta_line == 0 {
            char.saturating_sub(prev_start)
        } else {
            char
        };

        data.push(SemanticToken {
            delta_line,
            delta_start,
            length: len,
            token_type,
            token_modifiers_bitset,
        });
        prev_line = line;
        prev_start = char;
    }

    Some(SemanticTokens {
        data,
        result_id: None,
    })
}

fn token_intersects_range(line: u32, start_char: u32, length: u32, range: &Range) -> bool {
    let end_char = start_char.saturating_add(length);
    let token_start = (line, start_char);
    let token_end = (line, end_char);
    let range_start = (range.start.line, range.start.character);
    let range_end = (range.end.line, range.end.character);
    token_start < range_end && token_end > range_start
}

fn classify_token(
    tok: &Token,
    span: frontend::lexer::SimpleSpan,
    analysis: &DocumentAnalysis,
) -> u32 {
    match tok {
        Token::Fn
        | Token::Type
        | Token::Let
        | Token::In
        | Token::Return
        | Token::Match
        | Token::Circuit
        | Token::Run
        | Token::Borrow
        | Token::For
        | Token::If
        | Token::Then
        | Token::Else
        | Token::True
        | Token::False
        | Token::Adjoint
        | Token::Controlled
        | Token::Par => 0,
        Token::Int(_) | Token::Float(_) => 5,
        Token::Pipe
        | Token::Bind
        | Token::At
        | Token::Arrow
        | Token::LinearArrow
        | Token::Plus
        | Token::Minus
        | Token::Star
        | Token::Slash
        | Token::Caret
        | Token::Eq
        | Token::FatArrow
        | Token::Colon
        | Token::Comma
        | Token::Dot
        | Token::Underscore
        | Token::Backtick
        | Token::Bar => 6,
        Token::Ident(name) => {
            // Prefer def-site symbol lookup: `resolve_at` often misses binding names.
            if let Some(id) = analysis.symbols.by_def_span(span)
                && let Some(kind) = analysis.symbols.get(id).map(|s| s.kind)
            {
                return match kind {
                    SymbolKind::Function | SymbolKind::Gate | SymbolKind::Builtin => 2,
                    SymbolKind::Parameter => 4,
                    SymbolKind::TypeAlias | SymbolKind::TypeParam => 1,
                    _ => 3,
                };
            }
            if let Some(q) = resolve_at(analysis, span.start) {
                return match q.target {
                    frontend::analysis::ResolvedTarget::Gate(_) => 2,
                    frontend::analysis::ResolvedTarget::Builtin(_)
                    | frontend::analysis::ResolvedTarget::QuantumBuiltin(_) => 2,
                    frontend::analysis::ResolvedTarget::TypeAlias(_) => 7,
                    frontend::analysis::ResolvedTarget::Symbol(id) => {
                        match analysis.symbols.get(id).map(|s| s.kind) {
                            Some(SymbolKind::Function | SymbolKind::Gate | SymbolKind::Builtin) => {
                                2
                            }
                            Some(SymbolKind::Parameter) => 4,
                            Some(SymbolKind::TypeAlias | SymbolKind::TypeParam) => 1,
                            _ => 3,
                        }
                    }
                };
            }
            if matches!(
                name.as_str(),
                "Qubit"
                    | "Bit"
                    | "Bool"
                    | "Int"
                    | "Float"
                    | "Unit"
                    | "Nat"
                    | "List"
                    | "Matrix"
                    | "Circuit"
                    | "Q"
                    | "QReg"
            ) {
                1
            } else {
                3
            }
        }
        _ => 3,
    }
}

fn modifiers_for(
    tok: &Token,
    span: frontend::lexer::SimpleSpan,
    analysis: &DocumentAnalysis,
) -> u32 {
    let Token::Ident(_) = tok else {
        return 0;
    };

    let mut bits = 0u32;
    if analysis.symbols.by_def_span(span).is_some() {
        bits |= MOD_DEFINITION;
    }

    if is_readonly_ident(span, analysis) {
        bits |= MOD_READONLY;
    }
    bits
}

/// Classical / reusable names get `readonly`. Linear resources deliberately do not.
fn is_readonly_ident(span: frontend::lexer::SimpleSpan, analysis: &DocumentAnalysis) -> bool {
    if let Some(q) = resolve_at(analysis, span.start) {
        return match q.target {
            frontend::analysis::ResolvedTarget::Gate(_)
            | frontend::analysis::ResolvedTarget::Builtin(_)
            | frontend::analysis::ResolvedTarget::QuantumBuiltin(_) => true,
            frontend::analysis::ResolvedTarget::TypeAlias(_) => true,
            frontend::analysis::ResolvedTarget::Symbol(id) => {
                symbol_is_readonly(analysis, id, span)
            }
        };
    }
    if let Some(id) = analysis.symbols.by_def_span(span) {
        return symbol_is_readonly(analysis, id, span);
    }
    false
}

fn symbol_is_readonly(
    analysis: &DocumentAnalysis,
    id: frontend::analysis::SymbolId,
    span: frontend::lexer::SimpleSpan,
) -> bool {
    let Some(sym) = analysis.symbols.get(id) else {
        return false;
    };
    if matches!(
        sym.kind,
        SymbolKind::Gate | SymbolKind::Builtin | SymbolKind::QuantumBuiltin
    ) {
        return true;
    }
    if sym.kind == SymbolKind::LinearBinding {
        return false;
    }
    match ty_for_symbol(analysis, id, span) {
        Some(t) if t.is_linear_resource() => false,
        Some(_) => true,
        // Untyped classical-looking bindings still count as reusable.
        None => matches!(
            sym.kind,
            SymbolKind::Function
                | SymbolKind::Parameter
                | SymbolKind::LocalBinding
                | SymbolKind::TypeAlias
                | SymbolKind::TypeParam
        ),
    }
}

fn ty_for_symbol(
    analysis: &DocumentAnalysis,
    id: frontend::analysis::SymbolId,
    span: frontend::lexer::SimpleSpan,
) -> Option<&frontend::types::Ty> {
    let sym = analysis.symbols.get(id)?;
    if let Some(ty) = analysis
        .annotations
        .get(span)
        .or_else(|| analysis.annotations.get(sym.name_span))
        .or(sym.ty.as_ref())
    {
        return Some(ty);
    }
    // Def-site params often lack annotations on the name span; reuse a use-site type.
    for (use_span, target) in analysis.resolutions.entries() {
        if matches!(target, frontend::analysis::ResolvedTarget::Symbol(sid) if *sid == id)
            && let Some(ty) = analysis.annotations.get(use_span)
        {
            return Some(ty);
        }
    }
    None
}

fn line_of_offset(src: &str, offset: usize) -> usize {
    src[..offset.min(src.len())].matches('\n').count()
}

fn char_in_line(src: &str, offset: usize) -> usize {
    let o = offset.min(src.len());
    src[..o].rfind('\n').map(|i| o - i - 1).unwrap_or(o)
}
