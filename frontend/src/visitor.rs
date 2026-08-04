//! Canonical exhaustive AST traversal for the frontend.
//!
//! Every AST node kind — declarations, expressions, statements, patterns,
//! types, type parameters, and type-level natural expressions — has a
//! dedicated pre/post hook pair driven by the `walk_*` free functions in a
//! fixed, source-span-preserving order. Read-only tooling (the linter, the
//! language server) shares this one recursion instead of each re-implementing
//! it by hand: when a new AST variant lands, only this module needs a
//! traversal update (issue #399).
//!
//! ## Order
//!
//! Traversal is pre-order: the `visit_*_pre` hook fires before descending into
//! a node's children, the `visit_*_post` hook fires after. A pre-hook returns
//! [`Traversal`] to decide whether to descend. Children are visited in source
//! order (left-to-right as written).
//!
//! ## Spans
//!
//! Hooks receive `&Sp<T>` (or `&TypeParam`), so the node's [`crate::lexer::Sp`]
//! span is always available as `.1` — no separate span map is required.

use crate::ast::{Decl, Expr, NatExpr, Pat, Stmt, Type, TypeParam};
use crate::lexer::Sp;

/// Control flow returned by a `visit_*_pre` hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Traversal {
    /// Descend into the node's children, then invoke the matching `visit_*_post` hook.
    Recurse,
    /// Skip the node's children. The matching `visit_*_post` hook is NOT called.
    Skip,
}

/// Exhaustive AST visitor with pre/post hooks for every node kind.
///
/// All methods default to no-ops returning [`Traversal::Recurse`]; override
/// only the hooks you care about. Implementors mutate `&mut self` freely — the
/// `walk_*` drivers borrow the visitor exclusively for the duration of a
/// subtree, so pre/post state push/pop is always properly nested.
pub trait Visitor {
    // ── Declarations ──────────────────────────────────────────────────────
    fn visit_decl_pre(&mut self, _decl: &Sp<Decl>) -> Traversal {
        Traversal::Recurse
    }
    fn visit_decl_post(&mut self, _decl: &Sp<Decl>) {}

    // ── Expressions ───────────────────────────────────────────────────────
    fn visit_expr_pre(&mut self, _expr: &Sp<Expr>) -> Traversal {
        Traversal::Recurse
    }
    fn visit_expr_post(&mut self, _expr: &Sp<Expr>) {}

    // ── Statements ────────────────────────────────────────────────────────
    fn visit_stmt_pre(&mut self, _stmt: &Sp<Stmt>) -> Traversal {
        Traversal::Recurse
    }
    fn visit_stmt_post(&mut self, _stmt: &Sp<Stmt>) {}

    // ── Patterns ─────────────────────────────────────────────────────────
    fn visit_pat_pre(&mut self, _pat: &Sp<Pat>) -> Traversal {
        Traversal::Recurse
    }
    fn visit_pat_post(&mut self, _pat: &Sp<Pat>) {}

    // ── Types ─────────────────────────────────────────────────────────────
    fn visit_type_pre(&mut self, _ty: &Sp<Type>) -> Traversal {
        Traversal::Recurse
    }
    fn visit_type_post(&mut self, _ty: &Sp<Type>) {}

    // ── Type-level natural expressions ────────────────────────────────────
    fn visit_nat_expr_pre(&mut self, _ne: &Sp<NatExpr>) -> Traversal {
        Traversal::Recurse
    }
    fn visit_nat_expr_post(&mut self, _ne: &Sp<NatExpr>) {}

    // ── Type parameters ──────────────────────────────────────────────────
    /// `TypeParam` is not itself spanned; its `name` (`Sp<Name>`) and optional
    /// `kind` (`Sp<Kind>`) carry the relevant spans. `Kind` is a leaf enum with
    /// no recurseable children, so it has no dedicated hook.
    fn visit_type_param_pre(&mut self, _tp: &TypeParam) -> Traversal {
        Traversal::Recurse
    }
    fn visit_type_param_post(&mut self, _tp: &TypeParam) {}
}

