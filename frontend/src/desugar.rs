// run { } desugaring pass — see issue #8, SPEC.md §3.5
// Rewrites RunBlock { stmts } to nested Bind nodes before type checking.
// Preserves all source spans.

use crate::ast::{Decl, Expr, Pat, Stmt};
use crate::lexer::Sp;

pub fn desugar_decls(decls: Vec<Sp<Decl>>) -> Vec<Sp<Decl>> {
    decls.into_iter().map(desugar_decl).collect()
}

pub fn desugar_decl(decl: Sp<Decl>) -> Sp<Decl> {
    let (d, span) = decl;
    let d = match d {
        Decl::Fn {
            name,
            params,
            ret,
            body,
        } => Decl::Fn {
            name,
            params,
            ret,
            body: desugar_expr(body),
        },
        Decl::TypeAlias { name, params, ty } => Decl::TypeAlias { name, params, ty },
    };
    (d, span)
}

pub fn desugar_expr(expr: Sp<Expr>) -> Sp<Expr> {
    let (e, span) = expr;
    let e = match e {
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unit | Expr::Var(_) => e,

        Expr::Lam { params, body } => Expr::Lam {
            params,
            body: Box::new(desugar_expr(*body)),
        },
        Expr::App(a, b) => Expr::App(Box::new(desugar_expr(*a)), Box::new(desugar_expr(*b))),
        Expr::BinOp { op, lhs, rhs } => Expr::BinOp {
            op,
            lhs: Box::new(desugar_expr(*lhs)),
            rhs: Box::new(desugar_expr(*rhs)),
        },
        Expr::Neg(inner) => Expr::Neg(Box::new(desugar_expr(*inner))),
        Expr::Let { pat, rhs, body } => Expr::Let {
            pat,
            rhs: Box::new(desugar_expr(*rhs)),
            body: Box::new(desugar_expr(*body)),
        },
        Expr::If { cond, then, else_ } => Expr::If {
            cond: Box::new(desugar_expr(*cond)),
            then: Box::new(desugar_expr(*then)),
            else_: Box::new(desugar_expr(*else_)),
        },
        Expr::Match { scrutinee, arms } => Expr::Match {
            scrutinee: Box::new(desugar_expr(*scrutinee)),
            arms: arms
                .into_iter()
                .map(|(p, e)| (p, desugar_expr(e)))
                .collect(),
        },
        Expr::For { pat, iter, body } => Expr::For {
            pat,
            iter: Box::new(desugar_expr(*iter)),
            body: Box::new(desugar_expr(*body)),
        },
        Expr::Tuple(exprs) => Expr::Tuple(exprs.into_iter().map(desugar_expr).collect()),
        Expr::List(exprs) => Expr::List(exprs.into_iter().map(desugar_expr).collect()),

        Expr::CircuitBlock(stmts) => Expr::CircuitBlock(desugar_stmts(stmts)),
        Expr::Compose(a, b) => {
            Expr::Compose(Box::new(desugar_expr(*a)), Box::new(desugar_expr(*b)))
        }
        Expr::Par(body, count) => Expr::Par(
            Box::new(desugar_expr(*body)),
            Box::new(desugar_expr(*count)),
        ),
        Expr::Adjoint(inner) => Expr::Adjoint(Box::new(desugar_expr(*inner))),
        Expr::Controlled(inner) => Expr::Controlled(Box::new(desugar_expr(*inner))),
        Expr::GateApp { gate, qubits } => Expr::GateApp {
            gate: Box::new(desugar_expr(*gate)),
            qubits: Box::new(desugar_expr(*qubits)),
        },

        Expr::RunBlock(stmts) => {
            return desugar_run_block(stmts, span);
        }

        Expr::Bind { rhs, param, body } => Expr::Bind {
            rhs: Box::new(desugar_expr(*rhs)),
            param,
            body: Box::new(desugar_expr(*body)),
        },
        Expr::Return(inner) => Expr::Return(Box::new(desugar_expr(*inner))),

        Expr::Borrow { bindings, body } => Expr::Borrow {
            bindings,
            body: desugar_stmts(body),
        },
        Expr::Ascribe(inner, ty) => Expr::Ascribe(Box::new(desugar_expr(*inner)), ty),
    };
    (e, span)
}

fn desugar_stmts(stmts: Vec<Sp<Stmt>>) -> Vec<Sp<Stmt>> {
    stmts
        .into_iter()
        .map(|(stmt, span)| {
            let stmt = match stmt {
                Stmt::Bind { pat, rhs } => Stmt::Bind {
                    pat,
                    rhs: desugar_expr(rhs),
                },
                Stmt::Let { pat, rhs } => Stmt::Let {
                    pat,
                    rhs: desugar_expr(rhs),
                },
                Stmt::Expr(e) => Stmt::Expr(desugar_expr(e)),
            };
            (stmt, span)
        })
        .collect()
}

fn desugar_run_block(stmts: Vec<Sp<Stmt>>, block_span: crate::lexer::SimpleSpan) -> Sp<Expr> {
    if stmts.is_empty() {
        return (Expr::Unit, block_span);
    }

    let mut iter = stmts.into_iter().rev();

    let (last_stmt, mut current_span) = iter.next().unwrap();
    let mut current_expr = match last_stmt {
        Stmt::Expr(e) => desugar_expr(e),
        _ => panic!("last statement in run block must be an expression"),
    };

    for (stmt, stmt_span) in iter {
        current_span = chumsky::span::SimpleSpan::from(stmt_span.start..current_span.end);

        match stmt {
            Stmt::Bind { pat, rhs } => {
                let param = match pat.0 {
                    Pat::Var(name) => name,
                    Pat::Wildcard => "_".to_string(),
                    _ => panic!("bind pattern in run block must be a variable or wildcard"),
                };

                current_expr = (
                    Expr::Bind {
                        rhs: Box::new(desugar_expr(rhs)),
                        param,
                        body: Box::new(current_expr),
                    },
                    current_span,
                );
            }
            Stmt::Let { pat, rhs } => {
                current_expr = (
                    Expr::Let {
                        pat,
                        rhs: Box::new(desugar_expr(rhs)),
                        body: Box::new(current_expr),
                    },
                    current_span,
                );
            }
            Stmt::Expr(e) => {
                current_expr = (
                    Expr::Bind {
                        rhs: Box::new(desugar_expr(e)),
                        param: "_".to_string(),
                        body: Box::new(current_expr),
                    },
                    current_span,
                );
            }
        }
    }

    // For the outermost expression, ensure it uses the full block span.
    current_expr.1 = block_span;
    current_expr
}
