use crate::ast::{Decl, Expr, Pat, Stmt};
use crate::lexer::{SimpleSpan, Sp};

use super::docs::extract_leading_docs;
use super::prelude_names::{classical_builtins, gates, quantum_builtins};
use super::scopes::{ScopeId, ScopeStack};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SymbolId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    TypeAlias,
    TypeParam,
    Parameter,
    LocalBinding,
    LinearBinding,
    Builtin,
    Gate,
    QuantumBuiltin,
}

#[derive(Debug, Clone)]
pub struct Symbol {
    pub id: SymbolId,
    pub name: String,
    pub kind: SymbolKind,
    pub name_span: SimpleSpan,
    pub scope: ScopeId,
    pub ty: Option<crate::types::Ty>,
    /// Leading `--` / `{- -}` comments immediately above the declaration.
    pub docs: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SymbolIndex {
    pub symbols: Vec<Symbol>,
    pub scopes: Vec<super::scopes::Scope>,
    by_name: std::collections::HashMap<String, Vec<SymbolId>>,
    by_def_span: std::collections::HashMap<(usize, usize), SymbolId>,
}

struct Builder<'a> {
    src: &'a str,
    index: SymbolIndex,
    stack: ScopeStack,
    next_id: u32,
}

impl<'a> Builder<'a> {
    fn new(src: &'a str) -> Self {
        let root_span = SimpleSpan::from(0..src.len());
        Self {
            src,
            index: SymbolIndex {
                symbols: Vec::new(),
                scopes: Vec::new(),
                by_name: std::collections::HashMap::new(),
                by_def_span: std::collections::HashMap::new(),
            },
            stack: ScopeStack::new(root_span),
            next_id: 0,
        }
    }

    fn insert(
        &mut self,
        name: String,
        kind: SymbolKind,
        name_span: SimpleSpan,
        docs: Option<String>,
    ) -> SymbolId {
        let id = SymbolId(self.next_id);
        self.next_id += 1;
        let scope = self.stack.current();
        self.index
            .by_def_span
            .insert((name_span.start, name_span.end), id);
        self.index.by_name.entry(name.clone()).or_default().push(id);
        self.index.symbols.push(Symbol {
            id,
            name,
            kind,
            name_span,
            scope,
            ty: None,
            docs,
        });
        self.stack.add_symbol(id);
        id
    }

    fn insert_plain(&mut self, name: String, kind: SymbolKind, name_span: SimpleSpan) -> SymbolId {
        self.insert(name, kind, name_span, None)
    }

    fn finish(mut self) -> SymbolIndex {
        self.index.scopes = self.stack.scopes().to_vec();
        self.index
    }
}

impl SymbolIndex {
    pub fn empty() -> Self {
        Builder::new("").finish()
    }

    pub fn get(&self, id: SymbolId) -> Option<&Symbol> {
        self.symbols.get(id.0 as usize)
    }

    pub fn by_def_span(&self, span: SimpleSpan) -> Option<SymbolId> {
        self.by_def_span.get(&(span.start, span.end)).copied()
    }

    pub fn resolve_name_at(&self, name: &str, offset: usize) -> Option<SymbolId> {
        self.resolve_name_at_assuming_rename(name, offset, None)
    }

    /// Like [`Self::resolve_name_at`], but treats `renamed` as already named `new_name`.
    ///
    /// Used to preview whether a rename would still resolve every occurrence to the
    /// same binding (no intervening / sibling shadow).
    pub fn resolve_name_at_assuming_rename(
        &self,
        name: &str,
        offset: usize,
        renamed: Option<(SymbolId, &str)>,
    ) -> Option<SymbolId> {
        let mut scope_id = self.innermost_scope(offset)?;
        while let Some(scope) = self.scopes.get(scope_id.0 as usize) {
            for &sym_id in scope.symbols.iter().rev() {
                if let Some(sym) = self.get(sym_id) {
                    let effective = match renamed {
                        Some((id, new_name)) if id == sym_id => new_name,
                        _ => sym.name.as_str(),
                    };
                    if effective == name {
                        return Some(sym_id);
                    }
                }
            }
            scope_id = scope.parent?;
        }
        None
    }

