// Shared test support: normalize every span in an AST to 0..0 so that structural
// equality (derived PartialEq) compares trees while ignoring source positions.

#![allow(dead_code)]

use frontend::ast::*;
use frontend::lexer::{SimpleSpan, Sp};

fn z() -> SimpleSpan {
    (0..0).into()
}

/// Recurse into a `Sp<_>` (or `Box<Sp<_>>`), zero its span, and strip its contents.
macro_rules! go {
    ($b:expr, $f:ident) => {{
        $b.1 = z();
        $f(&mut $b.0);
    }};
}

pub fn strip_decls(decls: &mut [Sp<Decl>]) {
    for d in decls.iter_mut() {
        go!(d, strip_decl);
    }
}

fn strip_decl(d: &mut Decl) {
    match d {
        Decl::Fn {
            params, ret, body, ..
        } => {
            for (_, t) in params.iter_mut() {
                go!(t, strip_ty);
            }
            go!(ret, strip_ty);
            go!(body, strip_expr);
        }
        Decl::TypeAlias { ty, .. } => go!(ty, strip_ty),
    }
}

fn strip_nat(n: &mut NatExpr) {
    match n {
        NatExpr::Lit(_) | NatExpr::Var(_) | NatExpr::Hole => {}
        NatExpr::Add(a, b)
        | NatExpr::Sub(a, b)
        | NatExpr::Mul(a, b)
        | NatExpr::Div(a, b)
        | NatExpr::Exp(a, b) => {
            go!(a, strip_nat);
            go!(b, strip_nat);
        }
    }
}

fn strip_ty(t: &mut Type) {
    match t {
        Type::Qubit
        | Type::Bit
        | Type::Bool
        | Type::Int
        | Type::Float
        | Type::Unit
        | Type::Nat
        | Type::Var(_) => {}
        Type::QReg(n) => go!(n, strip_nat),
        Type::Q(b) | Type::List(b) => go!(b, strip_ty),
        Type::Matrix(n, m, t) => {
            go!(n, strip_nat);
            go!(m, strip_nat);
            go!(t, strip_ty);
        }
        Type::Circuit { n, m, d, .. } => {
            go!(n, strip_nat);
            go!(m, strip_nat);
            go!(d, strip_nat);
        }
        Type::Fn(a, b) | Type::Linear(a, b) => {
            go!(a, strip_ty);
            go!(b, strip_ty);
        }
        Type::Tuple(ts) => {
            for t in ts.iter_mut() {
                go!(t, strip_ty);
            }
        }
        Type::Named { args, .. } => {
            for a in args.iter_mut() {
                go!(a, strip_nat);
            }
        }
    }
}

fn strip_pat(p: &mut Pat) {
    match p {
        Pat::Wildcard | Pat::Var(_) | Pat::Lit(_) => {}
        Pat::Tuple(ps) => {
            for p in ps.iter_mut() {
                go!(p, strip_pat);
            }
        }
    }
}

fn strip_stmt(s: &mut Stmt) {
    match s {
        Stmt::Bind { pat, rhs } | Stmt::Let { pat, rhs } => {
            go!(pat, strip_pat);
            go!(rhs, strip_expr);
        }
        Stmt::Expr(e) => go!(e, strip_expr),
    }
}

fn strip_expr(e: &mut Expr) {
    match e {
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unit | Expr::Var(_) => {}
        Expr::App(f, x) => {
            go!(f, strip_expr);
            go!(x, strip_expr);
        }
        Expr::BinOp { lhs, rhs, .. } => {
            go!(lhs, strip_expr);
            go!(rhs, strip_expr);
        }
        Expr::Neg(x) | Expr::Adjoint(x) | Expr::Controlled(x) | Expr::Return(x) => {
            go!(x, strip_expr)
        }
        Expr::Lam { params, body } => {
            for (p, t) in params.iter_mut() {
                go!(p, strip_pat);
                if let Some(t) = t {
                    go!(t, strip_ty);
                }
            }
            go!(body, strip_expr);
        }
        Expr::Let { pat, rhs, body } => {
            go!(pat, strip_pat);
            go!(rhs, strip_expr);
            go!(body, strip_expr);
        }
        Expr::If { cond, then, else_ } => {
            go!(cond, strip_expr);
            go!(then, strip_expr);
            go!(else_, strip_expr);
        }
        Expr::Match { scrutinee, arms } => {
            go!(scrutinee, strip_expr);
            for (p, e) in arms.iter_mut() {
                go!(p, strip_pat);
                go!(e, strip_expr);
            }
        }
        Expr::For { pat, iter, body } => {
            go!(pat, strip_pat);
            go!(iter, strip_expr);
            go!(body, strip_expr);
        }
        Expr::Tuple(es) | Expr::List(es) => {
            for e in es.iter_mut() {
                go!(e, strip_expr);
            }
        }
        Expr::CircuitBlock(ss) | Expr::RunBlock(ss) => {
            for s in ss.iter_mut() {
                go!(s, strip_stmt);
            }
        }
        Expr::Compose(a, b) | Expr::Par(a, b) => {
            go!(a, strip_expr);
            go!(b, strip_expr);
        }
        Expr::GateApp { gate, qubits } => {
            go!(gate, strip_expr);
            go!(qubits, strip_expr);
        }
        Expr::Bind { rhs, body, .. } => {
            go!(rhs, strip_expr);
            go!(body, strip_expr);
        }
        Expr::Borrow { bindings, body } => {
            for (_, t) in bindings.iter_mut() {
                go!(t, strip_ty);
            }
            for s in body.iter_mut() {
                go!(s, strip_stmt);
            }
        }
        Expr::Ascribe(x, t) => {
            go!(x, strip_expr);
            go!(t, strip_ty);
        }
    }
}

/// Parse source into a span-stripped AST, panicking with diagnostics on failure.
pub fn parse_stripped(src: &str) -> Vec<Sp<Decl>> {
    let tokens = frontend::lexer::lex(src).expect("lex failed");
    let mut decls = frontend::parser::parse(&tokens).expect("parse failed");
    strip_decls(&mut decls);
    decls
}
