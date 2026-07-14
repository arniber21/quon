use std::collections::HashSet;

use frontend::analysis::{
    CompletionContext, DocumentAnalysis, SymbolKind, applyables, builtin_type, classical_builtins,
    completion_context_at, gate_type, gates, in_circuit_block, keywords, partial_ident,
    quantum_builtins, type_names,
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
    let ctx = completion_context_at(&analysis.src, &analysis.decls, offset);
    let in_circuit = in_circuit_block(&analysis.decls, offset);

    let mut items = Vec::new();
    let mut seen = HashSet::new();

    match ctx {
        CompletionContext::AfterAt => {
            push_gates(&mut items, &mut seen, &prefix);
            push_applyables(&mut items, &mut seen, &prefix);
        }
        CompletionContext::TypePosition => {
            push_types(&mut items, &mut seen, analysis, &prefix);
        }
        CompletionContext::Expression => {
            push_keywords(&mut items, &mut seen, &prefix);
            push_snippets(&mut items, &mut seen, &prefix);
            push_scope_symbols(&mut items, &mut seen, analysis, offset, &prefix);
            push_classical_builtins(&mut items, &mut seen, &prefix);
            push_quantum_builtins(&mut items, &mut seen, &prefix);
            if in_circuit {
                push_gates(&mut items, &mut seen, &prefix);
                push_applyables(&mut items, &mut seen, &prefix);
            }
        }
    }

    Some(CompletionResponse::Array(items))
}

fn push_keywords(items: &mut Vec<CompletionItem>, seen: &mut HashSet<String>, prefix: &str) {
    for kw in keywords() {
        if matches_prefix(prefix, kw) {
            push_item(
                items,
                seen,
                rich_item(
                    kw,
                    CompletionItemKind::KEYWORD,
                    None,
                    Some(format!("keyword `{kw}`")),
                ),
            );
        }
    }
}

fn push_gates(items: &mut Vec<CompletionItem>, seen: &mut HashSet<String>, prefix: &str) {
    for name in gates() {
        if !matches_prefix(prefix, name) {
            continue;
        }
        let detail = gate_type(name).map(|t| t.to_string());
        let docs = detail
            .as_ref()
            .map(|t| format!("**(gate)** `{name}`\n\n```quon\n{t}\n```"));
        push_item(
            items,
            seen,
            rich_item(name, CompletionItemKind::FUNCTION, detail, docs),
        );
    }
}

fn push_applyables(items: &mut Vec<CompletionItem>, seen: &mut HashSet<String>, prefix: &str) {
    for name in applyables() {
        if matches_prefix(prefix, name) {
            push_item(
                items,
                seen,
                rich_item(
                    name,
                    CompletionItemKind::FUNCTION,
                    Some("circuit applyable".into()),
                    Some(format!("**(applyable)** `{name}`")),
                ),
            );
        }
    }
}

fn push_classical_builtins(
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<String>,
    prefix: &str,
) {
    for name in classical_builtins() {
        if !matches_prefix(prefix, name) {
            continue;
        }
        let detail = builtin_type(name).map(|t| t.to_string());
        let docs = detail
            .as_ref()
            .map(|t| format!("**(builtin)** `{name}`\n\n```quon\n{t}\n```"));
        push_item(
            items,
            seen,
            rich_item(name, CompletionItemKind::FUNCTION, detail, docs),
        );
    }
}

fn push_quantum_builtins(
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<String>,
    prefix: &str,
) {
    for name in quantum_builtins() {
        if !matches_prefix(prefix, name) {
            continue;
        }
        let detail = builtin_type(name).map(|t| t.to_string());
        let docs = Some(match &detail {
            Some(t) => format!("**(quantum builtin)** `{name}`\n\n```quon\n{t}\n```"),
            None => format!("**(quantum builtin)** `{name}`"),
        });
        push_item(
            items,
            seen,
            rich_item(name, CompletionItemKind::FUNCTION, detail, docs),
        );
    }
}