    fn innermost_scope(&self, offset: usize) -> Option<ScopeId> {
        let mut best: Option<(usize, ScopeId)> = None;
        for scope in &self.scopes {
            if scope.span.start <= offset && offset <= scope.span.end {
                let size = scope.span.end.saturating_sub(scope.span.start);
                if best.is_none_or(|(s, _)| size < s) {
                    best = Some((size, scope.id));
                }
            }
        }
        best.map(|(_, id)| id)
    }

    pub fn alias_names(&self) -> impl Iterator<Item = &str> {
        self.symbols
            .iter()
            .filter(|s| s.kind == SymbolKind::TypeAlias)
            .map(|s| s.name.as_str())
    }
}

pub fn build_symbol_index(decls: &[Sp<Decl>], src: &str) -> SymbolIndex {
    let mut b = Builder::new(src);
    let empty = SimpleSpan::from(0..0);
    for name in classical_builtins() {
        b.insert_plain(name.to_string(), SymbolKind::Builtin, empty);
    }
    for name in gates() {
        b.insert_plain(name.to_string(), SymbolKind::Gate, empty);
    }
    for name in quantum_builtins() {
        b.insert_plain(name.to_string(), SymbolKind::QuantumBuiltin, empty);
    }

    for (decl, decl_span) in decls {
        match decl {
            Decl::Fn {
                name,
                params,
                ret: _,
                body,
            } => {
                // Top-level functions live in the parent (file) scope so call sites
                // in other decls can resolve them. Params/body stay in a child scope.
                let docs = extract_leading_docs(b.src, decl_span.start);
                b.insert(name.0.clone(), SymbolKind::Function, name.1, docs);
                b.stack.push(*decl_span);
                for (p, _) in params {
                    b.insert_plain(p.0.clone(), SymbolKind::Parameter, p.1);
                }
                walk_expr(body, &mut b);
                b.stack.pop();
            }
            Decl::TypeAlias {
                name,
                params,
                ty: _,
            } => {
                // Same as functions: alias names are file-scoped.
                let docs = extract_leading_docs(b.src, decl_span.start);
                b.insert(name.0.clone(), SymbolKind::TypeAlias, name.1, docs);
                b.stack.push(*decl_span);
                for p in params {
                    b.insert_plain(p.0.clone(), SymbolKind::TypeParam, p.1);
                }
                b.stack.pop();
            }
        }
    }
    b.finish()
}

fn walk_expr(expr: &Sp<Expr>, b: &mut Builder<'_>) {
    let (e, span) = expr;
    match e {
        Expr::Lam { params, body } => {
            b.stack.push(*span);
            for (pat, _) in params {
                bind_pat(pat, SymbolKind::Parameter, b);
            }
            walk_expr(body, b);
            b.stack.pop();
        }
        Expr::Let { pat, rhs, body } => {
            walk_expr(rhs, b);
            b.stack.push(*span);
            bind_pat(pat, SymbolKind::LocalBinding, b);
            walk_expr(body, b);
            b.stack.pop();
        }
        Expr::Bind { rhs, param, body } => {
            walk_expr(rhs, b);
            b.stack.push(*span);
            b.insert_plain(param.0.clone(), SymbolKind::LocalBinding, param.1);
            walk_expr(body, b);
            b.stack.pop();
        }
        Expr::If { cond, then, else_ } => {
            walk_expr(cond, b);
            walk_expr(then, b);
            walk_expr(else_, b);
        }
        Expr::Match { scrutinee, arms } => {
            walk_expr(scrutinee, b);
            for (pat, arm) in arms {
                let arm_span = pat.1.start..arm.1.end;
                b.stack.push(SimpleSpan::from(arm_span));
                bind_pat(pat, SymbolKind::LocalBinding, b);
                walk_expr(arm, b);
                b.stack.pop();
            }
        }
        Expr::For { pat, iter, body } => {
            walk_expr(iter, b);
            b.stack.push(*span);
            bind_pat(pat, SymbolKind::Parameter, b);
            walk_expr(body, b);
            b.stack.pop();
        }
        Expr::Borrow { bindings, body } => {
            b.stack.push(*span);
            for (name, _) in bindings {
                b.insert_plain(name.0.clone(), SymbolKind::Parameter, name.1);
            }
            walk_stmts(body, b);
            b.stack.pop();
        }
        Expr::CircuitBlock(stmts) | Expr::RunBlock(stmts) => {
            b.stack.push(*span);
            walk_stmts(stmts, b);
            b.stack.pop();
        }
        Expr::App(a, br)
        | Expr::Compose(a, br)
        | Expr::Par(a, br)
        | Expr::GateApp {
            gate: a,
            qubits: br,
            ..
        } => {
            walk_expr(a, b);
            walk_expr(br, b);
        }
        Expr::BinOp { lhs, rhs, .. } => {
            walk_expr(lhs, b);
            walk_expr(rhs, b);
        }
        Expr::Neg(inner)
        | Expr::Adjoint(inner)
        | Expr::Controlled(inner)
        | Expr::Return(inner)
        | Expr::Ascribe(inner, _) => walk_expr(inner, b),
        Expr::Tuple(es) | Expr::List(es) => {
            for e in es {
                walk_expr(e, b);
            }
        }
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unit | Expr::Var(_) => {}
    }
}

