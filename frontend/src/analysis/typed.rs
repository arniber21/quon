use std::collections::HashMap;

use crate::ast::Decl;
use crate::lexer::{SimpleSpan, Sp};
use crate::types::Ty;

use super::{DocumentAnalysis, TypeAnnotations};

/// Typed metadata exported for lint rules (issue #47).
///
/// Built from a successful [`DocumentAnalysis`] pass; rules must not re-derive types.
#[derive(Debug, Clone)]
pub struct TypedProgram {
    pub decls: Vec<Sp<Decl>>,
    pub fn_types: HashMap<String, Ty>,
    pub expr_types: HashMap<(usize, usize), Ty>,
}

impl TypedProgram {
    /// Build from analysis output. Returns `None` when parse/desugar/typecheck failed.
    pub fn from_analysis(analysis: &DocumentAnalysis) -> Option<Self> {
        if !analysis.diagnostics.is_empty() {
            return None;
        }
        let mut fn_types = HashMap::new();
        for sym in &analysis.symbols.symbols {
            if let Some(ty) = &sym.ty {
                fn_types.insert(sym.name.clone(), ty.clone());
            }
        }
        Some(Self {
            decls: analysis.decls.clone(),
            fn_types,
            expr_types: expr_types_map(&analysis.annotations),
        })
    }

    pub fn fn_type(&self, name: &str) -> Option<&Ty> {
        self.fn_types.get(name)
    }

    pub fn expr_type(&self, span: SimpleSpan) -> Option<&Ty> {
        self.expr_types.get(&(span.start, span.end))
    }
}

fn expr_types_map(annotations: &TypeAnnotations) -> HashMap<(usize, usize), Ty> {
    annotations.iter().map(|(k, v)| (k, v.clone())).collect()
}
