//! The **refinement / Z3 obligation** module — Nat/Expr obligation generation and
//! discharge for the type checker (issue #326; SPEC §3.3, §3.6; ADR-0002, ADR-0028).
//!
//! ## Judgment form
//!
//! The Circuit judgment ([`super::circuit`], ADR-0028) *assembles* a
//! [`DepthExpr`] and the classical judgment infers a type; this module owns the
//! **obligation** layer that *discharges* them. It is the single place a symbolic
//! depth / width / Nat obligation reaches the Z3 solver, and the single place a
//! deferred numeric-type obligation is recorded and later finalized.
//!
//! ```text
//!   Γ ; Φ ⊢ d ≤ d̂        depth upper-bound obligation (SPEC §3.3)
//!   Γ ; Φ ⊢ w = ŵ        width equality obligation
//!   Γ ; Φ ⊢ arg + 1 ≤ p  well-founded recursion measure (issue #60)
//!   Γ ⊢ τ : Numeric       deferred Int/Float obligation (issue #13)
//! ```
//!
//! `Φ` is the active assumption context — the equalities/disequalities a `match`
//! arm's pattern implies about a `Nat` scrutinee (issue #59). The solver itself —
//! [`crate::refinement::RefinementCtx`], [`Assumption`], [`DepthError`] — is the
//! crate-root Z3 bridge (issue #13); this module is the typechecker's
//! *obligation-generation / discharge* layer that calls into it. The
//! [`quon_core::DepthExpr`] algebra (ADR-0002) stays in `quon_core`.
//!
//! ## What this module owns
//!
//! * **Depth verification** ([`TypeChecker::verify_depth`]) — an inferred symbolic
//!   depth against a user annotation, discharged as `inferred ≤ annotated` (depth is
//!   an upper bound, SPEC §3.3) under the active assumptions; only genuinely symbolic
//!   obligations reach Z3.
//! * **Width verification** ([`TypeChecker::verify_width`]) — an exact qubit-count
//!   equality, discharged under the active assumptions.
//! * **Well-founded termination** ([`TypeChecker::check_termination`], [`RecCall`]) —
//!   every captured self-recursive call must admit a `Nat` parameter that provably
//!   decreases (`arg + 1 ≤ p` and `0 ≤ arg`) under that call's assumptions.
//! * **Assumption recording** ([`TypeChecker::push_arm_refinement`]) — the
//!   equalities/disequalities a `match` arm's pattern implies about a `Nat`
//!   scrutinee, pushed/popped around the arm body so obligations discharge under them.
//! * **Numeric obligations** ([`TypeChecker::numeric`],
//!   [`TypeChecker::finalize_numeric`]) — "must be `Int` or `Float`" obligations
//!   deferred on unsolved metavariables and discharged (defaulting to `Int`) at the
//!   end of each body.
//!
//! ## Module boundary (ADR-0028)
//!
//! Pure code-motion carve out of the [`super::TypeChecker`] monolith: the items below
//! are methods on [`super::TypeChecker`] (they read/write the checker's `refine`,
//! `assumptions`, `rec_calls`, `current_fn`, `numeric`, and `table` state) kept as
//! a single `impl` block in this file. [`super::TypeChecker`] remains the
//! bidirectional facade that dispatches into them; the Circuit judgment and the
//! classical Γ consume refined results through these seams — they do not embed solver
//! calls. Removing the Z3 bridge touches this module (and the Circuit width check it
//! shares); classical and Circuit judgment bodies need no edits beside calling
//! through these seams. This module is named `obligation` (not `refinement`) to avoid
//! a clash with the crate-root [`crate::refinement`] Z3 bridge it calls into.

use std::collections::HashMap;

use crate::ast::{Expr, LitPat, Name, Pat};
use crate::lexer::{SimpleSpan, Sp};
use crate::refinement::{Assumption, DepthError};
use crate::types::Ty;
use quon_core::DepthExpr;

use super::TypeError;

/// A captured self-recursive call site (issue #60): the value substitution it applies to the
/// current function's `Nat` parameters (keyed by parameter name), and the refinement assumptions
/// in force there. Collected while checking a body, then replayed after to verify a well-founded
/// decreasing measure exists — some `Nat` parameter `p` whose argument is provably `< p` (and
/// `≥ 0`) at *every* recursive call.
#[derive(Clone)]
pub(super) struct RecCall {
    pub(super) sigma: HashMap<String, DepthExpr>,
    pub(super) assumptions: Vec<Assumption>,
}