fn walk_stmts(stmts: &[Sp<Stmt>], b: &mut Builder<'_>) {
    if stmts.is_empty() {
        return;
    }
    let block_start = stmts.first().map(|(_, s)| s.start).unwrap_or(0);
    let block_end = stmts.last().map(|(_, s)| s.end).unwrap_or(0);
    b.stack.push(SimpleSpan::from(block_start..block_end));
    for (stmt, _) in stmts {
        match stmt {
            Stmt::Bind { pat, rhs } | Stmt::Let { pat, rhs } => {
                walk_expr(rhs, b);
                bind_pat(pat, SymbolKind::LocalBinding, b);
            }
            Stmt::Expr(e) => walk_expr(e, b),
        }
    }
    b.stack.pop();
}

fn bind_pat(pat: &Sp<Pat>, kind: SymbolKind, b: &mut Builder<'_>) {
    match &pat.0 {
        Pat::Var(name) => {
            b.insert_plain(name.clone(), kind, pat.1);
        }
        Pat::Tuple(ps) => {
            for p in ps {
                bind_pat(p, kind, b);
            }
        }
        Pat::Wildcard | Pat::Lit(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn top_level_fn_symbol() {
        let src = "fn f(): Int = 1\n";
        let decls = crate::desugar_program(src).expect("parse");
        let index = build_symbol_index(&decls, src);
        assert!(
            index
                .symbols
                .iter()
                .any(|s| s.kind == SymbolKind::Function && s.name == "f")
        );
    }

    #[test]
    fn leading_docs_attach_to_fn_symbol() {
        let src = "-- Prepare a Bell pair\nfn bell_state(): Int = 1\n";
        let decls = crate::desugar_program(src).expect("parse");
        let index = build_symbol_index(&decls, src);
        let sym = index
            .symbols
            .iter()
            .find(|s| s.name == "bell_state")
            .expect("bell_state");
        assert_eq!(sym.docs.as_deref(), Some("Prepare a Bell pair"));
    }

    #[test]
    fn circuit_block_later_stmt_sees_earlier_binding() {
        let src = r#"
fn f(): Circuit<1, 1, 1, Clifford> = circuit {
  let x = 0
  H @ x
}
"#;
        let decls = crate::desugar_program(src).expect("parse");
        let index = build_symbol_index(&decls, src);
        let h_offset = src.find("H @").expect("H");
        assert_eq!(
            index.resolve_name_at("x", h_offset),
            index
                .symbols
                .iter()
                .find(|s| s.name == "x" && s.kind == SymbolKind::LocalBinding)
                .map(|s| s.id)
        );
    }

    #[test]
    fn call_site_sees_other_top_level_fn() {
        let src = "fn g(): Int = 1\nfn f(): Int = g()\n";
        let decls = crate::desugar_program(src).expect("parse");
        let index = build_symbol_index(&decls, src);
        let call = src.find("g()").expect("call");
        let g = index
            .symbols
            .iter()
            .find(|s| s.name == "g" && s.kind == SymbolKind::Function)
            .map(|s| s.id);
        assert_eq!(index.resolve_name_at("g", call), g);
    }
}
