// run { } desugaring pass — see issue #8, SPEC.md §3.5
// Rewrites RunBlock { stmts } to nested Bind nodes before type checking.
// Preserves all source spans.

use crate::ast::{Decl, Expr, Pat, Stmt};
use crate::diagnostics::Diagnostic;
use crate::lexer::{SimpleSpan, Sp};

/// Desugar a declaration list, rewriting every `run { }` block into nested
/// `Bind`/`Return` nodes. Malformed `run` blocks (a block that does not end in
/// an expression, or a `<-` bind whose pattern is not a single variable) are
/// reported as [`Diagnostic`]s rather than panicking; the pass collects every
/// such error before returning so callers see them all at once.
pub fn desugar_decls(decls: Vec<Sp<Decl>>) -> Result<Vec<Sp<Decl>>, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let out: Vec<Sp<Decl>> = decls
        .into_iter()
        .map(|d| desugar_decl(d, &mut errors))
        .collect();
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// Desugar a single expression. Convenience wrapper around the internal
/// recursion for stage-level tests that work with one expression at a time.
pub fn desugar_expr(expr: Sp<Expr>) -> Result<Sp<Expr>, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let out = desugar_expr_acc(expr, &mut errors);
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

/// Fold a `borrow` body — a statement sequence — into the `Bind`/`Let`/`Return` chain the
/// type checker checks as a monadic block (issue #15). This is exactly the `run { }` folding
/// (tuple-pattern binds are destructured, the block must end in an expression), reused so the
/// two monadic block forms share one desugaring. The input statements have already been
/// desugared as part of the enclosing declaration; re-running the transform on them is
/// idempotent. Malformed blocks are reported as [`Diagnostic`]s, never panicked on.
pub fn fold_monadic_block(
    stmts: &[Sp<Stmt>],
    block_span: SimpleSpan,
) -> Result<Sp<Expr>, Vec<Diagnostic>> {
    let mut errors = Vec::new();
    let out = desugar_run_block(stmts.to_vec(), block_span, &mut errors);
    if errors.is_empty() {
        Ok(out)
    } else {
        Err(errors)
    }
}

fn desugar_decl(decl: Sp<Decl>, errors: &mut Vec<Diagnostic>) -> Sp<Decl> {
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
            body: desugar_expr_acc(body, errors),
        },
        Decl::TypeAlias { name, params, ty } => Decl::TypeAlias { name, params, ty },
    };
    (d, span)
}

fn desugar_expr_acc(expr: Sp<Expr>, errors: &mut Vec<Diagnostic>) -> Sp<Expr> {
    let (e, span) = expr;
    // Recurse into a child node, re-boxing the result. Takes the child unboxed
    // (callers deref with `*`) so a `Box` never round-trips through the helper.
    fn rec(child: Sp<Expr>, errors: &mut Vec<Diagnostic>) -> Box<Sp<Expr>> {
        Box::new(desugar_expr_acc(child, errors))
    }
    let e = match e {
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unit | Expr::Var(_) => e,

        Expr::Lam { params, body } => Expr::Lam {
            params,
            body: rec(*body, errors),
        },
        Expr::App(a, b) => Expr::App(rec(*a, errors), rec(*b, errors)),
        Expr::BinOp { op, lhs, rhs } => Expr::BinOp {
            op,
            lhs: rec(*lhs, errors),
            rhs: rec(*rhs, errors),
        },
        Expr::Neg(inner) => Expr::Neg(rec(*inner, errors)),
        Expr::Let { pat, rhs, body } => Expr::Let {
            pat,
            rhs: rec(*rhs, errors),
            body: rec(*body, errors),
        },
        Expr::If { cond, then, else_ } => Expr::If {
            cond: rec(*cond, errors),
            then: rec(*then, errors),
            else_: rec(*else_, errors),
        },
        Expr::Match { scrutinee, arms } => Expr::Match {
            scrutinee: rec(*scrutinee, errors),
            arms: arms
                .into_iter()
                .map(|(p, e)| (p, desugar_expr_acc(e, errors)))
                .collect(),
        },
        Expr::For { pat, iter, body } => Expr::For {
            pat,
            iter: rec(*iter, errors),
            body: rec(*body, errors),
        },
        Expr::Tuple(exprs) => Expr::Tuple(
            exprs
                .into_iter()
                .map(|e| desugar_expr_acc(e, errors))
                .collect(),
        ),
        Expr::List(exprs) => Expr::List(
            exprs
                .into_iter()
                .map(|e| desugar_expr_acc(e, errors))
                .collect(),
        ),

        Expr::CircuitBlock(stmts) => Expr::CircuitBlock(desugar_stmts(stmts, errors)),
        Expr::Compose(a, b) => Expr::Compose(rec(*a, errors), rec(*b, errors)),
        Expr::Par(body, count) => Expr::Par(rec(*body, errors), rec(*count, errors)),
        Expr::Adjoint(inner) => Expr::Adjoint(rec(*inner, errors)),
        Expr::Controlled(inner) => Expr::Controlled(rec(*inner, errors)),
        Expr::GateApp { gate, qubits } => Expr::GateApp {
            gate: rec(*gate, errors),
            qubits: rec(*qubits, errors),
        },

        Expr::RunBlock(stmts) => {
            return desugar_run_block(stmts, span, errors);
        }

        Expr::Bind { rhs, param, body } => Expr::Bind {
            rhs: rec(*rhs, errors),
            param,
            body: rec(*body, errors),
        },
        Expr::Return(inner) => Expr::Return(rec(*inner, errors)),

        Expr::Borrow { bindings, body } => Expr::Borrow {
            bindings,
            body: desugar_stmts(body, errors),
        },
        Expr::Ascribe(inner, ty) => Expr::Ascribe(rec(*inner, errors), ty),
    };
    (e, span)
}

