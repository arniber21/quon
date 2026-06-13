// AST → quantum.circ MLIR lowering — see issue #16, SPEC.md §6
// Translates a type-checked AST to in-memory quantum.circ IR using Melior.
// Circuit<n,m,d,C> indices are encoded as op attributes (ADR-0002).

use crate::ast::Decl;
use crate::lexer::Sp;

pub struct LoweringCtx {
    // Melior MLIR context lives in mlir_bridge; this struct holds a reference
    // to it once the bridge is available.
}

impl LoweringCtx {
    pub fn lower_decls(&mut self, _decls: &[Sp<Decl>]) {
        todo!("AST → quantum.circ lowering — see issue #16")
    }
}
