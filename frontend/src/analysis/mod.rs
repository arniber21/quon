mod annotations;
mod cursor;
mod prelude_names;
mod resolution;
mod scopes;
mod symbols;
mod typed;

pub use annotations::TypeAnnotations;
pub use cursor::{NodeAt, cursor_at, node_at_offset, partial_ident};
pub use prelude_names::{
    classical_builtins, gate_type, gates, is_quantum_builtin, keywords, quantum_builtins,
};
pub use resolution::{ResolutionMap, ResolvedTarget};
pub use symbols::{Symbol, SymbolId, SymbolIndex, SymbolKind, build_symbol_index};
pub use typed::TypedProgram;

use crate::ast::Decl;
use crate::diagnostics::Diagnostic;
use crate::lexer::{SimpleSpan, Sp};
use crate::typecheck::TypeChecker;
use crate::types::Ty;

/// Snapshot of one analysis pass over a source file.
#[derive(Debug, Clone)]
pub struct DocumentAnalysis {
    pub decls: Vec<Sp<Decl>>,
    pub symbols: SymbolIndex,
    pub annotations: TypeAnnotations,
    pub resolutions: ResolutionMap,
    pub diagnostics: Vec<Diagnostic>,
    pub src: String,
}

impl DocumentAnalysis {
    pub fn empty(src: impl Into<String>) -> Self {
        let src = src.into();
        Self {
            decls: Vec::new(),
            symbols: SymbolIndex::empty(),
            annotations: TypeAnnotations::default(),
            resolutions: ResolutionMap::default(),
            diagnostics: Vec::new(),
            src,
        }
    }
}

impl Default for DocumentAnalysis {
    fn default() -> Self {
        Self::empty(String::new())
    }
}

/// Full IDE analysis: rich diagnostics + intelligence snapshot in one pass.
pub fn analyze_with_rich(src: &str) -> crate::diagnostics::AnalysisResult {
    let mut intelligence = DocumentAnalysis::empty(src.to_string());
    let mut rich_diagnostics = Vec::new();

    let tokens = match crate::lexer::lex_rich(src) {
        Ok(t) => t,
        Err(diags) => {
            intelligence.diagnostics = diags.iter().map(Diagnostic::from).collect();
            return crate::diagnostics::AnalysisResult {
                diagnostics: diags,
                intelligence,
            };
        }
    };
    let decls = match crate::parser::parse_rich(&tokens) {
        Ok(d) => d,
        Err(diags) => {
            intelligence.diagnostics = diags.iter().map(Diagnostic::from).collect();
            return crate::diagnostics::AnalysisResult {
                diagnostics: diags,
                intelligence,
            };
        }
    };
    let decls = match crate::desugar::desugar_decls_rich(decls) {
        Ok(d) => d,
        Err(diags) => {
            intelligence.diagnostics = diags.iter().map(Diagnostic::from).collect();
            return crate::diagnostics::AnalysisResult {
                diagnostics: diags,
                intelligence,
            };
        }
    };

    intelligence.symbols = build_symbol_index(&decls, src.len());

    let mut checker = TypeChecker::new();
    checker.enable_analysis(&intelligence.symbols);
    let mut annotations = TypeAnnotations::default();
    let mut resolutions = ResolutionMap::default();
    checker.set_sinks(&mut annotations, &mut resolutions);

    if let Err(errs) = checker.check_decls(&decls) {
        rich_diagnostics = errs.iter().map(|e| e.to_rich_diagnostic(src)).collect();
        intelligence.diagnostics = rich_diagnostics.iter().map(Diagnostic::from).collect();
    }

    attach_types(&mut intelligence.symbols, &checker, &decls);
    intelligence.decls = decls;
    intelligence.annotations = annotations;
    intelligence.resolutions = resolutions;

    crate::diagnostics::AnalysisResult {
        diagnostics: rich_diagnostics,
        intelligence,
    }
}

/// Parse, desugar, build symbols, type-check with annotation/resolution sinks.
/// Always returns a snapshot (never `Err`).
pub fn analyze_program(src: &str) -> DocumentAnalysis {
    analyze_with_rich(src).intelligence
}

fn attach_types(symbols: &mut SymbolIndex, checker: &TypeChecker, decls: &[Sp<Decl>]) {
    for sym in &mut symbols.symbols {
        if sym.kind == SymbolKind::Function
            && let Some(ty) = checker.fn_type_of(&sym.name)
        {
            sym.ty = Some(ty.clone());
        }
    }
    for (decl, _) in decls {
        if let crate::ast::Decl::Fn { name, .. } = decl
            && let Some(ty) = checker.fn_type_of(&name.0)
            && let Some(id) = symbols.by_def_span(name.1)
            && let Some(s) = symbols.get_mut(id)
        {
            s.ty = Some(ty.clone());
        }
    }
}