fn desugar_stmts(stmts: Vec<Sp<Stmt>>, errors: &mut Vec<Diagnostic>) -> Vec<Sp<Stmt>> {
    stmts
        .into_iter()
        .map(|(stmt, span)| {
            let stmt = match stmt {
                Stmt::Bind { pat, rhs } => Stmt::Bind {
                    pat,
                    rhs: desugar_expr_acc(rhs, errors),
                },
                Stmt::Let { pat, rhs } => Stmt::Let {
                    pat,
                    rhs: desugar_expr_acc(rhs, errors),
                },
                Stmt::Expr(e) => Stmt::Expr(desugar_expr_acc(e, errors)),
            };
            (stmt, span)
        })
        .collect()
}

fn desugar_run_block(
    stmts: Vec<Sp<Stmt>>,
    block_span: SimpleSpan,
    errors: &mut Vec<Diagnostic>,
) -> Sp<Expr> {
    if stmts.is_empty() {
        return (Expr::Unit, block_span);
    }

    let mut iter = stmts.into_iter().rev();

    // The final statement is the block's monadic result, so it must be an
    // expression. A trailing `<-` bind or `let` has no continuation to bind
    // into; report it and recover with the bound right-hand side so we can keep
    // collecting errors from the rest of the block.
    let (last_stmt, mut current_span) = iter.next().unwrap();
    let mut current_expr = match last_stmt {
        Stmt::Expr(e) => desugar_expr_acc(e, errors),
        Stmt::Bind { rhs, .. } => {
            errors.push(Diagnostic::new(
                "a `run` block must end in an expression, not a `<-` bind",
                current_span,
            ));
            desugar_expr_acc(rhs, errors)
        }
        Stmt::Let { rhs, .. } => {
            errors.push(Diagnostic::new(
                "a `run` block must end in an expression, not a `let` binding",
                current_span,
            ));
            desugar_expr_acc(rhs, errors)
        }
    };

    for (stmt, stmt_span) in iter {
        current_span = SimpleSpan::from(stmt_span.start..current_span.end);

        current_expr = match stmt {
            Stmt::Bind { pat, rhs } => {
                let rhs = Box::new(desugar_expr_acc(rhs, errors));
                desugar_bind(pat, rhs, current_expr, current_span, errors)
            }
            Stmt::Let { pat, rhs } => (
                Expr::Let {
                    pat,
                    rhs: Box::new(desugar_expr_acc(rhs, errors)),
                    body: Box::new(current_expr),
                },
                current_span,
            ),
            Stmt::Expr(e) => (
                Expr::Bind {
                    rhs: Box::new(desugar_expr_acc(e, errors)),
                    param: "_".to_string(),
                    body: Box::new(current_expr),
                },
                current_span,
            ),
        };
    }

    // For the outermost expression, ensure it uses the full block span.
    current_expr.1 = block_span;
    current_expr
}

/// Desugar a `pat <- rhs` bind continuing into `body` (`bind(rhs, fn(pat) -> body)`).
///
/// The [`Expr::Bind`] node carries a single `Name`, not a pattern, so:
/// * a variable (or `_`) pattern becomes the bind parameter directly;
/// * a **tuple** pattern binds a fresh variable and immediately destructures it with a `let`,
///   `(a, b) <- e; k` ⟶ `bind(e, fn($t) -> let (a, b) = $t in k)`. The reference algorithms
///   (`hello_bell`, `teleport`, `syndrome_measure`) all rely on this form. The fresh name is
///   `$`-prefixed so it cannot collide with a lexable identifier, and span-keyed so distinct
///   binds never clash.
/// * a **literal** pattern is refutable — not a valid irrefutable bind — so it is reported and
///   recovered as `_`.
fn desugar_bind(
    pat: Sp<Pat>,
    rhs: Box<Sp<Expr>>,
    body: Sp<Expr>,
    span: SimpleSpan,
    errors: &mut Vec<Diagnostic>,
) -> Sp<Expr> {
    let pat_span = pat.1;
    let (param, body) = match &pat.0 {
        Pat::Var(name) => (name.clone(), body),
        Pat::Wildcard => ("_".to_string(), body),
        Pat::Tuple(_) => {
            let tmp = format!("$bind{}", pat_span.start);
            let destructured = (
                Expr::Let {
                    pat: pat.clone(),
                    rhs: Box::new((Expr::Var(tmp.clone()), pat_span)),
                    body: Box::new(body),
                },
                span,
            );
            (tmp, destructured)
        }
        Pat::Lit(_) => {
            errors.push(Diagnostic::new(
                "a `<-` bind in a `run` block cannot bind a literal pattern",
                pat_span,
            ));
            ("_".to_string(), body)
        }
    };
    (
        Expr::Bind {
            rhs,
            param,
            body: Box::new(body),
        },
        span,
    )
}
