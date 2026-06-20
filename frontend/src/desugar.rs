// run { } desugaring pass — see issue #8, SPEC.md §3.5
// Rewrites RunBlock { stmts } to nested Bind nodes before type checking.
// Preserves all source spans.

use crate::ast::{Decl, Expr};
use crate::lexer::Sp;

pub fn desugar_decls(_decls: Vec<Sp<Decl>>) -> Vec<Sp<Decl>> {
    todo!("run desugaring — see issue #8")
}

pub fn desugar_expr(_expr: Sp<Expr>) -> Sp<Expr> {
    todo!()
}
