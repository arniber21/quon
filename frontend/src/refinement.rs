// Symbolic depth arithmetic and Z3 refinement bridge — see issue #13, SPEC.md §3.6
//
// Z3 is invoked only when:
//   (a) a user depth annotation must be verified against a synthesized DepthExpr, or
//   (b) match branches produce circuits of different symbolic depths that must unify.
// Pure-constant DepthExprs never invoke Z3.

use quon_core::DepthExpr;
use std::sync::Arc;

pub struct RefinementCtx {
    z3: Arc<z3::Context>,
}

impl RefinementCtx {
    pub fn new() -> Self {
        Self {
            z3: Arc::new(z3::Context::new(&z3::Config::new())),
        }
    }

    /// Verify that `inferred` equals `annotated` under the given variable bindings.
    /// Returns Ok(()) if the constraint holds, Err with a message otherwise.
    pub fn verify_equal(
        &self,
        _inferred: &DepthExpr,
        _annotated: &DepthExpr,
    ) -> Result<(), String> {
        todo!("Z3 refinement — see issue #13")
    }

    /// Verify that two symbolic depths from match branches unify.
    pub fn unify(&self, _a: &DepthExpr, _b: &DepthExpr) -> Result<DepthExpr, String> {
        todo!()
    }
}

// The `DepthExpr` algebra (`seq`/`par`/`repeat`/`controlled`) and the S-expr
// codec (`to_sexpr`/`parse`) live in `quon_core`, shared with `mlir_bridge`.
