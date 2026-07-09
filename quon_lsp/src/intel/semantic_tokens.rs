use frontend::analysis::{DocumentAnalysis, SymbolKind, resolve_at};
use frontend::lexer::{Token, lex};
use tower_lsp::lsp_types::{Position, SemanticToken, SemanticTokens, SemanticTokensResult};

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

pub fn semantic_tokens_legend() -> tower_lsp::lsp_types::SemanticTokensLegend {
    tower_lsp::lsp_types::SemanticTokensLegend {
        token_types: TOKEN_TYPES.iter().map(|s| (*s).into()).collect(),
        token_modifiers: vec![],
    }
}

pub fn semantic_tokens_full(
    analysis: &DocumentAnalysis,
    _position: Position,
) -> Option<SemanticTokensResult> {
    let tokens = lex(&analysis.src).ok()?;
    let mut data = Vec::new();
    let mut prev_line = 0u32;
    let mut prev_start = 0u32;

    for (tok, span) in tokens {
        if matches!(tok, Token::Eof | Token::Newline) {
            continue;
        }
        let token_type = classify_token(&tok, span, analysis);
        let len = (span.end - span.start) as u32;
        let line = line_of_offset(&analysis.src, span.start) as u32;
        let char = char_in_line(&analysis.src, span.start) as u32;

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
            token_modifiers_bitset: 0,
        });
        prev_line = line;
        prev_start = char;
    }

    Some(SemanticTokensResult::Tokens(SemanticTokens {
        data,
        result_id: None,
    }))
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

fn line_of_offset(src: &str, offset: usize) -> usize {
    src[..offset.min(src.len())].matches('\n').count()
}

fn char_in_line(src: &str, offset: usize) -> usize {
    let o = offset.min(src.len());
    src[..o].rfind('\n').map(|i| o - i - 1).unwrap_or(o)
}
