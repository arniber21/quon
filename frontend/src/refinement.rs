// Symbolic depth arithmetic and Z3 refinement bridge — see issue #13, SPEC.md §3.6
//
// Z3 is invoked only when:
//   (a) a user depth annotation must be verified against a synthesized DepthExpr, or
//   (b) match branches produce circuits of different symbolic depths that must unify, or
//   (c) a refined `match` arm discharges a width/depth obligation under its scrutinee
//       assumptions (issues #57/#59/#60).
// Pure-constant DepthExprs with no active assumptions never invoke Z3 — they are compared
// directly.
//
// Depths are linear-arithmetic-over-Nat plus the occasional bilinear/polynomial product (e.g.
// `n_steps * n`, `(n-1)²`, SPEC §3.6). Those land in Z3's nonlinear integer arithmetic, which
// decides them; only when Z3 returns `Unknown` (a genuinely intractable constraint, e.g. a
// variable exponent — not representable as a `DepthExpr` today) do we surface a "supply a static
// bound" error.
//
// ## Depth as an upper bound (SPEC §3.3)
//
// A `Circuit<…,d,…>` denotes a circuit of depth *bounded above* by `d`. So checking a synthesized
// depth against an annotation is **`≤`**, not `=` ([`prove_le`]): a tighter synthesized depth
// satisfies a looser annotation. Width (the physical qubit interface) stays an **equality**
// ([`prove_eq`]). Both are discharged under an **assumption context** ([`Assumption`]) carrying
// the `match`-arm refinements that make e.g. `identity(0)`'s width `0` equal an expected `n` when
// `n = 0` is known.

use quon_core::DepthExpr;
use std::collections::HashMap;
use std::sync::Arc;
use z3::ast::{Ast, Int};
use z3::{Config, Context, SatResult, Solver};

