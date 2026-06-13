// Symbolic depth arithmetic and Z3 refinement bridge — see issue #13, SPEC.md §3.6
//
// Z3 is invoked only when:
//   (a) a user depth annotation must be verified against a synthesized DepthExpr, or
//   (b) match branches produce circuits of different symbolic depths that must unify.
// Pure-constant DepthExprs never invoke Z3.

use std::sync::Arc;
use crate::ast::DepthExpr;

pub struct RefinementCtx {
    z3: Arc<z3::Context>,
}

impl RefinementCtx {
    pub fn new() -> Self {
        Self { z3: Arc::new(z3::Context::new(&z3::Config::new())) }
    }

    /// Verify that `inferred` equals `annotated` under the given variable bindings.
    /// Returns Ok(()) if the constraint holds, Err with a message otherwise.
    pub fn verify_equal(&self, _inferred: &DepthExpr, _annotated: &DepthExpr) -> Result<(), String> {
        todo!("Z3 refinement — see issue #13")
    }

    /// Verify that two symbolic depths from match branches unify.
    pub fn unify(&self, _a: &DepthExpr, _b: &DepthExpr) -> Result<DepthExpr, String> {
        todo!()
    }
}

impl DepthExpr {
    /// Combine two depths under sequential composition (|>): d1 + d2.
    pub fn seq(self, rhs: Self) -> Self {
        DepthExpr::Add(Box::new(self), Box::new(rhs))
    }

    /// Combine two depths under parallel composition (par): max(d1, d2).
    pub fn par(self, rhs: Self) -> Self {
        DepthExpr::Max(Box::new(self), Box::new(rhs))
    }

    /// Scale a depth by a repeat count k: k * d.
    pub fn repeat(k: Self, d: Self) -> Self {
        DepthExpr::Mul(Box::new(k), Box::new(d))
    }

    /// Serialize to S-expression string for MLIR DepthExprAttr.
    pub fn to_sexpr(&self) -> String {
        match self {
            DepthExpr::Lit(n)     => n.to_string(),
            DepthExpr::Var(v)     => v.clone(),
            DepthExpr::Add(a, b)  => format!("(+ {} {})", a.to_sexpr(), b.to_sexpr()),
            DepthExpr::Mul(a, b)  => format!("(* {} {})", a.to_sexpr(), b.to_sexpr()),
            DepthExpr::Max(a, b)  => format!("(max {} {})", a.to_sexpr(), b.to_sexpr()),
        }
    }

    /// Deserialize from S-expression string.
    pub fn from_sexpr(_s: &str) -> Result<Self, String> {
        todo!("DepthExpr S-expr parser")
    }
}