// ── Refinement / Z3 obligations ───────────────────────────────────────────────
//
// All of the following are methods on `TypeChecker`, moved here from the facade monolith as a
// pure code-motion carve (ADR-0028, issue #326). They read and write the checker's shared state
// — the `refine`ment bridge, the active `assumptions`, the captured `rec_calls` and
// `current_fn`, the deferred `numeric` obligations, and the metavariable `table` — and call back
// into the facade's generic `depth_of`/`table` helpers (child-module access). Only the methods
// the facade dispatches into are `pub(super)`; there are no intra-module helpers here.

impl super::TypeChecker {
    /// Verify an inferred symbolic depth against a user-supplied annotation (issue #13). Depth is
    /// an **upper bound** (SPEC §3.3): the obligation is `inferred ≤ annotated`, discharged under
    /// the active refinement assumptions (issue #59). `prove_le` short-circuits on holes,
    /// structural equivalence, and (assumption-free) constants; only genuinely symbolic
    /// obligations reach Z3. Maps a [`DepthError`] to a span-accurate [`TypeError`].
    pub(super) fn verify_depth(
        &self,
        annotated: &DepthExpr,
        inferred: &DepthExpr,
        span: SimpleSpan,
    ) -> Result<(), TypeError> {
        match self.refine.prove_le(&self.assumptions, inferred, annotated) {
            Ok(()) => Ok(()),
            Err(DepthError::Mismatch) => Err(TypeError::DepthMismatch {
                expected: annotated.to_string(),
                found: inferred.to_string(),
                span,
            }),
            Err(DepthError::Intractable) => Err(TypeError::DepthIntractable {
                expr: format!("{inferred} <= {annotated}"),
                span,
            }),
        }
    }

    /// Verify a circuit width (qubit count) is exactly the expected one (SPEC §3.3: width is the
    /// invariant physical interface — no subtyping). Equality is discharged under the active
    /// refinement assumptions, so e.g. an arm producing width `0` satisfies an expected `n` when
    /// `n = 0` is assumed. Structural/constant fast paths inside `prove_eq` keep the common case
    /// solver-free.
    pub(super) fn verify_width(
        &self,
        expected: &DepthExpr,
        got: &DepthExpr,
        span: SimpleSpan,
    ) -> Result<(), TypeError> {
        match self.refine.prove_eq(&self.assumptions, expected, got) {
            Ok(()) => Ok(()),
            // A genuine mismatch *or* an intractable obligation both surface as a width error:
            // an undecidable width equality is, for the user, an unmet qubit-count requirement.
            Err(_) => Err(TypeError::QubitCountMismatch {
                expected: expected.to_string(),
                found: got.to_string(),
                span,
            }),
        }
    }

    /// Verify that the recursive calls captured while checking the current body admit a
    /// well-founded decreasing measure (issue #60, SPEC §3.3 "Recursive circuit functions"): a
    /// `Nat` parameter `p` whose call-site argument is provably `< p` *and* `≥ 0` at every
    /// recursive call, under that call's refinement assumptions. The `≥ 0` half is what forces a
    /// base case — without the `n ≥ 1` a `match { 0 => … }` arm supplies, `n − 1 ≥ 0` is not
    /// provable at `n = 0`, so the unguarded `f(n) = f(n−1)` is correctly rejected. A function
    /// with no captured recursive call is non-recursive here and trivially passes.
    pub(super) fn check_termination(&self, name: &Name, span: SimpleSpan) -> Result<(), TypeError> {
        if self.rec_calls.is_empty() {
            return Ok(());
        }
        let Some((_, nat_params)) = &self.current_fn else {
            return Ok(());
        };
        for p in nat_params {
            let param_var = DepthExpr::Var(p.clone());
            let decreases = self.rec_calls.iter().all(|call| {
                let Some(arg) = call.sigma.get(p) else {
                    return false;
                };
                let strictly_less = arg.clone().seq(DepthExpr::Nat(1));
                // `arg + 1 ≤ p`  (strict decrease)  and  `0 ≤ arg`  (stays in ℕ).
                self.refine
                    .prove_le(&call.assumptions, &strictly_less, &param_var)
                    .is_ok()
                    && self
                        .refine
                        .prove_le(&call.assumptions, &DepthExpr::Nat(0), arg)
                        .is_ok()
            });
            if decreases {
                return Ok(());
            }
        }
        Err(TypeError::IllFoundedRecursion {
            name: name.clone(),
            span,
        })
    }

