use frontend::ast::Decl;
use frontend::lexer::Sp;

use crate::doc::Doc;
use crate::print::Context;
use crate::print::{expr, ty};

pub fn print_decls(decls: &[Sp<Decl>], ctx: &mut Context<'_>) -> Doc {
    let parts: Vec<Doc> = decls.iter().map(|(d, _)| print_decl(d, ctx)).collect();
    if parts.is_empty() {
        Doc::nil()
    } else {
        let n = parts.len();
        Doc::concat(
            parts
                .into_iter()
                .flat_map(|d| [d, Doc::text(ctx.config.decl_sep)])
                .take(n * 2 - 1),
        )
    }
}

fn print_decl(d: &Decl, ctx: &mut Context<'_>) -> Doc {
    match d {
        Decl::Fn {
            name,
            params,
            ret,
            body,
        } => {
            let param_docs: Vec<Doc> = params
                .iter()
                .map(|(n, t)| {
                    Doc::concat([
                        Doc::text(n.0.clone()),
                        Doc::text(": "),
                        ty::print_ty(t, ctx),
                    ])
                })
                .collect();
            let params_doc = Doc::group(Doc::concat([
                Doc::text("("),
                comma_sep(param_docs),
                Doc::text(")"),
            ]));
            Doc::group(Doc::concat([
                Doc::text(format!("fn {}", name.0)),
                params_doc,
                Doc::text(": "),
                ty::print_ty(ret, ctx),
                Doc::text(" = "),
                expr::print_expr(body, ctx, expr::Prec::Top),
            ]))
        }
        Decl::TypeAlias { name, params, ty } => {
            let head = if params.is_empty() {
                format!("type {}", name.0)
            } else {
                format!(
                    "type {}<{}>",
                    name.0,
                    params
                        .iter()
                        .map(|p| p.0.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            Doc::group(Doc::concat([
                Doc::text(head),
                Doc::text(" = "),
                ty::print_ty(ty, ctx),
            ]))
        }
    }
}

fn comma_sep(items: Vec<Doc>) -> Doc {
    let n = items.len();
    if n == 0 {
        Doc::nil()
    } else {
        Doc::concat(
            items
                .into_iter()
                .flat_map(|d| [d, Doc::text(", ")])
                .take(n * 2 - 1),
        )
    }
}