impl SymbolIndex {
    fn get_mut(&mut self, id: SymbolId) -> Option<&mut Symbol> {
        self.symbols.get_mut(id.0 as usize)
    }
}

/// Resolve hover/definition target at byte offset.
pub fn resolve_at(analysis: &DocumentAnalysis, offset: usize) -> Option<ResolvedQuery> {
    let node = node_at_offset(&analysis.decls, offset)?;
    let (name, use_span) = match node {
        cursor::NodeAt::Name(n, sp) => (n, sp),
        _ => return None,
    };
    if let Some(target) = analysis.resolutions.get(use_span) {
        return Some(ResolvedQuery {
            name: name.to_string(),
            use_span,
            target: target.clone(),
        });
    }
    if let Some(id) = analysis.symbols.resolve_name_at(name, offset) {
        return Some(ResolvedQuery {
            name: name.to_string(),
            use_span,
            target: ResolvedTarget::Symbol(id),
        });
    }
    None
}

#[derive(Debug, Clone)]
pub struct ResolvedQuery {
    pub name: String,
    pub use_span: SimpleSpan,
    pub target: ResolvedTarget,
}

pub fn format_hover(query: &ResolvedQuery, analysis: &DocumentAnalysis) -> String {
    let mut lines = Vec::new();
    match &query.target {
        ResolvedTarget::Symbol(id) => {
            let Some(sym) = analysis.symbols.get(*id) else {
                return "`(unknown)`".to_string();
            };
            let kind = match sym.kind {
                SymbolKind::Function => "(function)",
                SymbolKind::Parameter => "(parameter)",
                SymbolKind::LocalBinding => "(variable)",
                SymbolKind::LinearBinding => "(linear resource)",
                SymbolKind::TypeAlias => "(type alias)",
                SymbolKind::TypeParam => "(type parameter)",
                SymbolKind::Builtin | SymbolKind::QuantumBuiltin => "(builtin)",
                SymbolKind::Gate => "(gate)",
            };
            lines.push(format!("**{}** `{}`", kind, sym.name));
            if let Some(ty) = analysis.annotations.get(query.use_span) {
                lines.push(format!("```quon\n{ty}\n```"));
                append_circuit_details(&mut lines, ty);
                if ty.is_linear_resource() && sym.kind != SymbolKind::Gate {
                    lines.push("*must be consumed exactly once*".to_string());
                }
            } else if let Some(ref ty) = sym.ty.clone().or_else(|| {
                if sym.kind == SymbolKind::Gate {
                    prelude_names::gate_type(&sym.name)
                } else {
                    None
                }
            }) {
                lines.push(format!("```quon\n{ty}\n```"));
                append_circuit_details(&mut lines, ty);
                if ty.is_linear_resource() && sym.kind != SymbolKind::Gate {
                    lines.push("*must be consumed exactly once*".to_string());
                }
            } else if let Some(ty) = analysis.annotations.get(sym.name_span) {
                lines.push(format!("```quon\n{ty}\n```"));
                append_circuit_details(&mut lines, ty);
            }
        }
        ResolvedTarget::Builtin(name) => {
            lines.push(format!("**(builtin)** `{name}`"));
            if let Some(scheme) = crate::typecheck::builtins::lookup(name) {
                lines.push(format!("```quon\n{}\n```", scheme.body));
            }
        }
        ResolvedTarget::QuantumBuiltin(name) => {
            lines.push(format!("**(quantum builtin)** `{name}`"));
        }
        ResolvedTarget::Gate(name) => {
            lines.push(format!("**(gate)** `{name}`"));
            if let Some(ty) = crate::analysis::prelude_names::gate_type(name) {
                lines.push(format!("```quon\n{ty}\n```"));
                append_circuit_details(&mut lines, &ty);
            }
        }
        ResolvedTarget::TypeAlias(id) => {
            if let Some(sym) = analysis.symbols.get(*id) {
                lines.push(format!("**(type alias)** `{}`", sym.name));
            }
        }
    }
    lines.join("\n\n")
}

fn append_circuit_details(lines: &mut Vec<String>, ty: &Ty) {
    if let Ty::Circuit { n, m, d, c } = ty {
        lines.push(format!("**Width**: n = `{n}`, m = `{m}`"));
        lines.push(format!("**Depth bound**: `{d}`"));
        lines.push(format!("**Clifford class**: `{}`", clifford_display(c)));
    } else if let Ty::QReg(n) = ty {
        lines.push(format!("**Register width**: {n}"));
    }
}

fn clifford_display(c: &crate::ast::CliffordClass) -> &'static str {
    use crate::ast::CliffordClass;
    match c {
        CliffordClass::Clifford | CliffordClass::Infer => "Clifford",
        CliffordClass::Universal => "Universal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_simple_fn() {
        let src = "fn f(): Int = 1\n";
        let a = analyze_program(src);
        assert!(a.diagnostics.is_empty());
        assert!(a.symbols.symbols.iter().any(|s| s.name == "f"));
    }
}