    /// Push the refinement assumptions that a `match` arm's pattern implies about a `Nat`
    /// scrutinee (issue #59). Only a scrutinee that is statically `Int`/`Nat` *and* lowers to a
    /// [`DepthExpr`] (a variable, literal, or arithmetic over them — not, say, a function call)
    /// carries refinement; everything else pushes nothing. The caller records the pre-push length
    /// and truncates back to it, so this never has to undo its own work.
    ///
    /// - A literal arm `k =>` (`LitPat::Int`) asserts `scrut = k`.
    /// - A wildcard/variable arm asserts `scrut ≠ kᵢ` for each *sibling* `Int` literal `kᵢ`. With
    ///   the solver's global Nat domain (`≥ 0`), a `{0, _}` match yields `scrut ≥ 1` in the `_`
    ///   arm — exactly what licenses `scrut - 1` as a non-saturating predecessor.
    pub(super) fn push_arm_refinement(
        &mut self,
        scrut_ty: &Ty,
        scrutinee: &Sp<Expr>,
        pat: &Sp<Pat>,
        arms: &[(Sp<Pat>, Sp<Expr>)],
    ) {
        if !matches!(self.table.resolve(scrut_ty), Ty::Int) {
            return;
        }
        let Ok(scrut) = self.depth_of(scrutinee) else {
            return;
        };
        match &pat.0 {
            Pat::Lit(LitPat::Int(k)) if *k >= 0 => {
                self.assumptions
                    .push(Assumption::Eq(scrut, DepthExpr::Nat(*k as u64)));
            }
            Pat::Wildcard | Pat::Var(_) => {
                for (p, _) in arms {
                    if let Pat::Lit(LitPat::Int(k)) = &p.0
                        && *k >= 0
                    {
                        self.assumptions
                            .push(Assumption::Ne(scrut.clone(), DepthExpr::Nat(*k as u64)));
                    }
                }
            }
            _ => {}
        }
    }

    /// Resolve `t` and require it to be `Int` or `Float`. An unsolved metavariable defers:
    /// the obligation is recorded and the metavariable is returned unchanged, so surrounding
    /// context can still solve it. [`Self::finalize_numeric`] discharges the obligation
    /// (defaulting a still-unsolved variable to `Int`) once the body is fully checked.
    pub(super) fn numeric(&mut self, t: &Ty, span: SimpleSpan) -> Result<Ty, TypeError> {
        let resolved = self.table.resolve(t);
        match resolved {
            Ty::Int => Ok(Ty::Int),
            Ty::Float => Ok(Ty::Float),
            Ty::Meta(id) => {
                self.numeric.push((id, span));
                Ok(resolved)
            }
            other => Err(TypeError::NotNumeric { found: other, span }),
        }
    }

