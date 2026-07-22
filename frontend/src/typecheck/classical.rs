//! The **classical Γ** typing module — the unrestricted, non-quantum judgment: synthesis and
//! checking of classical expressions, first-order unification coordination, pattern binding,
//! and exhaustiveness/reachability analysis for `match` (issues #9, #324; SPEC §3.8).
//!
//! ## Judgment form
//!
//! The classical fragment is the bidirectional judgment over the **unrestricted context** `Γ`
//! ([`super::Env`], names mapped to monomorphic types) — the values that may be used any
//! number of times, as opposed to the **linear context** `Δ` ([`super::Delta`]) which tracks
//! qubit resources consumed exactly once (CONTEXT.md "Linear context"). This module owns the
//! *classical* judgment:
//!
//! ```text
//!   Γ ; Δ ⊢ e ⇒ τ      synthesis: read the type off a classical term bottom-up
//!   Γ ; Δ ⊢ e ⇐ τ      checking:  push an expected type top-down
//! ```
//!
//! `Γ` holds classical names; `Δ` threads through every method so linear resources are not
//! lost (a classical `if`/`match` branches the residual `Δ` and joins it). The **Circuit**
//! judgment (`Circuit<n, m, d, C>`), the **Quantum Monad** (`Q<τ>`, `<-` binds, `run { }`),
//! the **borrow** block, and the Z3-backed **refinement** bridge are *other slices*
//! (#323/ADR-0028, #325, #326) and stay in [`super::TypeChecker`]; only the classical
//! judgment lives here.
//!
//! ## What this module owns
//!
//! * **Arithmetic** — `+`/`-`/`*`/`/`/`^` over `Int`/`Float` with deferred numeric metavariable
//!   resolution ([`TypeChecker::synth_arith`], [`TypeChecker::synth_pow`],
//!   [`TypeChecker::numeric`], [`TypeChecker::finalize_numeric`]).
//! * **Lists** — list synthesis with element unification
//!   ([`TypeChecker::synth_list`]); tuple construction and checking live in the facade
//!   because they double as the quantum tensor-introduction form (`Qubit`-tuples → `QReg<n>`).
//! * **Lambdas** — synthesis (annotated parameters only) and checking (peel one arrow per
//!   parameter) with a fresh linear context per body and capture-error framing
//!   ([`TypeChecker::synth_lambda`], [`TypeChecker::check_lambda`],
//!   [`TypeChecker::in_lambda_scope`]).
//! * **Branches** — `if`/`then`/`else` in synthesis and checking, residual `Δ` join
//!   ([`TypeChecker::branch_if`], [`TypeChecker::merge_branches`]); the Circuit branch
//!   depth join (`max`) is borrowed from the circuit slice ([`TypeChecker::join_branch_types`]).
//! * **Match** — scrutinee synthesis, arm checking/synthesis, dependent `Nat` refinement
//!   assumption push, and **exhaustiveness + reachability** via [`super::exhaust`]
//!   ([`TypeChecker::check_match`], [`TypeChecker::push_arm_refinement`]).
//! * **Patterns** — bind/check `let`/lambda/match patterns, routing linear resources into `Δ`
//!   and unrestricted names into `Γ` ([`TypeChecker::bind_pat`],
//!   [`TypeChecker::bind_pat_with_rhs`], [`TypeChecker::check_pat`],
//!   [`TypeChecker::check_pat_into`]).
//! * **Subsumption & function shape** — the `⇒ then =` rule and the `dom → cod` view with
//!   metavariable invention ([`TypeChecker::subsume`], [`TypeChecker::as_function`]); and
//!   prelude scheme instantiation ([`TypeChecker::instantiate`]).
//!
//! ## Module boundary (ADR-0035)
//!
//! This is a pure code-motion carve out of the [`super::TypeChecker`] monolith: the methods
//! below are *methods on [`super::TypeChecker`]* (they read/write the checker's `table`,
//! `numeric`, `lambda_linears`, and `assumptions`), kept as a single `impl` block in this
//! file. [`super::TypeChecker`] remains the bidirectional facade that dispatches into them;
//! the Circuit, Q-monad, borrow, and refinement slices are deliberately not moved (issues
//! #323, #325, #326). Visibility is the carve's seam: the methods the facade (or a sibling
//! slice) dispatches into are `pub(super)`; intra-classical helpers (`synth_pow`,
//! `merge_branches`, `push_arm_refinement`, `check_pat`) stay private to `classical.rs`.
//! Unification itself lives in [`super::unify`] (`Table`) and exhaustiveness analysis in
//! [`super::exhaust`]; this module *coordinates* them from the classical judgment, it does
//! not re-implement them.

