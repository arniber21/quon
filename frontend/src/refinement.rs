// Symbolic depth arithmetic and Z3 refinement bridge — see issue #13, SPEC.md §3.6
//
// Z3 is invoked only when:
//   (a) a user depth annotation must be verified against a synthesized DepthExpr, or
//   (b) match branches produce circuits of different symbolic depths that must unify.
// Pure-constant DepthExprs never invoke Z3 — they are compared directly.
//
// Depths are linear-arithmetic-over-Nat plus the occasional bilinear product (e.g.
// `n_steps * n`, SPEC §3.6). Those land in Z3's nonlinear integer arithmetic, which decides
// them; only when Z3 returns `Unknown` (a genuinely intractable constraint, e.g. a variable
// exponent — not representable as a `DepthExpr` today) do we surface a "supply a static
// bound" error.

use quon_core::DepthExpr;
use std::collections::HashMap;
use std::sync::Arc;
use z3::ast::{Ast, Int};
use z3::{Config, Context, SatResult, Solver};

pub struct RefinementCtx {
    z3: Arc<Context>,
}

/// Why a symbolic depth equality failed. The checker turns these into span-accurate
/// [`crate::typecheck::TypeError`]s; this type stays free of source-position concerns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepthError {
    /// A concrete counterexample exists — the two depths genuinely differ.
    Mismatch,
    /// Z3 could not decide the constraint (e.g. an intractable nonlinear term). The user
    /// should supply a static depth bound.
    Intractable,
}

impl Default for RefinementCtx {
    fn default() -> Self {
        Self::new()
    }
}

impl RefinementCtx {
    pub fn new() -> Self {
        Self {
            z3: Arc::new(Context::new(&Config::new())),
        }
    }

    /// Verify that `inferred` equals `annotated` for *all* assignments of their runtime
    /// variables. Returns `Ok(())` when the equality is universally valid.
    ///
    /// Pure-constant operands are compared directly without ever touching Z3 (so the common
    /// fully-static circuit pays no solver cost). Otherwise we ask Z3 whether the two depths
    /// can *differ*: if no counterexample exists (`Unsat`) they are equal; a counterexample
    /// (`Sat`) is a real mismatch; `Unknown` means the constraint is beyond what the solver
    /// can decide, which we report as a request for a static bound.
    pub fn verify_equal(
        &self,
        inferred: &DepthExpr,
        annotated: &DepthExpr,
    ) -> Result<(), DepthError> {
        // Fast path: both sides constant — no solver needed.
        if let (Some(a), Some(b)) = (inferred.as_const(), annotated.as_const()) {
            return if a == b {
                Ok(())
            } else {
                Err(DepthError::Mismatch)
            };
        }

        let ctx = &*self.z3;
        let mut vars: HashMap<String, Int<'_>> = HashMap::new();
        let lhs = to_int(ctx, inferred, &mut vars);
        let rhs = to_int(ctx, annotated, &mut vars);

        let solver = Solver::new(ctx);
        // Seek an assignment of the free variables under which the depths disagree.
        solver.assert(&lhs._eq(&rhs).not());
        match solver.check() {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => Err(DepthError::Mismatch),
            SatResult::Unknown => Err(DepthError::Intractable),
        }
    }

    /// Reconcile two symbolic depths arising from alternative `match` branches. They must be
    /// provably equal; the reconciled (normalized) depth is returned for the joined type.
    pub fn unify(&self, a: &DepthExpr, b: &DepthExpr) -> Result<DepthExpr, DepthError> {
        self.verify_equal(a, b)?;
        Ok(a.normalize())
    }
}

/// Translate a [`DepthExpr`] into a Z3 integer term, interning each distinct variable as a
/// single Z3 constant (so repeated occurrences of `n` refer to the same unknown).
fn to_int<'ctx>(
    ctx: &'ctx Context,
    e: &DepthExpr,
    vars: &mut HashMap<String, Int<'ctx>>,
) -> Int<'ctx> {
    match e {
        DepthExpr::Nat(n) => Int::from_u64(ctx, *n),
        DepthExpr::Var(name) => vars
            .entry(name.clone())
            .or_insert_with(|| Int::new_const(ctx, name.as_str()))
            .clone(),
        DepthExpr::Add(l, r) => {
            let a = to_int(ctx, l, vars);
            let b = to_int(ctx, r, vars);
            Int::add(ctx, &[&a, &b])
        }
        DepthExpr::Mul(l, r) => {
            let a = to_int(ctx, l, vars);
            let b = to_int(ctx, r, vars);
            Int::mul(ctx, &[&a, &b])
        }
        DepthExpr::Max(l, r) => {
            let a = to_int(ctx, l, vars);
            let b = to_int(ctx, r, vars);
            // Z3 has no native max: max(a, b) = if a <= b then b else a.
            a.le(&b).ite(&b, &a)
        }
    }
}

// The `DepthExpr` algebra (`seq`/`par`/`repeat`/`controlled`) and the S-expr
// codec (`to_sexpr`/`parse`) live in `quon_core`, shared with `mlir_bridge`.

#[cfg(test)]
mod tests {
    use super::*;

    fn var(s: &str) -> DepthExpr {
        DepthExpr::Var(s.into())
    }

    #[test]
    fn constants_compare_without_solver() {
        let ctx = RefinementCtx::new();
        assert!(
            ctx.verify_equal(&DepthExpr::Nat(2), &DepthExpr::Nat(2))
                .is_ok()
        );
        assert!(
            ctx.verify_equal(&DepthExpr::Nat(2), &DepthExpr::Nat(3))
                .is_err()
        );
    }

    #[test]
    fn proves_commutativity_and_associativity() {
        let ctx = RefinementCtx::new();
        // n + 1 == 1 + n
        let lhs = var("n").seq(DepthExpr::Nat(1));
        let rhs = DepthExpr::Nat(1).seq(var("n"));
        assert!(ctx.verify_equal(&lhs, &rhs).is_ok());
    }

    #[test]
    fn rejects_genuine_symbolic_mismatch() {
        let ctx = RefinementCtx::new();
        // n + 1 != n + 2
        let lhs = var("n").seq(DepthExpr::Nat(1));
        let rhs = var("n").seq(DepthExpr::Nat(2));
        assert!(ctx.verify_equal(&lhs, &rhs).is_err());
    }

    #[test]
    fn accepts_bilinear_product() {
        let ctx = RefinementCtx::new();
        // n_steps * n == n * n_steps  (SPEC §3.6 ising_evolve shape)
        let lhs = DepthExpr::repeat(var("n_steps"), var("n"));
        let rhs = DepthExpr::repeat(var("n"), var("n_steps"));
        assert!(ctx.verify_equal(&lhs, &rhs).is_ok());
    }

    #[test]
    fn unify_returns_normalized_depth() {
        let ctx = RefinementCtx::new();
        let a = var("n").seq(DepthExpr::Nat(0));
        let b = var("n");
        assert_eq!(ctx.unify(&a, &b), Ok(var("n").normalize()));
    }
}
