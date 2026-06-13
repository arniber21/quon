// Parser — see issue #7, SPEC.md §2
// Produces Sp<Decl> from Vec<Sp<Token>> using chumsky.

use crate::ast::Decl;
use crate::lexer::Sp;

pub fn parse(_tokens: &[Sp<crate::lexer::Token>]) -> Result<Vec<Sp<Decl>>, Vec<Sp<String>>> {
    todo!("parser — see issue #7")
}
