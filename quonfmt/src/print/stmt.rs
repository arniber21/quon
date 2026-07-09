use frontend::ast::Stmt;
use frontend::lexer::Sp;

use crate::doc::Doc;
use crate::print::expr;
use crate::print::{BlockKind, Context, pat};

pub fn print_stmt_block(keyword: &str, stmts: &[Sp<Stmt>], ctx: &mut Context<'_>) -> Doc {
    let kind = match keyword {
        "circuit" => BlockKind::Circuit,
        "run" => BlockKind::Run,
        "" => ctx.block_kind,
        _ => BlockKind::None,
    };
    let mut block_ctx = ctx.with_block(kind);
    let body = format_stmts(stmts, &mut block_ctx);
    if keyword.is_empty() {
        if stmts.is_empty() {
            Doc::text("{\n}")
        } else {
            Doc::text(format!("{{\n{body}\n}}"))
        }
    } else if stmts.is_empty() {
        Doc::text(format!("{keyword} {{\n}}"))
    } else {
        Doc::text(format!("{keyword} {{\n{body}\n}}"))
    }
}

fn format_stmts(stmts: &[Sp<Stmt>], ctx: &mut Context<'_>) -> String {
    let bind_count = stmts
        .iter()
        .filter(|(s, _)| matches!(s, Stmt::Bind { .. }))
        .count();
    let max_lhs = if bind_count >= 2 {
        stmts
            .iter()
            .filter_map(|(s, _)| match s {
                Stmt::Bind { pat, .. } => Some(pat_display_width(pat, ctx)),
                _ => None,
            })
            .max()
            .unwrap_or(0)
    } else {
        0
    };

    stmts
        .iter()
        .map(|(s, _)| format!("{}{}", ctx.current_indent(), format_stmt(s, ctx, max_lhs)))
        .collect::<Vec<_>>()
        .join("\n")
}

fn pat_display_width(p: &Sp<frontend::ast::Pat>, ctx: &mut Context<'_>) -> usize {
    use crate::doc::render;
    render(
        &pat::print_pat(p, ctx),
        ctx.config.max_width,
        ctx.config.indent,
    )
    .chars()
    .count()
}

pub fn circuit_stmt_needs_parens(e: &frontend::ast::Expr) -> bool {
    matches!(
        e,
        frontend::ast::Expr::Lam { .. }
            | frontend::ast::Expr::Let { .. }
            | frontend::ast::Expr::If { .. }
            | frontend::ast::Expr::Match { .. }
            | frontend::ast::Expr::For { .. }
            | frontend::ast::Expr::Bind { .. }
            | frontend::ast::Expr::RunBlock(_)
            | frontend::ast::Expr::Borrow { .. }
    )
}

fn format_stmt(s: &Stmt, ctx: &mut Context<'_>, max_lhs: usize) -> String {
    use crate::doc::render;

    match s {
        Stmt::Bind { pat, rhs } => {
            let lhs = render(
                &pat::print_pat(pat, ctx),
                ctx.config.max_width,
                ctx.config.indent,
            );
            let rhs_s = render(
                &expr::print_expr(rhs, ctx, expr::Prec::Top),
                ctx.config.max_width,
                ctx.config.indent,
            );
            if max_lhs > 0 {
                let pad = max_lhs.saturating_sub(lhs.chars().count());
                format!("{}{} <- {rhs_s}", lhs, " ".repeat(pad))
            } else {
                format!("{lhs} <- {rhs_s}")
            }
        }
        Stmt::Let { pat, rhs } => {
            let p = render(
                &pat::print_pat(pat, ctx),
                ctx.config.max_width,
                ctx.config.indent,
            );
            let rhs_s = render(
                &expr::print_expr(rhs, ctx, expr::Prec::Top),
                ctx.config.max_width,
                ctx.config.indent,
            );
            format!("let {p} = {rhs_s}")
        }
        Stmt::Expr(e) => {
            let doc = if ctx.block_kind == BlockKind::Circuit && circuit_stmt_needs_parens(&e.0) {
                Doc::concat([
                    Doc::text("("),
                    expr::print_expr(e, ctx, expr::Prec::Top),
                    Doc::text(")"),
                ])
            } else {
                expr::print_expr(e, ctx, expr::Prec::Top)
            };
            if let frontend::ast::Expr::Return(inner) = &e.0 {
                if ctx.block_kind == BlockKind::Run {
                    render(&doc, ctx.config.max_width, ctx.config.indent)
                } else if ctx.block_kind == BlockKind::Circuit {
                    render(
                        &expr::print_expr(inner, ctx, expr::Prec::Top),
                        ctx.config.max_width,
                        ctx.config.indent,
                    )
                } else {
                    render(&doc, ctx.config.max_width, ctx.config.indent)
                }
            } else {
                render(&doc, ctx.config.max_width, ctx.config.indent)
            }
        }
    }
}
