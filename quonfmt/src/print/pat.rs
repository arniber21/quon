use frontend::ast::{LitPat, Pat};
use frontend::lexer::Sp;

use crate::doc::Doc;
use crate::print::Context;

#[allow(clippy::only_used_in_recursion)]
pub fn print_pat(p: &Sp<Pat>, ctx: &mut Context<'_>) -> Doc {
    match &p.0 {
        Pat::Wildcard => Doc::text("_"),
        Pat::Var(n) => Doc::text(n.clone()),
        Pat::Tuple(ps) => {
            let inner: Vec<Doc> = ps.iter().map(|p| print_pat(p, ctx)).collect();
            Doc::concat([
                Doc::text("("),
                Doc::concat(
                    inner
                        .into_iter()
                        .flat_map(|d| [d, Doc::text(", ")])
                        .take(if ps.is_empty() { 0 } else { ps.len() * 2 - 1 }),
                ),
                Doc::text(")"),
            ])
        }
        Pat::Lit(LitPat::Int(n)) => Doc::text(n.to_string()),
        Pat::Lit(LitPat::Bool(b)) => Doc::text(b.to_string()),
    }
}
