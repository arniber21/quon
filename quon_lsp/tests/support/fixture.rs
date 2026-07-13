use frontend::analysis::cursor_at;
use frontend::analyze;
use tower_lsp::lsp_types::{
    CompletionItem, DocumentSymbolResponse, Position, Range, SignatureHelp, Url,
};

use quon_lsp::intel::{
    completions_at, definition_at, document_highlight_at, document_symbols, folding_ranges,
    full_document_range, hover_at, inlay_hints, prepare_rename_at, references_at, rename_at,
    semantic_tokens_full, signature_help_at,
};

fn fixture_url() -> Url {
    Url::parse("file:///test.qn").expect("url")
}

fn analyze_fixture(src: &str) -> frontend::AnalysisResult {
    analyze(src)
}

pub fn position_after_marker(src: &str) -> Position {
    let offset = cursor_at(src, "/*cursor*/");
    let before = src[..offset].replace("/*cursor*/", "");
    let line = before.matches('\n').count() as u32;
    let col = before
        .rfind('\n')
        .map(|i| (before.len() - i - 1) as u32)
        .unwrap_or(before.len() as u32);
    Position {
        line,
        character: col,
    }
}

pub fn src_without_marker(src: &str) -> String {
    src.replace("/*cursor*/", "")
}

pub fn hover_markdown(src: &str) -> Option<String> {
    let clean = src_without_marker(src);
    let pos = position_after_marker(src);
    let result = analyze_fixture(&clean);
    let hover = hover_at(&result.intelligence, pos)?;
    match hover.contents {
        tower_lsp::lsp_types::HoverContents::Markup(m) => Some(m.value),
        _ => None,
    }
}

pub fn definition_at_marker(src: &str) -> Option<tower_lsp::lsp_types::Range> {
    let clean = src_without_marker(src);
    let pos = position_after_marker(src);
    let result = analyze_fixture(&clean);
    let uri = fixture_url();
    let resp = definition_at(&result.intelligence, &uri, pos)?;
    match resp {
        tower_lsp::lsp_types::GotoDefinitionResponse::Scalar(loc) => Some(loc.range),
        _ => None,
    }
}

pub fn references_at_marker(
    src: &str,
    include_declaration: bool,
) -> Option<Vec<tower_lsp::lsp_types::Location>> {
    let clean = src_without_marker(src);
    let pos = position_after_marker(src);
    let result = analyze_fixture(&clean);
    let uri = fixture_url();
    references_at(&result.intelligence, &uri, pos, include_declaration)
}

pub fn highlights_at_marker(src: &str) -> Option<Vec<tower_lsp::lsp_types::DocumentHighlight>> {
    let clean = src_without_marker(src);
    let pos = position_after_marker(src);
    let result = analyze_fixture(&clean);
    document_highlight_at(&result.intelligence, pos)
}

pub fn prepare_rename_at_marker(
    src: &str,
) -> tower_lsp::jsonrpc::Result<Option<tower_lsp::lsp_types::PrepareRenameResponse>> {
    let clean = src_without_marker(src);
    let pos = position_after_marker(src);
    let result = analyze_fixture(&clean);
    prepare_rename_at(&result.intelligence, pos)
}

pub fn rename_at_marker(
    src: &str,
    new_name: &str,
) -> tower_lsp::jsonrpc::Result<Option<tower_lsp::lsp_types::WorkspaceEdit>> {
    let clean = src_without_marker(src);
    let pos = position_after_marker(src);
    let result = analyze_fixture(&clean);
    let uri = fixture_url();
    rename_at(&result.intelligence, &uri, pos, new_name)
}

pub fn completion_items(src: &str) -> Vec<CompletionItem> {
    let clean = src_without_marker(src);
    let pos = position_after_marker(src);
    let result = analyze_fixture(&clean);
    let resp = completions_at(&result.intelligence, pos).expect("completions");
    match resp {
        tower_lsp::lsp_types::CompletionResponse::Array(items) => items,
        _ => vec![],
    }
}

pub fn completion_labels(src: &str) -> Vec<String> {
    completion_items(src).into_iter().map(|i| i.label).collect()
}

pub fn signature_help_at_marker(src: &str) -> Option<SignatureHelp> {
    let clean = src_without_marker(src);
    let pos = position_after_marker(src);
    let result = analyze_fixture(&clean);
    signature_help_at(&result.intelligence, pos)
}

pub fn semantic_token_count(src: &str) -> usize {
    let result = analyze_fixture(src);
    let tokens = semantic_tokens_full(
        &result.intelligence,
        Position {
            line: 0,
            character: 0,
        },
    )
    .expect("tokens");
    match tokens {
        tower_lsp::lsp_types::SemanticTokensResult::Tokens(t) => t.data.len(),
        _ => 0,
    }
}

pub fn document_symbol_names(src: &str) -> Vec<String> {
    let result = analyze_fixture(src);
    let resp = document_symbols(&result.intelligence).expect("document symbols");
    let mut names = Vec::new();
    match resp {
        DocumentSymbolResponse::Nested(syms) => collect_symbol_names(&syms, &mut names),
        DocumentSymbolResponse::Flat(info) => {
            names.extend(info.into_iter().map(|s| s.name));
        }
    }
    names
}

fn collect_symbol_names(syms: &[tower_lsp::lsp_types::DocumentSymbol], out: &mut Vec<String>) {
    for s in syms {
        out.push(s.name.clone());
        if let Some(children) = &s.children {
            collect_symbol_names(children, out);
        }
    }
}

pub fn folding_range_count(src: &str) -> usize {
    let result = analyze_fixture(src);
    folding_ranges(&result.intelligence)
        .map(|r| r.len())
        .unwrap_or(0)
}

pub fn inlay_hint_labels(src: &str) -> Vec<String> {
    let result = analyze_fixture(src);
    let range = full_document_range(&result.intelligence.src);
    let hints = inlay_hints(&result.intelligence, range).unwrap_or_default();
    hints
        .into_iter()
        .map(|h| match h.label {
            tower_lsp::lsp_types::InlayHintLabel::String(s) => s,
            tower_lsp::lsp_types::InlayHintLabel::LabelParts(parts) => {
                parts.into_iter().map(|p| p.value).collect::<String>()
            }
        })
        .collect()
}

#[allow(dead_code)]
pub fn full_range(src: &str) -> Range {
    full_document_range(src)
}