use crate::ast::{BinOp, Expr, LitPat, Pat, Type};
use crate::lexer::{SimpleSpan, Sp};
use crate::refinement::Assumption;
use crate::types::Ty;
use quon_core::DepthExpr;

use super::error::TypeError;
use super::exhaust;
use super::{Delta, Env, Scheme};

// ── Classical Γ judgment: synth/check, unify, exhaust ─────────────────────────
//
// All of the following are methods on `TypeChecker`, moved here from the facade monolith as
// a pure code-motion carve (ADR-0035). They read and write the checker's shared state — the
// metavariable `table`, the deferred `numeric` obligations, the `lambda_linears` capture
// stack, and the refinement `assumptions` — and call back into the facade's generic
// `synth`/`check`/`expect_type`/`ensure_consumed`/`depth_of` helpers (child-module access).
// Only the methods the facade or a sibling slice dispatches into are `pub(super)`;
// intra-classical helpers stay private.

impl super::TypeChecker {
    // ── Arithmetic (Int/Float binops) ──────────────────────────────────────────

    pub(super) fn synth_arith(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        op: BinOp,
        lhs: &Sp<Expr>,
        rhs: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let lt = self.synth(env, delta, lhs)?;
        let rt = self.synth(env, delta, rhs)?;
        if op == BinOp::Pow {
            return self.synth_pow(&lt, &rt, span);
        }
        self.table.unify(&lt, &rt, span)?;
        self.numeric(&lt, span)
    }