    /// Discharge the deferred numeric obligations collected while checking one body: every
    /// metavariable used arithmetically must resolve to `Int`/`Float`, defaulting to `Int`
    /// when nothing else constrained it. Clears the obligation list for the next body.
    pub(super) fn finalize_numeric(&mut self) -> Result<(), TypeError> {
        for (id, span) in std::mem::take(&mut self.numeric) {
            match self.table.resolve(&Ty::Meta(id)) {
                Ty::Int | Ty::Float => {}
                Ty::Meta(_) => self.table.unify(&Ty::Meta(id), &Ty::Int, span)?,
                other => return Err(TypeError::NotNumeric { found: other, span }),
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    //! Exercises the refinement/Z3 obligation seams directly as pure functions: the depth
    //! upper-bound discharge ([`TypeChecker::verify_depth`]), the width equality discharge
    //! ([`TypeChecker::verify_width`]) — including under `match`-arm assumptions — the
    //! well-founded recursion measure ([`TypeChecker::check_termination`], [`RecCall`]), and the
    //! deferred numeric obligation record/finalize cycle ([`TypeChecker::numeric`],
    //! [`TypeChecker::finalize_numeric`]). These are the exact discharge paths the facade
    //! dispatches into; a regression here flags a change in the carved obligation interface
    //! itself.

    use super::*;

    fn sp() -> SimpleSpan {
        SimpleSpan::from(0..0)
    }

    fn var(n: &str) -> DepthExpr {
        DepthExpr::Var(n.to_string())
    }

    // ── verify_depth: depth is an *upper bound* (inferred ≤ annotated) ─────────────

    #[test]
    fn verify_depth_accepts_tighter_inferred() {
        let tc = super::super::TypeChecker::new();
        // inferred 3 ≤ annotated 5 → obligation holds (constant fast path, no Z3).
        assert!(
            tc.verify_depth(&DepthExpr::Nat(5), &DepthExpr::Nat(3), sp())
                .is_ok()
        );
    }

    #[test]
    fn verify_depth_rejects_looser_inferred() {
        let tc = super::super::TypeChecker::new();
        // inferred 5 ≤ annotated 3 → counterexample → DepthMismatch.
        let err = tc
            .verify_depth(&DepthExpr::Nat(3), &DepthExpr::Nat(5), sp())
            .unwrap_err();
        assert!(matches!(err, TypeError::DepthMismatch { .. }));
    }

    // ── verify_width: exact qubit-count equality ──────────────────────────────────

    #[test]
    fn verify_width_accepts_equal_constants() {
        let tc = super::super::TypeChecker::new();
        assert!(
            tc.verify_width(&DepthExpr::Nat(2), &DepthExpr::Nat(2), sp())
                .is_ok()
        );
    }

    #[test]
    fn verify_width_rejects_unequal_constants() {
        let tc = super::super::TypeChecker::new();
        let err = tc
            .verify_width(&DepthExpr::Nat(2), &DepthExpr::Nat(3), sp())
            .unwrap_err();
        assert!(matches!(err, TypeError::QubitCountMismatch { .. }));
    }

    // ── discharge under match-arm assumptions (issue #59) ──────────────────────────

    #[test]
    fn verify_width_proves_under_eq_assumption() {
        let mut tc = super::super::TypeChecker::new();
        // `n = 0` in force → width `n` equals width `0`.
        tc.assumptions
            .push(Assumption::Eq(var("n"), DepthExpr::Nat(0)));
        assert!(tc.verify_width(&var("n"), &DepthExpr::Nat(0), sp()).is_ok());
        // …and `n` does *not* equal `1` under `n = 0`.
        assert!(
            tc.verify_width(&var("n"), &DepthExpr::Nat(1), sp())
                .is_err()
        );
    }

    #[test]
    fn verify_depth_proves_under_ne_assumption() {
        let mut tc = super::super::TypeChecker::new();
        // `n ≠ 0` (so `n ≥ 1`) in force → `1 ≤ n` holds, the predecessor-reasoning shape.
        tc.assumptions
            .push(Assumption::Ne(var("n"), DepthExpr::Nat(0)));
        assert!(tc.verify_depth(&var("n"), &DepthExpr::Nat(1), sp()).is_ok());
    }

    // ── check_termination: well-founded recursion measure (issue #60) ─────────────

    #[test]
    fn check_termination_trivially_passes_without_rec_calls() {
        let tc = super::super::TypeChecker::new();
        // No captured recursive call → non-recursive → trivially well-founded.
        assert!(tc.check_termination(&"f".to_string(), sp()).is_ok());
    }

    #[test]
    fn check_termination_accepts_decreasing_measure_under_assumption() {
        let mut tc = super::super::TypeChecker::new();
        tc.current_fn = Some(("f".to_string(), vec!["n".to_string()]));
        // Call site `f(0)` under `n = 5`: `0 + 1 ≤ 5` and `0 ≤ 0` → decreases.
        tc.rec_calls.push(RecCall {
            sigma: [("n".to_string(), DepthExpr::Nat(0))].into_iter().collect(),
            assumptions: vec![Assumption::Eq(var("n"), DepthExpr::Nat(5))],
        });
        assert!(tc.check_termination(&"f".to_string(), sp()).is_ok());
    }

    #[test]
    fn check_termination_rejects_unbounded_decrease() {
        let mut tc = super::super::TypeChecker::new();
        tc.current_fn = Some(("f".to_string(), vec!["n".to_string()]));
        // `f(0)` with no assumption: `0 + 1 ≤ n` fails at `n = 0` → ill-founded.
        tc.rec_calls.push(RecCall {
            sigma: [("n".to_string(), DepthExpr::Nat(0))].into_iter().collect(),
            assumptions: vec![],
        });
        let err = tc.check_termination(&"f".to_string(), sp()).unwrap_err();
        assert!(matches!(err, TypeError::IllFoundedRecursion { .. }));
    }

    // ── numeric obligation record / finalize (issue #13) ───────────────────────────

    #[test]
    fn numeric_accepts_int_and_float() {
        let mut tc = super::super::TypeChecker::new();
        assert_eq!(tc.numeric(&Ty::Int, sp()).unwrap(), Ty::Int);
        assert_eq!(tc.numeric(&Ty::Float, sp()).unwrap(), Ty::Float);
    }

    #[test]
    fn numeric_rejects_non_numeric() {
        let mut tc = super::super::TypeChecker::new();
        assert!(tc.numeric(&Ty::Bool, sp()).is_err());
        assert!(tc.numeric(&Ty::Unit, sp()).is_err());
    }

    #[test]
    fn finalize_numeric_defaults_unsolved_metavariable_to_int() {
        let mut tc = super::super::TypeChecker::new();
        let fresh = tc.table.fresh();
        // An unsolved metavariable used arithmetically defers…
        assert_eq!(tc.numeric(&fresh, sp()).unwrap(), fresh);
        // …and finalizes to `Int` once the body is fully checked.
        tc.finalize_numeric().unwrap();
        assert_eq!(tc.table.resolve(&fresh), Ty::Int);
    }
}
