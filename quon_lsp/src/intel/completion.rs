use std::collections::HashSet;

use frontend::analysis::{
    DocumentAnalysis, SymbolKind, classical_builtins, gate_type, gates, keywords, partial_ident,
};
use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionResponse, Documentation, InsertTextFormat,
    MarkupContent, MarkupKind, Position,
};

use crate::convert::position_to_offset;

pub fn completions_at(
    analysis: &DocumentAnalysis,
    position: Position,
) -> Option<CompletionResponse> {
    let offset = position_to_offset(&analysis.src, position)?;
    let (_start, _end, prefix) = partial_ident(&analysis.src, offset);
    let mut items = Vec::new();

    let mut seen_labels = HashSet::new();

    for kw in keywords() {
        if prefix.is_empty() || kw.starts_with(prefix.as_str()) {
            push_item(
                &mut items,
                &mut seen_labels,
                item(kw, CompletionItemKind::KEYWORD, None),
            );
        }
    }

    for name in gates() {
        if prefix.is_empty() || name.starts_with(prefix.as_str()) {
            let detail = gate_type(name).map(|t| t.to_string());
            push_item(
                &mut items,
                &mut seen_labels,
                item(name, CompletionItemKind::FUNCTION, detail),
            );
        }
    }

    for name in classical_builtins() {
        if prefix.is_empty() || name.starts_with(prefix.as_str()) {
            push_item(
                &mut items,
                &mut seen_labels,
                item(name, CompletionItemKind::FUNCTION, None),
            );
        }
    }

    for sym in &analysis.symbols.symbols {
        if sym.name_span.start == sym.name_span.end {
            continue;
        }
        if !matches!(
            sym.kind,
            SymbolKind::LocalBinding | SymbolKind::Parameter | SymbolKind::Function
        ) {
            continue;
        }
        if !(prefix.is_empty() || sym.name.starts_with(prefix.as_str())) {
            continue;
        }
        if analysis.symbols.resolve_name_at(&sym.name, offset) != Some(sym.id) {
            continue;
        }
        push_item(
            &mut items,
            &mut seen_labels,
            item_with_docs(
                &sym.name,
                match sym.kind {
                    SymbolKind::Function => CompletionItemKind::FUNCTION,
                    _ => CompletionItemKind::VARIABLE,
                },
                sym.ty.as_ref().map(|t| t.to_string()),
                sym.docs.clone(),
            ),
        );
    }

    for sym in &analysis.symbols.symbols {
        if sym.kind != SymbolKind::TypeAlias {
            continue;
        }
        if sym.name_span.start == sym.name_span.end {
            continue;
        }
        if !(prefix.is_empty() || sym.name.starts_with(prefix.as_str())) {
            continue;
        }
        push_item(
            &mut items,
            &mut seen_labels,
            item_with_docs(&sym.name, CompletionItemKind::CLASS, None, sym.docs.clone()),
        );
    }

    static TYPE_NAMES: &[&str] = &[
        "Qubit", "QReg", "Bit", "Bool", "Int", "Float", "Unit", "Nat", "List", "Matrix", "Circuit",
        "Q",
    ];
    for ty in TYPE_NAMES {
        if prefix.is_empty() || ty.starts_with(prefix.as_str()) {
            push_item(
                &mut items,
                &mut seen_labels,
                item(ty, CompletionItemKind::TYPE_PARAMETER, None),
            );
        }
    }

    if prefix.is_empty() || "fn".starts_with(prefix.as_str()) {
        push_item(
            &mut items,
            &mut seen_labels,
            CompletionItem {
                label: "fn".into(),
                kind: Some(CompletionItemKind::SNIPPET),
                insert_text: Some("fn ${1:name}(${2:params}): ${3:Ret} = ${0:body}".into()),
                insert_text_format: Some(InsertTextFormat::SNIPPET),
                ..Default::default()
            },
        );
    }

    Some(CompletionResponse::Array(items))
}

fn push_item(items: &mut Vec<CompletionItem>, seen: &mut HashSet<String>, item: CompletionItem) {
    if seen.insert(item.label.clone()) {
        items.push(item);
    }
}

fn item(label: &str, kind: CompletionItemKind, detail: Option<String>) -> CompletionItem {
    item_with_docs(label, kind, detail, None)
}

fn item_with_docs(
    label: &str,
    kind: CompletionItemKind,
    detail: Option<String>,
    docs: Option<String>,
) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail,
        documentation: docs.map(|value| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            })
        }),
        ..Default::default()
    }
}
