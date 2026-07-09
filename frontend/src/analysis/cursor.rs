use crate::ast::{Decl, Expr, Pat, Stmt, Type};
use crate::lexer::{SimpleSpan, Sp};

/// Smallest AST node spanning `offset`, preferring leaves.
pub fn node_at_offset(decls: &[Sp<Decl>], offset: usize) -> Option<NodeAt<'_>> {
    let mut best: Option<(usize, NodeAt<'_>)> = None;
    for (decl, _) in decls {
        visit_decl(decl, offset, &mut best);
    }
    best.map(|(_, n)| n)
}

#[derive(Debug, Clone, Copy)]
pub enum NodeAt<'a> {
    Decl(&'a Decl),
    Expr(&'a Expr),
    Pat(&'a Pat),
    Type(&'a Type),
    Stmt(&'a Stmt),
    Name(&'a str, SimpleSpan),
}

fn consider<'a>(
    span: SimpleSpan,
    offset: usize,
    node: NodeAt<'a>,
    best: &mut Option<(usize, NodeAt<'a>)>,
) {
    if span.start <= offset && offset <= span.end {
        let size = span.end - span.start;
        if best.is_none_or(|(s, _)| size <= s) {
            *best = Some((size, node));
        }
    }
}

fn offset_in(span: SimpleSpan, offset: usize) -> bool {
    span.start <= offset && offset <= span.end
}

fn visit_decl<'a>(decl: &'a Decl, offset: usize, best: &mut Option<(usize, NodeAt<'a>)>) {
    match decl {
        Decl::Fn { name, body, .. } => {
            if offset_in(name.1, offset) {
                consider(name.1, offset, NodeAt::Name(&name.0, name.1), best);
            }
            visit_expr(&body.0, body.1, offset, best);
        }
        Decl::TypeAlias { name, ty, .. } => {
            if offset_in(name.1, offset) {
                consider(name.1, offset, NodeAt::Name(&name.0, name.1), best);
            }
            visit_type(&ty.0, ty.1, offset, best);
        }
    }
}

fn visit_expr<'a>(
    e: &'a Expr,
    span: SimpleSpan,
    offset: usize,
    best: &mut Option<(usize, NodeAt<'a>)>,
) {
    if !offset_in(span, offset) {
        return;
    }
    match e {
        Expr::Var(name) => {
            // Approximate ident span as whole expr span for resolution.
            consider(span, offset, NodeAt::Name(name, span), best);
        }
        Expr::Lam { params, body } => {
            for (pat, _) in params {
                visit_pat(&pat.0, pat.1, offset, best);
            }
            visit_expr(&body.0, body.1, offset, best);
        }
        Expr::Let { pat, rhs, body } => {
            visit_expr(&rhs.0, rhs.1, offset, best);
            visit_pat(&pat.0, pat.1, offset, best);
            visit_expr(&body.0, body.1, offset, best);
        }
        Expr::Bind { rhs, param, body } => {
            visit_expr(&rhs.0, rhs.1, offset, best);
            if offset_in(param.1, offset) {
                consider(param.1, offset, NodeAt::Name(&param.0, param.1), best);
            }
            visit_expr(&body.0, body.1, offset, best);
        }
        Expr::Borrow { bindings, body } => {
            for (n, t) in bindings {
                if offset_in(n.1, offset) {
                    consider(n.1, offset, NodeAt::Name(&n.0, n.1), best);
                }
                visit_type(&t.0, t.1, offset, best);
            }
            for stmt in body {
                visit_stmt(stmt, offset, best);
            }
        }
        Expr::App(a, b) | Expr::Compose(a, b) | Expr::Par(a, b) => {
            visit_expr(&a.0, a.1, offset, best);
            visit_expr(&b.0, b.1, offset, best);
        }
        Expr::GateApp { gate, qubits } => {
            visit_expr(&gate.0, gate.1, offset, best);
            visit_expr(&qubits.0, qubits.1, offset, best);
        }
        Expr::BinOp { lhs, rhs, .. } => {
            visit_expr(&lhs.0, lhs.1, offset, best);
            visit_expr(&rhs.0, rhs.1, offset, best);
        }
        Expr::If { cond, then, else_ } => {
            visit_expr(&cond.0, cond.1, offset, best);
            visit_expr(&then.0, then.1, offset, best);
            visit_expr(&else_.0, else_.1, offset, best);
        }
        Expr::Match { scrutinee, arms } => {
            visit_expr(&scrutinee.0, scrutinee.1, offset, best);
            for (pat, arm) in arms {
                visit_pat(&pat.0, pat.1, offset, best);
                visit_expr(&arm.0, arm.1, offset, best);
            }
        }
        Expr::For { pat, iter, body } => {
            visit_pat(&pat.0, pat.1, offset, best);
            visit_expr(&iter.0, iter.1, offset, best);
            visit_expr(&body.0, body.1, offset, best);
        }
        Expr::CircuitBlock(stmts) | Expr::RunBlock(stmts) => {
            for stmt in stmts {
                visit_stmt(stmt, offset, best);
            }
        }
        Expr::Neg(inner)
        | Expr::Adjoint(inner)
        | Expr::Controlled(inner)
        | Expr::Return(inner)
        | Expr::Ascribe(inner, _) => visit_expr(&inner.0, inner.1, offset, best),
        Expr::Tuple(es) | Expr::List(es) => {
            for e in es {
                visit_expr(&e.0, e.1, offset, best);
            }
        }
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unit => {
            consider(span, offset, NodeAt::Expr(e), best);
        }
    }
}

fn visit_pat<'a>(
    p: &'a Pat,
    span: SimpleSpan,
    offset: usize,
    best: &mut Option<(usize, NodeAt<'a>)>,
) {
    if !offset_in(span, offset) {
        return;
    }
    match p {
        Pat::Var(name) => consider(span, offset, NodeAt::Name(name, span), best),
        Pat::Tuple(ps) => {
            for pat in ps {
                visit_pat(&pat.0, pat.1, offset, best);
            }
        }
        Pat::Wildcard | Pat::Lit(_) => consider(span, offset, NodeAt::Pat(p), best),
    }
}

fn visit_type<'a>(
    t: &'a Type,
    span: SimpleSpan,
    offset: usize,
    best: &mut Option<(usize, NodeAt<'a>)>,
) {
    if !offset_in(span, offset) {
        return;
    }
    if let Type::Named { name, .. } = t {
        consider(span, offset, NodeAt::Name(name, span), best);
    }
    consider(span, offset, NodeAt::Type(t), best);
}

fn visit_stmt<'a>(stmt: &'a Sp<Stmt>, offset: usize, best: &mut Option<(usize, NodeAt<'a>)>) {
    let (s, span) = stmt;
    if !offset_in(*span, offset) {
        return;
    }
    match s {
        Stmt::Bind { pat, rhs } | Stmt::Let { pat, rhs } => {
            visit_pat(&pat.0, pat.1, offset, best);
            visit_expr(&rhs.0, rhs.1, offset, best);
        }
        Stmt::Expr(e) => visit_expr(&e.0, e.1, offset, best),
    }
}

/// Extract a partial identifier prefix at `offset` by scanning backward/forward.
pub fn partial_ident(src: &str, offset: usize) -> (usize, usize, String) {
    let bytes = src.as_bytes();
    let o = offset.min(bytes.len());
    let mut start = o;
    while start > 0 && is_ident_part(bytes[start - 1]) {
        start -= 1;
    }
    let mut end = o;
    while end < bytes.len() && is_ident_part(bytes[end]) {
        end += 1;
    }
    let text = src.get(start..end).unwrap_or("").to_string();
    (start, end, text)
}

fn is_ident_part(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Cursor marker helper for tests: `/*cursor*/` in source.
pub fn cursor_at(src: &str, marker: &str) -> usize {
    src.find(marker)
        .unwrap_or_else(|| panic!("marker {marker:?} not found"))
}