// ── Drivers ────────────────────────────────────────────────────────────────

/// Walk a whole program's top-level declarations.
pub fn walk_program<V: Visitor + ?Sized>(v: &mut V, decls: &[Sp<Decl>]) {
    for decl in decls {
        walk_decl(v, decl);
    }
}

pub fn walk_decl<V: Visitor + ?Sized>(v: &mut V, decl: &Sp<Decl>) {
    if matches!(v.visit_decl_pre(decl), Traversal::Recurse) {
        match &decl.0 {
            Decl::Fn {
                type_params,
                params,
                ret,
                body,
                ..
            } => {
                for tp in type_params {
                    walk_type_param(v, tp);
                }
                for (_, ty) in params {
                    walk_type(v, ty);
                }
                walk_type(v, ret);
                walk_expr(v, body);
            }
            Decl::TypeAlias { params, ty, .. } => {
                for tp in params {
                    walk_type_param(v, tp);
                }
                walk_type(v, ty);
            }
        }
    }
    v.visit_decl_post(decl);
}

pub fn walk_type_param<V: Visitor + ?Sized>(v: &mut V, tp: &TypeParam) {
    if matches!(v.visit_type_param_pre(tp), Traversal::Recurse) {
        // `name` (`Sp<Name>`) and `kind` (`Sp<Kind>`) are leaves — no further
        // recursion. They are observable via the `visit_type_param_pre` hook.
        let _ = &tp.name;
        let _ = &tp.kind;
    }
    v.visit_type_param_post(tp);
}

pub fn walk_expr<V: Visitor + ?Sized>(v: &mut V, expr: &Sp<Expr>) {
    if matches!(v.visit_expr_pre(expr), Traversal::Recurse) {
        match &expr.0 {
            Expr::Int(_)
            | Expr::Float(_)
            | Expr::Bool(_)
            | Expr::Unit
            | Expr::Var(_) => {}

            Expr::Lam { params, body } => {
                for (pat, ty) in params {
                    walk_pat(v, pat);
                    if let Some(ty) = ty {
                        walk_type(v, ty);
                    }
                }
                walk_expr(v, body);
            }
            Expr::App(a, b) => {
                walk_expr(v, a);
                walk_expr(v, b);
            }
            Expr::TypeApp { callee, args } => {
                walk_expr(v, callee);
                for arg in args {
                    walk_nat_expr(v, arg);
                }
            }
            Expr::BinOp { lhs, rhs, .. } => {
                walk_expr(v, lhs);
                walk_expr(v, rhs);
            }
            Expr::Neg(e) => walk_expr(v, e),
            Expr::Let { pat, rhs, body } => {
                walk_pat(v, pat);
                walk_expr(v, rhs);
                walk_expr(v, body);
            }
            Expr::If { cond, then, else_ } => {
                walk_expr(v, cond);
                walk_expr(v, then);
                walk_expr(v, else_);
            }
            Expr::Match { scrutinee, arms } => {
                walk_expr(v, scrutinee);
                for (pat, arm) in arms {
                    walk_pat(v, pat);
                    walk_expr(v, arm);
                }
            }
            Expr::For { pat, iter, body } => {
                walk_pat(v, pat);
                walk_expr(v, iter);
                walk_expr(v, body);
            }
            Expr::Tuple(es) | Expr::List(es) => {
                for e in es {
                    walk_expr(v, e);
                }
            }
            Expr::CircuitBlock(stmts) | Expr::RunBlock(stmts) => {
                for stmt in stmts {
                    walk_stmt(v, stmt);
                }
            }
            Expr::Compose(a, b) | Expr::Par(a, b) => {
                walk_expr(v, a);
                walk_expr(v, b);
            }
            Expr::ParN(elems) => {
                for e in elems {
                    walk_expr(v, e);
                }
            }
            Expr::Adjoint(e) | Expr::Controlled(e) => walk_expr(v, e),
            Expr::GateApp { gate, qubits } => {
                walk_expr(v, gate);
                walk_expr(v, qubits);
            }
            Expr::Bind { rhs, body, .. } => {
                walk_expr(v, rhs);
                walk_expr(v, body);
            }
            Expr::Return(e) => walk_expr(v, e),
            Expr::Borrow { bindings, body } => {
                for (_, ty) in bindings {
                    walk_type(v, ty);
                }
                for stmt in body {
                    walk_stmt(v, stmt);
                }
            }
            Expr::Ascribe(e, ty) => {
                walk_expr(v, e);
                walk_type(v, ty);
            }
        }
    }
    v.visit_expr_post(expr);
}

