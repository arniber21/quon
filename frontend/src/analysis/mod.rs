mod annotations;
mod cursor;
mod docs;
mod prelude_names;
mod resolution;
mod scopes;
mod symbols;
mod typed;

pub use annotations::TypeAnnotations;
pub use cursor::{NodeAt, cursor_at, node_at_offset, partial_ident};
pub use docs::extract_leading_docs;
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

    intelligence.symbols = build_symbol_index(&decls, src);

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
    // Standing on a file-scoped definition (fn / type alias name) that is not
    // also a use site — look up by definition span.
    if let Some(id) = analysis.symbols.by_def_span(use_span) {
        let target = match analysis.symbols.get(id).map(|s| s.kind) {
            Some(SymbolKind::TypeAlias) => ResolvedTarget::TypeAlias(id),
            _ => ResolvedTarget::Symbol(id),
        };
        return Some(ResolvedQuery {
            name: name.to_string(),
            use_span,
            target,
        });
    }
    None
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OccurrenceKind {
    /// Binding / definition site.
    Write,
    /// Use site recorded in [`ResolutionMap`].
    Read,
}

/// In-file occurrences of the symbol under `query` (definition + resolved uses).
///
/// `Symbol` and `TypeAlias` targets that share a [`SymbolId`] are treated as the
/// same entity — alias definitions often resolve as `Symbol` while uses record
/// `TypeAlias`.
pub fn occurrences_of(
    analysis: &DocumentAnalysis,
    target: &ResolvedTarget,
) -> Vec<(SimpleSpan, OccurrenceKind)> {
    let Some(id) = target_symbol_id(target) else {
        return Vec::new();
    };

    let mut out = Vec::new();
    if let Some(sym) = analysis.symbols.get(id)
        && sym.name_span.start != sym.name_span.end
    {
        out.push((sym.name_span, OccurrenceKind::Write));
    }
    for (span, resolved) in analysis.resolutions.entries() {
        if target_symbol_id(resolved) == Some(id) && span.start != span.end {
            out.push((span, OccurrenceKind::Read));
        }
    }

    out.sort_by_key(|(span, _)| (span.start, span.end));
    out.dedup_by_key(|(span, _)| (span.start, span.end));
    out
}

pub fn target_symbol_id(target: &ResolvedTarget) -> Option<SymbolId> {
    match target {
        ResolvedTarget::Symbol(id) | ResolvedTarget::TypeAlias(id) => Some(*id),
        ResolvedTarget::Builtin(_)
        | ResolvedTarget::Gate(_)
        | ResolvedTarget::QuantumBuiltin(_) => None,
    }
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
            if let Some(docs) = sym.docs.as_deref() {
                lines.push(docs.to_string());
            }
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
                if let Some(docs) = sym.docs.as_deref() {
                    lines.push(docs.to_string());
                }
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

    #[test]
    fn call_site_resolves_to_fn() {
        let src = "fn g(): Int = 1\nfn f(): Int = /*cursor*/g()\n";
        let clean = src.replace("/*cursor*/", "");
        let offset = cursor_at(src, "/*cursor*/");
        let a = analyze_program(&clean);
        let q = resolve_at(&a, offset).expect("resolve call");
        assert_eq!(q.name, "g");
        match q.target {
            ResolvedTarget::Symbol(id) => {
                let sym = a.symbols.get(id).expect("sym");
                assert_eq!(sym.kind, SymbolKind::Function);
                assert_eq!(sym.name, "g");
            }
            other => panic!("expected Symbol, got {other:?}"),
        }
    }

    #[test]
    fn type_alias_use_resolves() {
        let src = "type MyInt = Int\nfn f(): /*cursor*/MyInt = 1\n";
        let clean = src.replace("/*cursor*/", "");
        let offset = cursor_at(src, "/*cursor*/");
        let a = analyze_program(&clean);
        let q = resolve_at(&a, offset).expect("resolve alias");
        assert_eq!(q.name, "MyInt");
        assert!(matches!(q.target, ResolvedTarget::TypeAlias(_)));
    }

    #[test]
    fn param_def_site_resolves() {
        let src = "fn f(/*cursor*/x: Int): Int = x\n";
        let clean = src.replace("/*cursor*/", "");
        let offset = cursor_at(src, "/*cursor*/");
        let a = analyze_program(&clean);
        let q = resolve_at(&a, offset).expect("resolve param");
        assert_eq!(q.name, "x");
        let occs = occurrences_of(&a, &q.target);
        assert!(
            occs.iter().any(|(_, k)| *k == OccurrenceKind::Write),
            "expected write occurrence"
        );
        assert!(
            occs.iter().any(|(_, k)| *k == OccurrenceKind::Read),
            "expected read occurrence: {occs:?}"
        );
    }
}
