use frontend::analysis::cursor_at;
use frontend::analyze;
use tower_lsp::lsp_types::{CompletionItem, Position, SignatureHelp, Url};

use quon_lsp::intel::{
    completions_at, definition_at, document_highlight_at, hover_at, prepare_rename_at,
    references_at, rename_at, semantic_tokens_full, signature_help_at,
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
