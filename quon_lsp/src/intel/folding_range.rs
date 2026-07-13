use frontend::analysis::DocumentAnalysis;
use frontend::ast::{Decl, Expr, Stmt};
use frontend::lexer::{SimpleSpan, Sp};
use tower_lsp::lsp_types::FoldingRange;

use crate::convert::offset_to_position;

/// Folding ranges for `circuit` / `run` / `borrow` / `match` / `for` and multi-line decls.
pub fn folding_ranges(analysis: &DocumentAnalysis) -> Option<Vec<FoldingRange>> {
    let mut ranges = Vec::new();
    for (decl, decl_span) in &analysis.decls {
        push_fold(&mut ranges, &analysis.src, *decl_span);
        match decl {
            Decl::Fn { body, .. } => walk_expr(body, &analysis.src, &mut ranges),
            Decl::TypeAlias { .. } => {}
        }
    }
    if ranges.is_empty() {
        None
    } else {
        Some(ranges)
    }
}

fn walk_expr(expr: &Sp<Expr>, src: &str, out: &mut Vec<FoldingRange>) {
    let (e, span) = expr;
    match e {
        Expr::CircuitBlock(stmts) | Expr::RunBlock(stmts) => {
            push_fold(out, src, *span);
            walk_stmts(stmts, src, out);
        }
        Expr::Borrow { body, .. } => {
            push_fold(out, src, *span);
            walk_stmts(body, src, out);
        }
        Expr::Match { scrutinee, arms } => {
            push_fold(out, src, *span);
            walk_expr(scrutinee, src, out);
            for (pat, arm) in arms {
                let _ = pat;
                walk_expr(arm, src, out);
            }
        }
        Expr::For { pat, iter, body } => {
            let _ = pat;
            push_fold(out, src, *span);
            walk_expr(iter, src, out);
            walk_expr(body, src, out);
        }
        // Desugared `run { … }` keeps the original block span on the outermost Bind/Let.
        Expr::Bind { rhs, body, .. } => {
            if looks_like_keyword(src, *span, "run") {
                push_fold(out, src, *span);
            }
            walk_expr(rhs, src, out);
            walk_expr(body, src, out);
        }
        Expr::Let { pat, rhs, body } => {
            let _ = pat;
            // Nested `let … in` spanning multiple lines is a useful fold region.
            if span_multiline(src, *span) {
                push_fold(out, src, *span);
            }
            walk_expr(rhs, src, out);
            walk_expr(body, src, out);
        }
        Expr::If { cond, then, else_ } => {
            if span_multiline(src, *span) {
                push_fold(out, src, *span);
            }
            walk_expr(cond, src, out);
            walk_expr(then, src, out);
            walk_expr(else_, src, out);
        }
        Expr::Lam { params, body } => {
            let _ = params;
            if span_multiline(src, *span) {
                push_fold(out, src, *span);
            }
            walk_expr(body, src, out);
        }
        Expr::App(a, b)
        | Expr::Compose(a, b)
        | Expr::Par(a, b)
        | Expr::GateApp {
            gate: a, qubits: b, ..
        }
        | Expr::BinOp { lhs: a, rhs: b, .. } => {
            walk_expr(a, src, out);
            walk_expr(b, src, out);
        }
        Expr::Neg(inner)
        | Expr::Adjoint(inner)
        | Expr::Controlled(inner)
        | Expr::Return(inner)
        | Expr::Ascribe(inner, _) => walk_expr(inner, src, out),
        Expr::Tuple(es) | Expr::List(es) => {
            for e in es {
                walk_expr(e, src, out);
            }
        }
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unit | Expr::Var(_) => {}
    }
}

fn walk_stmts(stmts: &[Sp<Stmt>], src: &str, out: &mut Vec<FoldingRange>) {
    for (stmt, _) in stmts {
        match stmt {
            Stmt::Bind { rhs, .. } | Stmt::Let { rhs, .. } => walk_expr(rhs, src, out),
            Stmt::Expr(e) => walk_expr(e, src, out),
        }
    }
}

fn looks_like_keyword(src: &str, span: SimpleSpan, kw: &str) -> bool {
    let start = span.start.min(src.len());
    let end = span.end.min(src.len());
    if start >= end {
        return false;
    }
    src[start..end].trim_start().starts_with(kw)
}

fn span_multiline(src: &str, span: SimpleSpan) -> bool {
    let start = offset_to_position(src, span.start);
    let end = offset_to_position(src, span.end);
    end.line > start.line
}

fn push_fold(out: &mut Vec<FoldingRange>, src: &str, span: SimpleSpan) {
    let start = offset_to_position(src, span.start);
    let end = offset_to_position(src, span.end);
    if end.line <= start.line {
        return;
    }
    // Prefer folding interior lines so the header keyword stays visible.
    let end_line = if end.character == 0 && end.line > start.line {
        end.line - 1
    } else {
        end.line
    };
    if end_line <= start.line {
        return;
    }
    out.push(FoldingRange {
        start_line: start.line,
        start_character: Some(start.character),
        end_line,
        end_character: Some(end.character),
        kind: None,
        collapsed_text: None,
    });
}