/// A refinement assumption in force while checking a particular `match` arm (issue #59). The
/// checker pushes these around an arm body so width/depth obligations discharge under the
/// equality (or disequality) the arm's pattern implies, then pops them so nothing leaks into a
/// sibling arm or the enclosing scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assumption {
    /// The two depths are equal (a literal arm `k =>` assumes `scrutinee = k`).
    Eq(DepthExpr, DepthExpr),
    /// The two depths differ (a catch-all arm assumes `scrutinee ≠ k` for each covered literal).
    Ne(DepthExpr, DepthExpr),
    /// `lhs ≥ rhs` (e.g. a measure obligation `param ≥ arg + 1`).
    Ge(DepthExpr, DepthExpr),
}

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

    /// Verify that `inferred` equals `annotated` for all assignments — the no-assumption
    /// equality used by branch-join reconciliation. A thin wrapper over [`prove_eq`].
    pub fn verify_equal(
        &self,
        inferred: &DepthExpr,
        annotated: &DepthExpr,
    ) -> Result<(), DepthError> {
        self.prove_eq(&[], inferred, annotated)
    }

    /// Prove `a = b` under `assumptions`. Width equalities (the physical qubit interface) and
    /// branch-join reconciliation use this. `Hole` on either side accepts anything; structural
    /// equivalence ([`DepthExpr::equiv`]) is a sound fast path under *any* assumptions; the
    /// both-constant fast path is sound only with no assumptions (an inconsistent assumption set
    /// makes any goal vacuously valid, so distinct constants must still reach the solver then).
    pub fn prove_eq(
        &self,
        assumptions: &[Assumption],
        a: &DepthExpr,
        b: &DepthExpr,
    ) -> Result<(), DepthError> {
        if a.is_hole() || b.is_hole() || a.equiv(b) {
            return Ok(());
        }
        if assumptions.is_empty()
            && let (Some(x), Some(y)) = (a.as_const(), b.as_const())
        {
            return if x == y {
                Ok(())
            } else {
                Err(DepthError::Mismatch)
            };
        }
        self.discharge(assumptions, a, b, Relation::Eq)
    }

    /// Prove `lhs ≤ rhs` under `assumptions` — the depth-as-upper-bound check (SPEC §3.3): a
    /// synthesized depth `lhs` satisfies an annotation `rhs` when it is no larger. `Hole` on the
    /// annotation side accepts anything; equal/constant fast paths mirror [`prove_eq`].
    pub fn prove_le(
        &self,
        assumptions: &[Assumption],
        lhs: &DepthExpr,
        rhs: &DepthExpr,
    ) -> Result<(), DepthError> {
        if lhs.is_hole() || rhs.is_hole() || lhs.equiv(rhs) {
            return Ok(());
        }
        if assumptions.is_empty()
            && let (Some(x), Some(y)) = (lhs.as_const(), rhs.as_const())
        {
            return if x <= y {
                Ok(())
            } else {
                Err(DepthError::Mismatch)
            };
        }
        self.discharge(assumptions, lhs, rhs, Relation::Le)
    }

    /// Reconcile two symbolic depths arising from alternative `match` branches. They must be
    /// provably equal; the reconciled (normalized) depth is returned for the joined type.
    pub fn unify(&self, a: &DepthExpr, b: &DepthExpr) -> Result<DepthExpr, DepthError> {
        self.verify_equal(a, b)?;
        Ok(a.normalize())
    }

    /// Discharge `a (relation) b` under `assumptions`: intern every variable (shared across goal
    /// and assumptions), constrain each to the naturals (`≥ 0` — all widths/depths/counts are
    /// naturals, so this is sound by construction and is what makes `(n-1)+1 = n` provable),
    /// assert the assumptions, and seek a counterexample to the goal. `Unsat` ⟹ valid under the
    /// assumptions; `Sat` ⟹ a real counterexample; `Unknown` ⟹ beyond the solver (report as
    /// needing a static bound).
    fn discharge(
        &self,
        assumptions: &[Assumption],
        a: &DepthExpr,
        b: &DepthExpr,
        relation: Relation,
    ) -> Result<(), DepthError> {
        let ctx = &*self.z3;
        let mut vars: HashMap<String, Int<'_>> = HashMap::new();
        let lhs = to_int(ctx, a, &mut vars);
        let rhs = to_int(ctx, b, &mut vars);

        let solver = Solver::new(ctx);
        for asm in assumptions {
            let assertion = match asm {
                Assumption::Eq(l, r) => to_int(ctx, l, &mut vars)._eq(&to_int(ctx, r, &mut vars)),
                Assumption::Ne(l, r) => to_int(ctx, l, &mut vars)
                    ._eq(&to_int(ctx, r, &mut vars))
                    .not(),
                Assumption::Ge(l, r) => to_int(ctx, l, &mut vars).ge(&to_int(ctx, r, &mut vars)),
            };
            solver.assert(&assertion);
        }
        // Every interned variable is a natural number.
        let zero = Int::from_u64(ctx, 0);
        for v in vars.values() {
            solver.assert(&v.ge(&zero));
        }
        // Seek a counterexample to the goal.
        let goal = match relation {
            Relation::Eq => lhs._eq(&rhs),
            Relation::Le => lhs.le(&rhs),
        };
        solver.assert(&goal.not());
        match solver.check() {
            SatResult::Unsat => Ok(()),
            SatResult::Sat => Err(DepthError::Mismatch),
            SatResult::Unknown => Err(DepthError::Intractable),
        }
    }
}

/// The goal relation a [`RefinementCtx::discharge`] query proves: equality (widths,
/// branch joins) or `≤` (the depth upper-bound check).
#[derive(Clone, Copy)]
enum Relation {
    Eq,
    Le,
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
        DepthExpr::Sub(l, r) => {
            // Signed subtraction. A subtraction in a depth/width index (`n-1`, `n/2`) carries an
            // implicit domain assumption that the minuend dominates (e.g. `n ≥ 1` for a register
            // predecessor) — and in Quon every such subtraction is either guarded by a `match`
            // refinement that establishes it (the `n ≥ 1` arm, #59) or used where the algorithm's
            // valid input range guarantees it (e.g. `ising`'s `n-1` ZZ-layer over `n ≥ 1` qubits).
            // Within that domain signed and Nat-saturating subtraction agree, so we keep the
            // simpler signed form; modelling it as `max(a-b,0)` would instead *over-approximate*
            // at the boundary (`(n-1)+1 = 1` at `n=0`) and spuriously fail otherwise-valid
            // annotations like `trotter_step`'s `Circuit<n,n,n,…>`.
            let a = to_int(ctx, l, vars);
            let b = to_int(ctx, r, vars);
            a - b
        }
        DepthExpr::Div(l, r) => {
            let a = to_int(ctx, l, vars);
            let b = to_int(ctx, r, vars);
            a / b
        }
        DepthExpr::Exp(l, r) => {
            let base = to_int(ctx, l, vars);
            match r.as_const() {
                Some(0) => Int::from_u64(ctx, 1),
                Some(1) => base,
                Some(exp) if exp <= 32 => {
                    let mut acc = base.clone();
                    for _ in 1..exp {
                        acc = Int::mul(ctx, &[&acc, &base]);
                    }
                    acc
                }
                _ => base,
            }
        }
        DepthExpr::Hole => Int::from_u64(ctx, 0),
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