    /// Exponentiation promotes to `Float` when the base is `Float` (e.g. `2.0 ^ n`).
    fn synth_pow(&mut self, base: &Ty, exp: &Ty, span: SimpleSpan) -> Result<Ty, TypeError> {
        match (self.table.resolve(base), self.table.resolve(exp)) {
            (Ty::Float, Ty::Int) | (Ty::Float, Ty::Float) => Ok(Ty::Float),
            (Ty::Int, Ty::Int) => Ok(Ty::Int),
            (Ty::Meta(_), _) | (_, Ty::Meta(_)) => {
                let m = self.table.fresh();
                self.table.unify(base, &m, span)?;
                self.numeric(&m, span)
            }
            (found, _) => Err(TypeError::NotNumeric { found, span }),
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

    // ── Lists ──────────────────────────────────────────────────────────────────

    pub(super) fn synth_list(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        es: &[Sp<Expr>],
        _span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        match es.split_first() {
            None => Ok(Ty::list(self.table.fresh())),
            Some((head, tail)) => {
                let elem = self.synth(env, delta, head)?;
                for e in tail {
                    self.check(env, delta, e, &elem)?;
                }
                Ok(Ty::list(elem))
            }
        }
    }

    // ── Lambdas ───────────────────────────────────────────────────────────────

    pub(super) fn synth_lambda(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        params: &[(Sp<Pat>, Option<Sp<Type>>)],
        body: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        // Synthesis only works when every parameter is annotated; otherwise the domain
        // is unconstrained and the lambda needs an expected type (checking mode).
        let mut dom_tys = Vec::with_capacity(params.len());
        let mut inner = env.clone();
        // A lambda owns a *fresh* linear context: it may consume its own linear parameters
        // but not resources captured from the enclosing scope (see `synth_var`).
        let mut lam_delta = Delta::new();
        let mut introduced = Vec::new();
        for (pat, ann) in params {
            let Some(ann) = ann else {
                return Err(TypeError::AmbiguousLambda { span });
            };
            let ty = self.resolve_type(ann)?;
            let mut bound = self.bind_pat(pat, &ty, &mut inner, &mut lam_delta)?;
            introduced.append(&mut bound);
            dom_tys.push(ty);
        }
        let cod = self.in_lambda_scope(delta, &mut lam_delta, &introduced, |me, d| {
            me.synth(&inner, d, body)
        })?;
        // Curry: a nullary lambda is `Unit -> cod`; otherwise `T₁ -> … -> Tₙ -> cod`.
        if dom_tys.is_empty() {
            return Ok(Ty::func(Ty::Unit, cod));
        }
        Ok(dom_tys
            .into_iter()
            .rev()
            .fold(cod, |acc, t| Ty::func(t, acc)))
    }

    /// Run a lambda body with the enclosing scope's linear resources made *visible only as
    /// capture errors*: their names are pushed as a frame onto `lambda_linears` so any use
    /// inside `body_fn` reports [`TypeError::LinearCapture`]. The lambda's own linear
    /// parameters (`introduced`, live in `lam_delta`) must be fully consumed by the body.
    pub(super) fn in_lambda_scope<R>(
        &mut self,
        enclosing: &Delta,
        lam_delta: &mut Delta,
        introduced: &[(String, SimpleSpan)],
        body_fn: impl FnOnce(&mut Self, &mut Delta) -> Result<R, TypeError>,
    ) -> Result<R, TypeError> {
        self.lambda_linears.push(enclosing.residual());
        let outcome = body_fn(self, lam_delta)
            .and_then(|r| self.ensure_consumed(lam_delta, introduced).map(|()| r));
        self.lambda_linears.pop();
        outcome
    }

    // ── Subsumption ───────────────────────────────────────────────────────────

    /// The subsumption rule: `Γ ; Δ ⊢ e ⇒ τ'`, then `τ' = τ`.
    pub(super) fn subsume(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        expr: &Sp<Expr>,
        expected: &Ty,
    ) -> Result<(), TypeError> {
        let got = self.synth(env, delta, expr)?;
        self.expect_type(expected, &got, expr.1)
    }

    // ── Branches (if / then / else) ────────────────────────────────────────────

    /// Type an `if`/`then`/`else`, shared by synthesis (`expected = None`) and checking
    /// (`expected = Some(τ)`). The condition is checked first, threading `Δ` (it may consume
    /// resources both branches then share). Each branch runs against a *clone* of the
    /// post-condition `Δ`; the residuals must agree, so a resource spent on one path but not
    /// the other is rejected ([`TypeError::LinearBranchMismatch`]). The merged residual
    /// becomes the surrounding `Δ`.
    pub(super) fn branch_if(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        cond: &Sp<Expr>,
        then: &Sp<Expr>,
        else_: &Sp<Expr>,
        expected: Option<&Ty>,
    ) -> Result<Ty, TypeError> {
        // A condition is `Bool` or a classical measurement `Bit` (SPEC §3.5: a measured bit
        // drives classical control, as in teleport's `if x_bit then X else identity(1)`).
        let cond_ty = self.synth(env, delta, cond)?;
        match self.table.resolve(&cond_ty) {
            Ty::Bool | Ty::Bit => {}
            Ty::Meta(_) => self.table.unify(&cond_ty, &Ty::Bool, cond.1)?,
            other => {
                return Err(TypeError::Mismatch {
                    expected: Ty::Bool,
                    found: other,
                    span: cond.1,
                });
            }
        }
        let mut d_then = delta.clone();
        let mut d_else = delta.clone();
        let result = match expected {
            Some(t) => {
                self.check(env, &mut d_then, then, t)?;
                self.check(env, &mut d_else, else_, t)?;
                t.clone()
            }
            None => {
                let then_ty = self.synth(env, &mut d_then, then)?;
                let else_ty = self.synth(env, &mut d_else, else_)?;
                self.join_branch_types(&then_ty, &else_ty, then.1)?
            }
        };
        self.merge_branches(delta, &[(&d_then, then.1), (&d_else, else_.1)])?;
        Ok(result)
    }

    /// Join the linear residuals of a set of alternative branches into `out`. Every branch
    /// must leave exactly the same resources live; otherwise some resource was consumed on
    /// one path but dropped on another. The diagnostic points at the branch that *failed to
    /// consume* the witness resource.
    fn merge_branches(
        &self,
        out: &mut Delta,
        branches: &[(&Delta, SimpleSpan)],
    ) -> Result<(), TypeError> {
        let (first, _) = branches[0];
        let first_res = first.residual();
        for (other, span) in &branches[1..] {
            let other_res = other.residual();
            if other_res != first_res {
                let witness = first_res
                    .symmetric_difference(&other_res)
                    .next()
                    .cloned()
                    .unwrap_or_default();
                // Point at whichever branch still holds the resource (i.e. did not consume it).
                let offender = if other.is_available(&witness) {
                    *span
                } else {
                    branches[0].1
                };
                return Err(TypeError::LinearBranchMismatch {
                    name: witness,
                    span: offender,
                });
            }
        }
        *out = first.clone();
        Ok(())
    }

    // ── Lambda checking ───────────────────────────────────────────────────────

    /// Check a lambda against an expected (curried) function type by peeling one arrow per
    /// parameter: for `fn(p₁, …, pₙ) -> body ⇐ τ`, each `pᵢ` consumes one `Dᵢ -> …` layer
    /// and the body is checked against whatever type remains. A nullary lambda expects
    /// `Unit -> τ`.
    pub(super) fn check_lambda(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        params: &[(Sp<Pat>, Option<Sp<Type>>)],
        body: &Sp<Expr>,
        expected: &Ty,
        span: SimpleSpan,
    ) -> Result<(), TypeError> {
        let mut inner = env.clone();
        let mut current = expected.clone();
        // As in `synth_lambda`: a fresh linear context for the body; its own linear
        // parameters must be consumed, ambient resources may not be captured.
        let mut lam_delta = Delta::new();
        let mut introduced = Vec::new();

        if params.is_empty() {
            let (dom, cod) = self.as_function(&current, span)?;
            self.table.unify(&dom, &Ty::Unit, span)?;
            return self.in_lambda_scope(delta, &mut lam_delta, &introduced, |me, d| {
                me.check(&inner, d, body, &cod)
            });
        }

        for (pat, ann) in params {
            let (dom, cod) = self.as_function(&current, span)?;
            if let Some(ann) = ann {
                let annotated = self.resolve_type(ann)?;
                self.table.unify(&annotated, &dom, ann.1)?;
            }
            let mut bound = self.bind_pat(pat, &dom, &mut inner, &mut lam_delta)?;
            introduced.append(&mut bound);
            current = cod;
        }
        self.in_lambda_scope(delta, &mut lam_delta, &introduced, |me, d| {
            me.check(&inner, d, body, &current)
        })
    }

    // ── match ──────────────────────────────────────────────────────────────────

    /// Check a `match`. When `expected` is `Some`, every arm is checked against it;
    /// otherwise arm bodies are synthesized and unified to a common type. Exhaustiveness
    /// and reachability are validated against the scrutinee's type either way.
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
    fn push_arm_refinement(
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

    pub(super) fn check_match(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        scrutinee: &Sp<Expr>,
        arms: &[(Sp<Pat>, Sp<Expr>)],
        expected: Option<&Ty>,
    ) -> Result<Ty, TypeError> {
        // The scrutinee is evaluated once and may consume resources; the arms are
        // alternatives that then each branch from the same post-scrutinee `Δ`.
        let scrut_ty = self.synth(env, delta, scrutinee)?;

        // Each arm gets a clone of `Δ`; its pattern may bind further linear resources, which
        // (like any scope) must be consumed within the arm. The residual ambient resources
        // are collected for the join.
        //
        // In checking mode every arm is checked against the expected type. In synthesis mode
        // arms are synthesized and folded with [`join_branch_types`], so circuit arms differing
        // only in depth reconcile by `max` rather than being forced equal.
        let mut arm_deltas: Vec<(Delta, SimpleSpan)> = Vec::with_capacity(arms.len());
        let mut joined: Option<Ty> = None;
        for (pat, body) in arms {
            let mut inner = env.clone();
            let mut d_arm = delta.clone();
            let introduced = self.check_pat(pat, &scrut_ty, &mut inner, &mut d_arm)?;

            // Dependent refinement (issue #59): augment the proof context with what this arm's
            // pattern reveals about a `Nat` scrutinee — `scrut = k` for a literal arm, `scrut ≠ kᵢ`
            // for a wildcard/variable arm against every sibling literal. Discharged by `expect_type`
            // while checking the arm body (so e.g. `identity(0)`'s width `0 = n` proves under `n = 0`,
            // and `n - 1`'s predecessor reasoning gets `n ≥ 1` from `{0, _}`). Popped on *every* path
            // (the `truncate`), so nothing leaks into sibling arms or the enclosing scope.
            let base = self.assumptions.len();
            self.push_arm_refinement(&scrut_ty, scrutinee, pat, arms);
            let arm_outcome = match expected {
                Some(t) => self.check(&inner, &mut d_arm, body, t).map(|()| None),
                None => self.synth(&inner, &mut d_arm, body).map(Some),
            };
            self.assumptions.truncate(base);

            if let Some(arm_ty) = arm_outcome? {
                joined = Some(match joined {
                    None => arm_ty,
                    Some(prev) => self.join_branch_types(&prev, &arm_ty, body.1)?,
                });
            }
            self.ensure_consumed(&d_arm, &introduced)?;
            arm_deltas.push((d_arm, body.1));
        }
        let result = match expected {
            Some(t) => t.clone(),
            None => joined.unwrap_or_else(|| self.table.fresh()),
        };

        // Exhaustiveness + reachability against the (zonked) scrutinee type.
        let zonked = self.table.zonk(&scrut_ty);
        let pats: Vec<&Pat> = arms.iter().map(|(p, _)| &p.0).collect();
        let analysis = exhaust::analyze(&pats, &zonked);
        if let Some(&idx) = analysis.unreachable.first() {
            return Err(TypeError::UnreachableArm {
                span: arms[idx].0.1,
            });
        }
        if let Some(witness) = analysis.missing {
            return Err(TypeError::NonExhaustive {
                witness: witness.to_string(),
                span: scrutinee.1,
            });
        }

        // Join: all arms must leave the same ambient linear resources live.
        if !arm_deltas.is_empty() {
            let refs: Vec<(&Delta, SimpleSpan)> = arm_deltas.iter().map(|(d, s)| (d, *s)).collect();
            self.merge_branches(delta, &refs)?;
        }

        Ok(self.table.zonk(&result))
    }

    // ── Patterns ─────────────────────────────────────────────────────────────

    /// Bind the variables of an irrefutable `let`/lambda pattern, routing linear-resource
    /// bindings into `Δ` and unrestricted ones into `Γ`. Returns the linear names introduced
    /// (with their binding spans) so the enclosing scope can enforce they are consumed.
    pub(super) fn bind_pat(
        &mut self,
        pat: &Sp<Pat>,
        ty: &Ty,
        env: &mut Env,
        delta: &mut Delta,
    ) -> Result<Vec<(String, SimpleSpan)>, TypeError> {
        self.bind_pat_with_rhs(pat, None, ty, env, delta)
    }

    pub(super) fn bind_pat_with_rhs(
        &mut self,
        pat: &Sp<Pat>,
        rhs: Option<&Sp<Expr>>,
        ty: &Ty,
        env: &mut Env,
        delta: &mut Delta,
    ) -> Result<Vec<(String, SimpleSpan)>, TypeError> {
        let mut introduced = Vec::new();
        self.check_pat_into(pat, rhs, ty, env, delta, &mut introduced)?;
        Ok(introduced)
    }

    /// Type-check a pattern against `ty`, binding its variables. Convenience wrapper around
    /// [`Self::check_pat_into`] that allocates the introduced-names list.
    fn check_pat(
        &mut self,
        pat: &Sp<Pat>,
        ty: &Ty,
        env: &mut Env,
        delta: &mut Delta,
    ) -> Result<Vec<(String, SimpleSpan)>, TypeError> {
        self.bind_pat(pat, ty, env, delta)
    }

    /// Type-check a pattern against `ty`, binding unrestricted variables into `env`, linear
    /// ones into `delta`, and pushing each linear binding onto `introduced`.
    pub(super) fn check_pat_into(
        &mut self,
        pat: &Sp<Pat>,
        rhs: Option<&Sp<Expr>>,
        ty: &Ty,
        env: &mut Env,
        delta: &mut Delta,
        introduced: &mut Vec<(String, SimpleSpan)>,
    ) -> Result<(), TypeError> {
        let span = pat.1;
        match &pat.0 {
            // A wildcard over a linear resource is a silent discard — rejected (no-dropping),
            // except a `QReg` remainder after `split`, which SPEC §3.4 allows to drop.
            Pat::Wildcard => {
                let resolved = self.table.resolve(ty);
                if resolved.is_linear_resource() && !matches!(resolved, Ty::QReg(_)) {
                    let (bound_name, binding_span, let_span) = match rhs {
                        Some((Expr::Var(name), rhs_span)) => {
                            (Some(name.clone()), Some(*rhs_span), None)
                        }
                        _ => (None, None, None),
                    };
                    return Err(TypeError::LinearDiscard {
                        name: resolved.to_string(),
                        bound_name,
                        binding_span,
                        let_span,
                        span,
                    });
                }
                Ok(())
            }
            Pat::Var(name) => {
                let resolved = self.table.resolve(ty);
                // A `_`-prefixed name discards a register without using it — this is the named
                // form of the wildcard discard above, for `split` remainders written `_rest`
                // (SPEC §3.4). As with the wildcard, only a `QReg` may be dropped this way; a
                // bare `Qubit` (or any other linear resource) must still be consumed, so a typo
                // like `let _q = some_qubit` is a no-dropping error, not a silent leak.
                if name.starts_with('_') && matches!(resolved, Ty::QReg(_)) {
                    return Ok(());
                }
                // Binding-site annotation powers inlay hints / hover on `let` names (#176).
                self.record_annotation(span, &resolved);
                if resolved.is_linear_resource() {
                    delta.introduce(name.clone(), resolved, span)?;
                    introduced.push((name.clone(), span));
                } else {
                    env.insert(name.clone(), ty.clone());
                }
                Ok(())
            }
            Pat::Lit(lit) => {
                let lit_ty = match lit {
                    crate::ast::LitPat::Int(n)
                        if matches!(self.table.resolve(ty), Ty::Bit) && (*n == 0 || *n == 1) =>
                    {
                        Ty::Bit
                    }
                    crate::ast::LitPat::Int(_) => Ty::Int,
                    crate::ast::LitPat::Bool(_) => Ty::Bool,
                };
                self.table.unify(ty, &lit_ty, span)
            }
            Pat::Tuple(ps) => match self.table.resolve(ty) {
                Ty::Tuple(ts) if ts.len() == ps.len() => {
                    for (p, t) in ps.iter().zip(&ts) {
                        self.check_pat_into(p, None, t, env, delta, introduced)?;
                    }
                    Ok(())
                }
                Ty::Tuple(ts) => Err(TypeError::ArityMismatch {
                    expected: ts.len(),
                    found: ps.len(),
                    span,
                }),
                // A `QReg<n>` destructured by an n-tuple pattern: each slot is a `Qubit`
                // (the register size must be statically known).
                Ty::QReg(n) if n.as_const().is_some() => {
                    let size = n.as_const().unwrap_or(0) as usize;
                    if size != ps.len() {
                        return Err(TypeError::ArityMismatch {
                            expected: size,
                            found: ps.len(),
                            span,
                        });
                    }
                    for p in ps {
                        self.check_pat_into(p, None, &Ty::Qubit, env, delta, introduced)?;
                    }
                    Ok(())
                }
                Ty::Meta(_) => {
                    let fresh: Vec<Ty> = (0..ps.len()).map(|_| self.table.fresh()).collect();
                    self.table.unify(ty, &Ty::Tuple(fresh.clone()), span)?;
                    for (p, t) in ps.iter().zip(&fresh) {
                        self.check_pat_into(p, None, t, env, delta, introduced)?;
                    }
                    Ok(())
                }
                other => Err(TypeError::Mismatch {
                    expected: other,
                    found: Ty::Tuple(vec![self.table.fresh(); ps.len()]),
                    span,
                }),
            },
        }
    }

    // ── Function-shape / instantiation helpers ─────────────────────────────────

    /// View `ty` as a function `dom -> cod`, inventing metavariables if `ty` is a
    /// metavariable. Errors if `ty` is a concrete non-function.
    pub(super) fn as_function(&mut self, ty: &Ty, span: SimpleSpan) -> Result<(Ty, Ty), TypeError> {
        match self.table.resolve(ty) {
            Ty::Fn(a, b) | Ty::Linear(a, b) => Ok((*a, *b)),
            m @ Ty::Meta(_) => {
                let dom = self.table.fresh();
                let cod = self.table.fresh();
                self.table
                    .unify(&m, &Ty::func(dom.clone(), cod.clone()), span)?;
                Ok((dom, cod))
            }
            other => Err(TypeError::NotAFunction { found: other, span }),
        }
    }

    pub(super) fn instantiate(&mut self, scheme: &Scheme) -> Ty {
        scheme.instantiate(&mut self.table)
    }
}

#[cfg(test)]
mod tests {
    //! Locks the extracted classical-Γ typing interface. The pure-function-testable seams —
    //! [`Table::unify`] coordination via [`as_function`], the deferred numeric metavariable
    //! obligation ([`numeric`]/[`finalize_numeric`]), and prelude scheme instantiation
    //! ([`instantiate`]) — are exercised directly on a fresh [`TypeChecker`]. Exhaustiveness
    //! and pattern-matrix reachability are covered by the [`exhaust`] module's own tests; the
    //! classical judgment *coordinates* them, so these tests cover the classical-specific glue.

    use super::super::TypeChecker;
    use super::super::builtins;
    use super::*;
    use crate::ast::{LitPat, Name, Pat};

    /// Build a `Sp<T>` with a dummy zero span.
    fn span() -> SimpleSpan {
        (0..0).into()
    }

    fn sp<T>(t: T) -> Sp<T> {
        (t, span())
    }

    // ── as_function: unification-coordinated function-shape view ──────────────

    #[test]
    fn as_function_peels_concrete_arrow() {
        let mut tc = TypeChecker::new();
        let ty = Ty::func(Ty::Int, Ty::Bool);
        let (dom, cod) = tc.as_function(&ty, span()).expect("concrete function");
        assert_eq!(dom, Ty::Int);
        assert_eq!(cod, Ty::Bool);
    }

    #[test]
    fn as_function_invents_dom_cod_for_metavar() {
        let mut tc = TypeChecker::new();
        let m = tc.table.fresh();
        let (dom, cod) = tc
            .as_function(&m, span())
            .expect("metavariable unified to function");
        // After unification the metavar resolves to a function whose dom/cod are the fresh vars.
        let resolved = tc.table.resolve(&m);
        assert!(matches!(resolved, Ty::Fn(_, _)));
        // The returned dom/cod are the exact fresh variables embedded in the unified type.
        assert!(matches!(dom, Ty::Meta(_)));
        assert!(matches!(cod, Ty::Meta(_)));
    }

    #[test]
    fn as_function_rejects_non_function() {
        let mut tc = TypeChecker::new();
        let err = tc.as_function(&Ty::Int, span());
        assert!(matches!(err, Err(TypeError::NotAFunction { .. })));
    }

    // ── numeric / finalize_numeric: deferred obligation defaulting ─────────────

    #[test]
    fn numeric_resolves_int_and_float() {
        let mut tc = TypeChecker::new();
        assert_eq!(tc.numeric(&Ty::Int, span()).unwrap(), Ty::Int);
        assert_eq!(tc.numeric(&Ty::Float, span()).unwrap(), Ty::Float);
        // No obligations recorded for solved types.
        assert!(tc.numeric.is_empty());
    }

    #[test]
    fn numeric_defers_metavar_then_finalize_defaults_to_int() {
        let mut tc = TypeChecker::new();
        let m = tc.table.fresh();
        let returned = tc.numeric(&m, span()).unwrap();
        // The metavariable is returned unchanged, and one obligation is recorded.
        assert!(matches!(returned, Ty::Meta(_)));
        assert_eq!(tc.numeric.len(), 1);
        // Finalize defaults the unsolved metavariable to Int.
        tc.finalize_numeric().unwrap();
        assert!(tc.numeric.is_empty());
        assert_eq!(tc.table.resolve(&m), Ty::Int);
    }

    #[test]
    fn finalize_numeric_keeps_a_metavar_solved_to_float() {
        let mut tc = TypeChecker::new();
        let m = tc.table.fresh();
        tc.numeric(&m, span()).unwrap();
        // A later unification solves the metavariable to Float.
        tc.table.unify(&m, &Ty::Float, span()).unwrap();
        tc.finalize_numeric().unwrap();
        assert_eq!(tc.table.resolve(&m), Ty::Float);
    }

    #[test]
    fn numeric_rejects_non_numeric() {
        let mut tc = TypeChecker::new();
        let err = tc.numeric(&Ty::Bool, span());
        assert!(matches!(err, Err(TypeError::NotNumeric { .. })));
    }

    // ── instantiate: prelude scheme instantiation ─────────────────────────────

    #[test]
    fn instantiate_replaces_rigid_vars_with_fresh_metavars() {
        let mut tc = TypeChecker::new();
        let map_scheme = builtins::lookup("map").expect("map is a builtin");
        let ty = tc.instantiate(&map_scheme);
        // `map : (A -> B, List<A>) -> List<B>` instantiates to a curried function.
        let (dom, cod) = tc
            .as_function(&ty, span())
            .expect("map instantiates to a function");
        // Domain is itself a function `A -> B`.
        assert!(matches!(tc.table.resolve(&dom), Ty::Fn(_, _)));
        // Codomain is `List<A> -> List<B>` — peel the second arrow.
        let (dom2, cod2) = tc
            .as_function(&cod, span())
            .expect("map codomain is curried");
        assert!(matches!(tc.table.resolve(&dom2), Ty::List(_))); // List<A>
        assert!(matches!(tc.table.resolve(&cod2), Ty::List(_))); // List<B>
    }

    #[test]
    fn instantiate_monomorphic_scheme_is_unchanged() {
        let mut tc = TypeChecker::new();
        let range_scheme = builtins::lookup("range").expect("range is a builtin");
        let ty = tc.instantiate(&range_scheme);
        // `range : Int -> List<Int>` — no quantified vars, so the body is returned as-is.
        let (dom, cod) = tc.as_function(&ty, span()).expect("range is a function");
        assert_eq!(dom, Ty::Int);
        assert_eq!(cod, Ty::list(Ty::Int));
    }

    // ── check_pat_into: classical pattern binding (Γ / Δ routing) ──────────────

    #[test]
    fn check_pat_binds_unrestricted_var_into_gamma() {
        let mut tc = TypeChecker::new();
        let mut env = Env::new();
        let mut delta = Delta::new();
        let mut introduced = Vec::new();
        let pat = sp(Pat::Var(Name::from("x")));
        tc.check_pat_into(&pat, None, &Ty::Int, &mut env, &mut delta, &mut introduced)
            .unwrap();
        // An unrestricted `Int` binding goes into Γ, not Δ.
        assert_eq!(env.get("x"), Some(&Ty::Int));
        assert!(introduced.is_empty());
        assert!(delta.residual().is_empty());
    }

    #[test]
    fn check_pat_wildcard_over_int_is_ok() {
        let mut tc = TypeChecker::new();
        let mut env = Env::new();
        let mut delta = Delta::new();
        let mut introduced = Vec::new();
        let pat = sp(Pat::Wildcard);
        tc.check_pat_into(&pat, None, &Ty::Int, &mut env, &mut delta, &mut introduced)
            .unwrap();
        // A wildcard over a non-linear type is fine; nothing is introduced.
        assert!(introduced.is_empty());
    }

    #[test]
    fn check_pat_int_literal_unifies_with_int() {
        let mut tc = TypeChecker::new();
        let mut env = Env::new();
        let mut delta = Delta::new();
        let mut introduced = Vec::new();
        let pat = sp(Pat::Lit(LitPat::Int(42)));
        tc.check_pat_into(&pat, None, &Ty::Int, &mut env, &mut delta, &mut introduced)
            .unwrap();
        // A literal pattern introduces no bindings.
        assert!(introduced.is_empty());
    }

    #[test]
    fn check_pat_int_literal_0_or_1_unifies_with_bit() {
        let mut tc = TypeChecker::new();
        let mut env = Env::new();
        let mut delta = Delta::new();
        let mut introduced = Vec::new();
        let pat = sp(Pat::Lit(LitPat::Int(1)));
        tc.check_pat_into(&pat, None, &Ty::Bit, &mut env, &mut delta, &mut introduced)
            .unwrap();
        // `1` is a valid `Bit` literal; the scrutinee unifies to Bit.
    }

    #[test]
    fn check_pat_tuple_destructures_into_components() {
        let mut tc = TypeChecker::new();
        let mut env = Env::new();
        let mut delta = Delta::new();
        let mut introduced = Vec::new();
        let inner_pats = vec![sp(Pat::Var(Name::from("a"))), sp(Pat::Var(Name::from("b")))];
        let pat = sp(Pat::Tuple(inner_pats));
        let ty = Ty::Tuple(vec![Ty::Int, Ty::Bool]);
        tc.check_pat_into(&pat, None, &ty, &mut env, &mut delta, &mut introduced)
            .unwrap();
        // Each component is bound into Γ (unrestricted).
        assert_eq!(env.get("a"), Some(&Ty::Int));
        assert_eq!(env.get("b"), Some(&Ty::Bool));
        assert!(introduced.is_empty());
    }

    #[test]
    fn check_pat_tuple_arity_mismatch_errors() {
        let mut tc = TypeChecker::new();
        let mut env = Env::new();
        let mut delta = Delta::new();
        let mut introduced = Vec::new();
        let inner_pats = vec![sp(Pat::Var(Name::from("a"))), sp(Pat::Var(Name::from("b")))];
        let pat = sp(Pat::Tuple(inner_pats));
        let ty = Ty::Tuple(vec![Ty::Int]); // only one component
        let err = tc.check_pat_into(&pat, None, &ty, &mut env, &mut delta, &mut introduced);
        assert!(matches!(err, Err(TypeError::ArityMismatch { .. })));
    }
}
