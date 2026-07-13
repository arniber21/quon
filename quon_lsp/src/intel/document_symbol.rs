use frontend::analysis::{DocumentAnalysis, Scope, ScopeId, SymbolKind as QuonSymbolKind};
use frontend::ast::Decl;
use tower_lsp::lsp_types::{DocumentSymbol, DocumentSymbolResponse, SymbolKind};

use crate::convert::span_to_range;

/// Hierarchical outline: top-level `fn` / `type`, with nested locals when present.
pub fn document_symbols(analysis: &DocumentAnalysis) -> Option<DocumentSymbolResponse> {
    let mut symbols = Vec::new();
    for (decl, decl_span) in &analysis.decls {
        match decl {
            Decl::Fn { name, .. } => {
                let mut children = Vec::new();
                if let Some(fn_id) = analysis.symbols.by_def_span(name.1)
                    && let Some(fn_sym) = analysis.symbols.get(fn_id)
                {
                    children = nested_locals(analysis, fn_sym.scope);
                }
                symbols.push(doc_symbol(
                    name.0.clone(),
                    fn_detail(analysis, &name.0),
                    SymbolKind::FUNCTION,
                    span_to_range(&analysis.src, *decl_span),
                    span_to_range(&analysis.src, name.1),
                    children,
                ));
            }
            Decl::TypeAlias { name, params, .. } => {
                let children = params
                    .iter()
                    .map(|p| {
                        doc_symbol(
                            p.0.clone(),
                            None,
                            SymbolKind::TYPE_PARAMETER,
                            span_to_range(&analysis.src, p.1),
                            span_to_range(&analysis.src, p.1),
                            Vec::new(),
                        )
                    })
                    .collect();
                symbols.push(doc_symbol(
                    name.0.clone(),
                    Some("type alias".into()),
                    SymbolKind::STRUCT,
                    span_to_range(&analysis.src, *decl_span),
                    span_to_range(&analysis.src, name.1),
                    children,
                ));
            }
        }
    }
    if symbols.is_empty() {
        None
    } else {
        Some(DocumentSymbolResponse::Nested(symbols))
    }
}

fn nested_locals(analysis: &DocumentAnalysis, fn_scope: ScopeId) -> Vec<DocumentSymbol> {
    let mut out = Vec::new();
    for sym in &analysis.symbols.symbols {
        if !matches!(
            sym.kind,
            QuonSymbolKind::Parameter
                | QuonSymbolKind::LocalBinding
                | QuonSymbolKind::LinearBinding
                | QuonSymbolKind::TypeParam
        ) {
            continue;
        }
        if sym.name_span.start == sym.name_span.end {
            continue;
        }
        if !scope_is_under(&analysis.symbols.scopes, sym.scope, fn_scope) {
            continue;
        }
        // Desugaring invents `$bind…` temps — keep them out of the outline.
        if sym.name.starts_with('$') {
            continue;
        }
        let detail = sym.ty.as_ref().map(|t| t.to_string()).or_else(|| {
            analysis
                .annotations
                .get(sym.name_span)
                .map(|t| t.to_string())
        });
        out.push(doc_symbol(
            sym.name.clone(),
            detail,
            SymbolKind::VARIABLE,
            span_to_range(&analysis.src, sym.name_span),
            span_to_range(&analysis.src, sym.name_span),
            Vec::new(),
        ));
    }
    out
}

fn scope_is_under(scopes: &[Scope], mut scope: ScopeId, ancestor: ScopeId) -> bool {
    loop {
        if scope == ancestor {
            return true;
        }
        let Some(s) = scopes.get(scope.0 as usize) else {
            return false;
        };
        match s.parent {
            Some(p) => scope = p,
            None => return false,
        }
    }
}

fn fn_detail(analysis: &DocumentAnalysis, name: &str) -> Option<String> {
    analysis
        .symbols
        .symbols
        .iter()
        .find(|s| s.kind == QuonSymbolKind::Function && s.name == name)
        .and_then(|s| s.ty.as_ref())
        .map(|t| t.to_string())
}

#[allow(deprecated)]
fn doc_symbol(
    name: String,
    detail: Option<String>,
    kind: SymbolKind,
    range: tower_lsp::lsp_types::Range,
    selection_range: tower_lsp::lsp_types::Range,
    children: Vec<DocumentSymbol>,
) -> DocumentSymbol {
    DocumentSymbol {
        name,
        detail,
        kind,
        tags: None,
        deprecated: None,
        range,
        selection_range,
        children: if children.is_empty() {
            None
        } else {
            Some(children)
        },
    }
}