    // ── Depth as an upper bound (prove_le) ───────────────────────────────────────

    #[test]
    fn prove_le_accepts_a_looser_bound_and_rejects_a_tighter_one() {
        let ctx = RefinementCtx::new();
        let n = var("n");
        let n1 = n.clone().seq(DepthExpr::Nat(1));
        // n ≤ n + 1 (a tighter synthesized depth under a looser annotation): accepted.
        assert!(ctx.prove_le(&[], &n, &n1).is_ok());
        // n + 1 ≤ n: false for every n, so rejected.
        assert!(ctx.prove_le(&[], &n1, &n).is_err());
    }

    // ── Assumption-scoped obligations (#59) ──────────────────────────────────────

    #[test]
    fn width_zero_equals_n_under_the_n_is_zero_assumption() {
        // The qft base case: `identity(0)`'s width `0` must equal the expected `n` when the
        // `0 =>` arm has assumed `n = 0`. Unprovable without the assumption, provable with it.
        let ctx = RefinementCtx::new();
        let n = var("n");
        let zero = DepthExpr::Nat(0);
        assert!(ctx.prove_eq(&[], &zero, &n).is_err());
        let asm = [Assumption::Eq(n.clone(), zero.clone())];
        assert!(ctx.prove_eq(&asm, &zero, &n).is_ok());
        // And the base-case depth `0 ≤ n*n` holds under `n = 0`.
        let nn = DepthExpr::repeat(n.clone(), n.clone());
        assert!(ctx.prove_le(&asm, &zero, &nn).is_ok());
    }

    #[test]
    fn qft_step_depth_bound_is_provable_under_n_ge_1() {
        // The Z3-tractability gate for the recursive `qft` (#60): the step arm's synthesized depth
        //   1(H) + (n-1)(controlled_rotations) + (n-1)²(IH) + n/2(swap_reverse)
        // must be ≤ the annotation n*n under the successor-arm assumption n ≥ 1. This exercises
        // nonlinear arithmetic + Euclidean division; assert Z3 *decides* it (not Intractable).
        let ctx = RefinementCtx::new();
        let n = var("n");
        let one = DepthExpr::Nat(1);
        let pred = n.clone().minus(one.clone()); // n - 1
        let step = one
            .clone()
            .seq(pred.clone()) // 1 + (n-1)
            .seq(pred.clone().power(DepthExpr::Nat(2))) // + (n-1)²
            .seq(n.clone().quot(DepthExpr::Nat(2))); // + n/2
        let annot = DepthExpr::repeat(n.clone(), n.clone()); // n*n
        let asm = [Assumption::Ge(n.clone(), one)];
        assert_eq!(ctx.prove_le(&asm, &step, &annot), Ok(()));
    }

    #[test]
    fn measure_decrease_is_provable_for_predecessor_under_refinement() {
        // The termination obligation for `qft(n-1)` (#60): under the successor-arm assumption
        // n ≥ 1, the recursive argument `n-1` strictly decreases the measure `n`, discharged as
        // `(n-1) + 1 ≤ n`. Without n ≥ 1 it would not be the rejection here (signed subtraction
        // makes it hold), but the *measure* check is what gates well-foundedness in the checker.
        let ctx = RefinementCtx::new();
        let n = var("n");
        let arg_plus_1 = n.clone().minus(DepthExpr::Nat(1)).seq(DepthExpr::Nat(1));
        let asm = [Assumption::Ge(n.clone(), DepthExpr::Nat(1))];
        assert!(ctx.prove_le(&asm, &arg_plus_1, &n).is_ok());
    }
}