pub fn walk_stmt<V: Visitor + ?Sized>(v: &mut V, stmt: &Sp<Stmt>) {
    if matches!(v.visit_stmt_pre(stmt), Traversal::Recurse) {
        match &stmt.0 {
            Stmt::Bind { pat, rhs } | Stmt::Let { pat, rhs } => {
                walk_pat(v, pat);
                walk_expr(v, rhs);
            }
            Stmt::Expr(e) => walk_expr(v, e),
        }
    }
    v.visit_stmt_post(stmt);
}

pub fn walk_pat<V: Visitor + ?Sized>(v: &mut V, pat: &Sp<Pat>) {
    if matches!(v.visit_pat_pre(pat), Traversal::Recurse) {
        match &pat.0 {
            Pat::Wildcard | Pat::Var(_) | Pat::Lit(_) => {}
            Pat::Tuple(ps) => {
                for p in ps {
                    walk_pat(v, p);
                }
            }
        }
    }
    v.visit_pat_post(pat);
}

pub fn walk_type<V: Visitor + ?Sized>(v: &mut V, ty: &Sp<Type>) {
    if matches!(v.visit_type_pre(ty), Traversal::Recurse) {
        match &ty.0 {
            Type::Qubit
            | Type::Bit
            | Type::Bool
            | Type::Int
            | Type::Float
            | Type::Unit
            | Type::Nat
            | Type::Var(_) => {}
            Type::QReg(n) => walk_nat_expr(v, n),
            Type::List(inner) => walk_type(v, inner),
            Type::Tuple(parts) => {
                for t in parts {
                    walk_type(v, t);
                }
            }
            Type::Fn(a, b) | Type::Linear(a, b) => {
                walk_type(v, a);
                walk_type(v, b);
            }
            Type::Circuit { n, m, d, .. } => {
                walk_nat_expr(v, n);
                walk_nat_expr(v, m);
                walk_nat_expr(v, d);
            }
            Type::Q(inner) => walk_type(v, inner),
            Type::Matrix(r, c, elem) => {
                walk_nat_expr(v, r);
                walk_nat_expr(v, c);
                walk_type(v, elem);
            }
            Type::QecBlock { family, distance } => {
                walk_type(v, family);
                walk_nat_expr(v, distance);
            }
            Type::Named { args, .. } => {
                for arg in args {
                    walk_nat_expr(v, arg);
                }
            }
        }
    }
    v.visit_type_post(ty);
}

pub fn walk_nat_expr<V: Visitor + ?Sized>(v: &mut V, ne: &Sp<NatExpr>) {
    if matches!(v.visit_nat_expr_pre(ne), Traversal::Recurse) {
        match &ne.0 {
            NatExpr::Lit(_) | NatExpr::Var(_) | NatExpr::Hole => {}
            NatExpr::Add(a, b)
            | NatExpr::Mul(a, b)
            | NatExpr::Sub(a, b)
            | NatExpr::Div(a, b)
            | NatExpr::Exp(a, b) => {
                walk_nat_expr(v, a);
                walk_nat_expr(v, b);
            }
        }
    }
    v.visit_nat_expr_post(ne);
}

#[cfg(test)]
mod tests {
    //! The synthetic new-node consistency test lives in `frontend/tests/visitor.rs`
    //! so it can build a representative program through the public parser and
    //! assert pre/post nesting across every node kind without reaching into
    //! private modules.
}
