use frontend::analysis::{
    DocumentAnalysis, SymbolKind, classical_builtins, gate_type, gates, keywords, partial_ident,
    resolve_at,
};
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, InsertTextFormat, Position,
};

use crate::convert::position_to_offset;

pub fn completions_at(
    analysis: &DocumentAnalysis,
    position: Position,
) -> Option<CompletionResponse> {
    let offset = position_to_offset(&analysis.src, position)?;
    let (_start, _end, prefix) = partial_ident(&analysis.src, offset);
    let mut items = Vec::new();

    for kw in keywords() {
        if prefix.is_empty() || kw.starts_with(prefix.as_str()) {
            items.push(item(kw, CompletionItemKind::KEYWORD, None));
        }
    }

    for name in gates() {
        if prefix.is_empty() || name.starts_with(prefix.as_str()) {
            let detail = gate_type(name).map(|t| t.to_string());
            items.push(item(name, CompletionItemKind::FUNCTION, detail));
        }
    }

    for name in classical_builtins() {
        if prefix.is_empty() || name.starts_with(prefix.as_str()) {
            items.push(item(name, CompletionItemKind::FUNCTION, None));
        }
    }

    for sym in &analysis.symbols.symbols {
        if sym.name_span.start == sym.name_span.end {
            continue;
        }
        if matches!(
            sym.kind,
            SymbolKind::LocalBinding | SymbolKind::Parameter | SymbolKind::Function
        ) && (prefix.is_empty() || sym.name.starts_with(prefix.as_str()))
        {
            items.push(item(
                &sym.name,
                CompletionItemKind::VARIABLE,
                sym.ty.as_ref().map(|t| t.to_string()),
            ));
        }
    }

    for alias in analysis.symbols.alias_names() {
        if prefix.is_empty() || alias.starts_with(prefix.as_str()) {
            items.push(item(alias, CompletionItemKind::CLASS, None));
        }
    }

    static TYPE_NAMES: &[&str] = &[
        "Qubit", "QReg", "Bit", "Bool", "Int", "Float", "Unit", "Nat", "List", "Matrix", "Circuit",
        "Q",
    ];
    for ty in TYPE_NAMES {
        if prefix.is_empty() || ty.starts_with(prefix.as_str()) {
            items.push(item(ty, CompletionItemKind::TYPE_PARAMETER, None));
        }
    }

    if prefix.is_empty() || "fn".starts_with(prefix.as_str()) {
        items.push(CompletionItem {
            label: "fn".into(),
            kind: Some(CompletionItemKind::SNIPPET),
            insert_text: Some("fn ${1:name}(${2:params}): ${3:Ret} = ${0:body}".into()),
            insert_text_format: Some(InsertTextFormat::SNIPPET),
            ..Default::default()
        });
    }

    let _ = resolve_at(analysis, offset);
    Some(CompletionResponse::Array(items))
}

fn item(label: &str, kind: CompletionItemKind, detail: Option<String>) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail,
        ..Default::default()
    }
}