fn push_scope_symbols(
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<String>,
    analysis: &DocumentAnalysis,
    offset: usize,
    prefix: &str,
) {
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
        if !matches_prefix(prefix, &sym.name) {
            continue;
        }
        // Top-level `fn` names are globally visible (checker Γ); locals/params use scope walk.
        let in_scope = match sym.kind {
            SymbolKind::Function => true,
            _ => analysis.symbols.resolve_name_at(&sym.name, offset) == Some(sym.id),
        };
        if !in_scope {
            continue;
        }
        let kind = match sym.kind {
            SymbolKind::Function => CompletionItemKind::FUNCTION,
            SymbolKind::Parameter => CompletionItemKind::VARIABLE,
            _ => CompletionItemKind::VARIABLE,
        };
        let detail = sym.ty.as_ref().map(|t| t.to_string());
        let kind_label = match sym.kind {
            SymbolKind::Function => "function",
            SymbolKind::Parameter => "parameter",
            _ => "variable",
        };
        let docs = symbol_docs(
            sym.docs.as_deref(),
            kind_label,
            &sym.name,
            detail.as_deref(),
        );
        push_item(items, seen, rich_item(&sym.name, kind, detail, docs));
    }
}

fn push_types(
    items: &mut Vec<CompletionItem>,
    seen: &mut HashSet<String>,
    analysis: &DocumentAnalysis,
    prefix: &str,
) {
    for ty in type_names() {
        if matches_prefix(prefix, ty) {
            push_item(
                items,
                seen,
                rich_item(
                    ty,
                    CompletionItemKind::TYPE_PARAMETER,
                    Some("builtin type".into()),
                    Some(format!("**(type)** `{ty}`")),
                ),
            );
        }
    }
    for sym in &analysis.symbols.symbols {
        if sym.kind != SymbolKind::TypeAlias {
            continue;
        }
        if sym.name_span.start == sym.name_span.end {
            continue;
        }
        if !matches_prefix(prefix, &sym.name) {
            continue;
        }
        let docs = symbol_docs(sym.docs.as_deref(), "type alias", &sym.name, None);
        push_item(
            items,
            seen,
            rich_item(
                &sym.name,
                CompletionItemKind::CLASS,
                Some("type alias".into()),
                docs,
            ),
        );
    }
}

/// Combine leading `--` docs (from symbol index) with kind/type markup.
fn symbol_docs(
    leading: Option<&str>,
    kind_label: &str,
    name: &str,
    ty: Option<&str>,
) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(docs) = leading {
        parts.push(docs.to_string());
    }
    parts.push(format!("**({kind_label})** `{name}`"));
    if let Some(t) = ty {
        parts.push(format!("```quon\n{t}\n```"));
    }
    Some(parts.join("\n\n"))
}

fn push_snippets(items: &mut Vec<CompletionItem>, seen: &mut HashSet<String>, prefix: &str) {
    let snippets: &[(&str, &str, &str)] = &[
        (
            "fn",
            "fn ${1:name}(${2:params}): ${3:Ret} = ${0:body}",
            "function declaration",
        ),
        ("circuit", "circuit {\n\t$0\n}", "circuit { … } block"),
        ("run", "run {\n\t$0\n}", "run { … } block"),
        (
            "borrow",
            "borrow ${1:q}: ${2:Qubit} in {\n\t$0\n}",
            "borrow … in { … }",
        ),
    ];
    for (label, insert, doc) in snippets {
        if matches_prefix(prefix, label) {
            push_item(
                items,
                seen,
                CompletionItem {
                    label: (*label).into(),
                    kind: Some(CompletionItemKind::SNIPPET),
                    detail: Some((*doc).into()),
                    documentation: Some(Documentation::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: format!("```quon\n{insert}\n```"),
                    })),
                    insert_text: Some((*insert).into()),
                    insert_text_format: Some(InsertTextFormat::SNIPPET),
                    ..Default::default()
                },
            );
        }
    }
}

fn matches_prefix(prefix: &str, label: &str) -> bool {
    prefix.is_empty() || label.starts_with(prefix)
}

fn push_item(items: &mut Vec<CompletionItem>, seen: &mut HashSet<String>, item: CompletionItem) {
    if seen.insert(item.label.clone()) {
        items.push(item);
    }
}

fn rich_item(
    label: &str,
    kind: CompletionItemKind,
    detail: Option<String>,
    documentation: Option<String>,
) -> CompletionItem {
    CompletionItem {
        label: label.to_string(),
        kind: Some(kind),
        detail,
        documentation: documentation.map(|value| {
            Documentation::MarkupContent(MarkupContent {
                kind: MarkupKind::Markdown,
                value,
            })
        }),
        ..Default::default()
    }
}
