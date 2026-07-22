//! The **Quantum Monad / borrow judgment** module — the `Q<τ>` monadic type, `<-` binds,
//! `run { }` blocks, and `borrow` ancilla policy (issues #14, #15, #325; SPEC §3.4, §3.5).
//!
//! ## Judgment form
//!
//! Quon's **Quantum Monad** is the type `Q<τ>` — a quantum computation that may perform
//! mid-circuit measurement and return a value of type `τ` (CONTEXT.md "Quantum Monad").
//! Computations in `Q` are written with `run { }` blocks, which the desugar pass folds into a
//! `Bind`/`Let`/`Return` chain *before* checking (issue #8), so the checker never sees a raw
//! `RunBlock` — only the monadic bind algebra it reduces to. This module owns the *Quantum
//! Monad* and *borrow* judgments:
//!
//! ```text
//!   Γ ; Δ ⊢ return v ⇒ Q<τ>                    lift a value into the monad
//!   Γ ; Δ ⊢ x <- e₁; e₂ ⇒ Q<β>                 monadic bind (e₁ : Q<α>, auto-lifts a pure resource)
//!   Γ ; Δ ⊢ borrow bᵢ: Tᵢ in { body } ⇒ Q<τ>   scoped ancilla allocation
//! ```
//!
//! `Γ` ([`super::Env`]) holds classical names; `Δ` (the linear context, [`super::Delta`])
//! tracks the qubit resources a monadic computation consumes exactly once. `Δ` itself is the
//! bookkeeping module ([`super::linear`]); this module only *coordinates* escape and cleanup
//! policy against it. The classical fragment, the **Circuit** judgment, and the Z3-backed
//! **refinement** bridge are *other slices* (#323, #326) and stay in [`super::TypeChecker`];
//! only the Quantum Monad / borrow judgment lives here.
//!
//! ## What this module owns
//!
//! * **`Q<τ>` synthesis** — `return v` lifts `v : τ` to `Q<τ>`
//!   ([`TypeChecker::synth_return`]); the monad is entered by `return`/`measure`/`qreg`/
//!   `reset`/`discard` (ADR-0006), never by a pure value.
//! * **Monadic bind** `x <- e₁; e₂` — `e₁` must be `Q<α>`, *or* a pure quantum resource
//!   (`Qubit`/`QReg`/`Circuit`) which is **auto-lifted** (ADR-0006); a pure classical value is
//!   rejected as [`TypeError::ExpectedMonad`]. The continuation must itself be `Q<β>`
//!   ([`TypeChecker::synth_bind`]).
//! * **Monadic combinators** — `measure_all : QReg<n> -> Q<List<Bit>>`, plus `map_q` and
//!   `sequence_q` (SPEC §5) ([`TypeChecker::synth_measure_all`], [`synth_map_q`],
//!   [`synth_sequence_q`]). The bare `measure`/`reset`/`discard`/`qubit` primitives are prelude
//!   builtins ([`super::builtins`]), not moved here.
//! * **Borrow block** `borrow bᵢ: Tᵢ in { body }` — scoped ancilla allocation. Each ancilla
//!   enters `Δ` as a linear resource; the body is checked as a monadic block producing `Q<τ>`
//!   ([`TypeChecker::synth_borrow`]).
//!
//! ## Borrow policy: consume + no-escape (issue #180)
//!
//! A `borrow` block (CONTEXT.md "Borrow block") is well-typed iff each ancilla is
//! **consumed exactly once** inside the block and **does not escape** in the result. Two
//! guarantees are enforced:
//!
//! * **Cleanup (no-dropping).** Every ancilla must be consumed — measured, `reset`,
//!   `discard`ed, or fed into any consuming operation. A still-live ancilla at block exit is a
//!   [`TypeError::LinearUnconsumed`] (via [`TypeChecker::ensure_consumed`], coordinating with
//!   `Δ`). Per **issue #180**, valid cleanup includes `measure`, `reset`, *and* `discard` —
//!   **not** only a structural `reset`/`discard` terminal. **ADR-0003**'s structural-only
//!   wording (the final use of `q` must literally be `reset(q)`/`discard(q)`) is **superseded
//!   by #180** for borrow-block cleanup; the shipped rule is the weaker, consume-based one
//!   (option A of #180), which lets the 3-qubit bit-flip `syndrome_measure` type-check with
//!   mid-circuit `measure` cleanup.
//! * **No escape.** A borrowed name may not appear in the block's *result* value, even buried
//!   in a returned register — otherwise the ancilla outlives its scope
//!   ([`TypeError::BorrowEscape`]). Mentions in *consuming* positions (`measure(a)`,
//!   `discard(a)`) are exactly how an ancilla is reclaimed, so only `Return` payloads are
//!   inspected ([`find_borrow_escape`]).
//!
//! ## Module boundary (ADR-0031)
//!
//! Pure code-motion carve out of the [`super::TypeChecker`] monolith (the #325 slice, twin of
//! the Circuit carve #323 / ADR-0028): the methods below are *methods on
//! [`super::TypeChecker`]* (they read/write the checker's `table` and call back into the
//! facade's `synth`/`resolve_type`/`bind_pat`/`ensure_consumed` helpers), kept as a single
//! `impl` block in this file. [`super::TypeChecker`] remains the bidirectional facade that
//! dispatches into them; only the methods the facade dispatches into are `pub(super)`, the
//! escape-detection helpers stay private. No behavior change — the bodies are moved verbatim.

use crate::ast::{Expr, Name, Pat, Stmt, Type};
use crate::lexer::{SimpleSpan, Sp};
use crate::types::Ty;
use quon_core::DepthExpr;
use std::collections::BTreeSet;

use super::error::TypeError;
use super::{Delta, Env};

// ── Quantum Monad / borrow judgment: Q<τ> synthesis, bind, borrow policy ──────
//
// All of the following are methods on `TypeChecker`, moved here from the facade monolith as
// a pure code-motion carve (ADR-0031). They read and write the checker's shared state — the
// metavariable `table` — and call back into the facade's generic `synth`/`resolve_type`/
// `bind_pat`/`ensure_consumed` helpers (child-module access). Only the methods the facade
// dispatches into are `pub(super)`; the borrow escape-detection helpers stay private.

impl super::TypeChecker {
    /// `return v` (SPEC §3.5): synthesize `v`'s type `τ` and lift it into the quantum monad,
    /// yielding `Q<τ>`. This is the canonical `Q<τ>` synthesis rule — the monad is entered by
    /// `return`/`measure`/`qreg`/`reset`/`discard` (ADR-0006), never by a pure value.
    pub(super) fn synth_return(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        v: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let t = self.synth(env, delta, v)?;
        Ok(Ty::Q(Box::new(t)))
    }

    /// Sequential Z-basis measurement of every qubit in a register (SPEC §5).
    pub(super) fn synth_measure_all(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        q: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let q_ty = self.synth(env, delta, q)?;
        match self.table.resolve(&q_ty) {
            Ty::QReg(_) => Ok(Ty::Q(Box::new(Ty::list(Ty::Bit)))),
            other => Err(TypeError::Mismatch {
                expected: Ty::QReg(DepthExpr::Var("n".into())),
                found: other,
                span: q.1,
            }),
        }
    }

    /// Monadic map (SPEC §5): `(A -> Q<B>, List<A>) -> Q<List<B>>`.
    pub(super) fn synth_map_q(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        f: &Sp<Expr>,
        xs: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let xs_ty = self.synth(env, delta, xs)?;
        let elem_a = self.table.fresh();
        self.table.unify(&xs_ty, &Ty::list(elem_a.clone()), xs.1)?;
        let f_ty = self.synth(env, delta, f)?;
        let (dom, cod) = self.as_function(&f_ty, f.1)?;
        self.table.unify(&dom, &elem_a, f.1)?;
        match self.table.resolve(&cod) {
            Ty::Q(inner) => Ok(Ty::Q(Box::new(Ty::list(*inner)))),
            other => Err(TypeError::Mismatch {
                expected: Ty::Q(Box::new(self.table.fresh())),
                found: other,
                span: f.1,
            }),
        }
    }

    /// Sequence a list of monadic computations (SPEC §5): `List<Q<A>> -> Q<List<A>>`.
    pub(super) fn synth_sequence_q(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        cs: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let cs_ty = self.synth(env, delta, cs)?;
        let elem = self.table.fresh();
        self.table
            .unify(&cs_ty, &Ty::list(Ty::Q(Box::new(elem.clone()))), cs.1)?;
        Ok(Ty::Q(Box::new(Ty::list(elem))))
    }

    /// A monadic bind `x <- e₁; e₂` (`Bind(e₁, fn(x) -> e₂)`, SPEC §3.5). `e₁` must produce a
    /// quantum computation `Q<A>` — or a pure value `A`, which is auto-lifted (so a unitary
    /// result threaded through `<-` is accepted). `x : A` is then bound (routed by linearity)
    /// and the continuation `e₂` is checked; it must itself be `Q<B>`, the type of the bind.
    /// `Δ` threads `e₁` then `e₂`, so a resource consumed by `e₁` is gone for `e₂` — reusing a
    /// just-measured qubit surfaces as [`TypeError::LinearUsedTwice`].
    pub(super) fn synth_bind(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        rhs: &Sp<Expr>,
        param: &Sp<Name>,
        body: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let rhs_ty = self.synth(env, delta, rhs)?;
        // The value the bind threads. The payload of a `Q<A>` is the usual case. A *pure*
        // quantum resource is auto-lifted: a unitary application `c @ r : QReg<m>` has no
        // measurement, so it stays outside `Q`, yet is naturally named with `<-` (teleport's
        // `(a, b) <- bell_state() @ (alice, bob)`). A pure *classical* value (or an unsolved
        // metavariable) is not a quantum computation — binding it with `<-` is the error AC5
        // names; use `let` instead.
        let bound = match self.table.resolve(&rhs_ty) {
            Ty::Q(inner) => *inner,
            pure if pure.is_linear_resource() => pure,
            _ => {
                return Err(TypeError::ExpectedMonad {
                    found: self.table.zonk(&rhs_ty),
                    span: rhs.1,
                });
            }
        };
        let mut inner = env.clone();
        // Bind the threaded value. `_` discards it (a `Pat::Wildcard`, which rejects a leftover
        // linear resource as a no-dropping error); a name routes by linearity via `bind_pat`.
        let pat = if param.0 == "_" {
            (Pat::Wildcard, param.1)
        } else {
            (Pat::Var(param.0.clone()), param.1)
        };
        let introduced = self.bind_pat(&pat, &bound, &mut inner, delta)?;
        let body_ty = self.synth(&inner, delta, body)?;
        self.ensure_consumed(delta, &introduced)?;
        // The continuation must be a quantum computation.
        match self.table.resolve(&body_ty) {
            Ty::Q(_) => Ok(body_ty),
            other => Err(TypeError::ExpectedMonad {
                found: other,
                span: body.1,
            }),
        }
    }

    /// A `borrow b₁: T₁, … in { body }` block (SPEC §3.4, ADR-0003): scoped ancilla
    /// allocation. Each ancilla enters `Δ` as a linear resource and the body is checked as a
    /// monadic block producing `Q<τ>`. Two guarantees are enforced:
    ///
    /// * **Cleanup (no-dropping).** Every ancilla must be consumed inside the block —
    ///   measured, `reset`, `discard`ed, or fed into an operation that consumes it. A still-live
    ///   ancilla at block exit is a [`TypeError::LinearUnconsumed`] (via [`Self::ensure_consumed`]).
    /// * **No escape.** A borrowed name may not appear in the block's *result* value, even
    ///   buried inside a returned register — otherwise the ancilla outlives its scope
    ///   ([`TypeError::BorrowEscape`]). The data qubits a block legitimately returns are fine;
    ///   only the *borrowed* names are forbidden in a `return`.
    ///
    /// The body is a statement sequence; it is folded into the same `Bind`/`Let`/`Return` chain
    /// a `run { }` block desugars to (issue #8) and then synthesized.
    pub(super) fn synth_borrow(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        bindings: &[(Sp<Name>, Sp<Type>)],
        body: &[Sp<Stmt>],
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let block =
            crate::desugar::fold_monadic_block(body, span).map_err(|_| TypeError::Unsupported {
                construct: "malformed borrow block",
                span,
            })?;

        // Introduce each ancilla into `Δ`. An ancilla must be a linear quantum resource; a
        // classical `borrow x: Int` is meaningless (there is nothing to reclaim).
        let mut introduced = Vec::new();
        let mut borrowed: BTreeSet<String> = BTreeSet::new();
        for (name, ann) in bindings {
            let ty = self.resolve_type(ann)?;
            if !ty.is_linear_resource() {
                return Err(TypeError::Mismatch {
                    expected: Ty::Qubit,
                    found: ty,
                    span: ann.1,
                });
            }
            self.record_annotation(name.1, &ty);
            delta.introduce(name.0.clone(), ty, name.1)?;
            introduced.push((name.0.clone(), name.1));
            borrowed.insert(name.0.clone());
        }

        // No-escape: reject before threading `Δ`, since a `return a` would otherwise *consume*
        // the ancilla into the result and pass the cleanup check, hiding the escape.
        if let Some((name, esc_span)) = find_borrow_escape(&block, &borrowed) {
            let borrow_span = bindings
                .iter()
                .find(|(n, _)| n.0 == name)
                .map(|(_, ann)| ann.1)
                .unwrap_or(span);
            return Err(TypeError::BorrowEscape {
                name,
                span: esc_span,
                borrow_span,
            });
        }

        let result_ty = self.synth(env, delta, &block)?;
        self.ensure_consumed(delta, &introduced)?;
        match self.table.resolve(&result_ty) {
            Ty::Q(_) => Ok(result_ty),
            other => Err(TypeError::ExpectedMonad { found: other, span }),
        }
    }
}

// ── Borrow escape detection ───────────────────────────────────────────────────

/// Walk a folded monadic block looking for a borrowed ancilla escaping in a `return`. The
/// only escape route is a value flowing out of the block, i.e. the argument of a `return`:
/// a borrowed name appearing there (`return a`, or buried in `return (q `tensored` a)`) means
/// the ancilla outlives its scope. Mentions in *consuming* positions (`measure(a)`,
/// `discard(a)`, `a `tensored` b` whose result is then measured) are fine — those are exactly
/// how an ancilla is reclaimed — so only `Return` payloads are inspected. Returns the first
/// offending name with the span of the returned value.
fn find_borrow_escape(
    expr: &Sp<Expr>,
    borrowed: &BTreeSet<String>,
) -> Option<(String, SimpleSpan)> {
    match &expr.0 {
        Expr::Return(v) => {
            if let Some(name) = first_borrowed_var(v, borrowed) {
                return Some((name, v.1));
            }
            // A `return` may itself wrap further structure (e.g. a nested block); keep walking.
            find_borrow_escape(v, borrowed)
        }
        Expr::Bind { rhs, body, .. } => {
            find_borrow_escape(rhs, borrowed).or_else(|| find_borrow_escape(body, borrowed))
        }
        Expr::Let { rhs, body, .. } => {
            find_borrow_escape(rhs, borrowed).or_else(|| find_borrow_escape(body, borrowed))
        }
        Expr::If { cond, then, else_ } => find_borrow_escape(cond, borrowed)
            .or_else(|| find_borrow_escape(then, borrowed))
            .or_else(|| find_borrow_escape(else_, borrowed)),
        Expr::Match { scrutinee, arms } => find_borrow_escape(scrutinee, borrowed).or_else(|| {
            arms.iter()
                .find_map(|(_, e)| find_borrow_escape(e, borrowed))
        }),
        _ => None,
    }
}

/// The first borrowed name appearing anywhere in a returned value expression, if any.
fn first_borrowed_var(expr: &Sp<Expr>, borrowed: &BTreeSet<String>) -> Option<String> {
    let mut found = None;
    collect_var_hit(expr, borrowed, &mut found);
    found
}

/// Recursively scan `expr` for a `Var` whose name is in `borrowed`, recording the first hit.
fn collect_var_hit(expr: &Sp<Expr>, borrowed: &BTreeSet<String>, found: &mut Option<String>) {
    if found.is_some() {
        return;
    }
    match &expr.0 {
        Expr::Var(name) => {
            if borrowed.contains(name) {
                *found = Some(name.clone());
            }
        }
        Expr::App(a, b)
        | Expr::Compose(a, b)
        | Expr::Par(a, b)
        | Expr::GateApp { gate: a, qubits: b } => {
            collect_var_hit(a, borrowed, found);
            collect_var_hit(b, borrowed, found);
        }
        Expr::TypeApp { callee, .. } => collect_var_hit(callee, borrowed, found),
        Expr::BinOp { lhs, rhs, .. } => {
            collect_var_hit(lhs, borrowed, found);
            collect_var_hit(rhs, borrowed, found);
        }
        Expr::Neg(a) | Expr::Adjoint(a) | Expr::Controlled(a) | Expr::Return(a) => {
            collect_var_hit(a, borrowed, found);
        }
        Expr::Ascribe(a, _) => collect_var_hit(a, borrowed, found),
        Expr::Tuple(es) | Expr::List(es) => {
            for e in es {
                collect_var_hit(e, borrowed, found);
            }
        }
        Expr::If { cond, then, else_ } => {
            collect_var_hit(cond, borrowed, found);
            collect_var_hit(then, borrowed, found);
            collect_var_hit(else_, borrowed, found);
        }
        Expr::Let { rhs, body, .. } => {
            collect_var_hit(rhs, borrowed, found);
            collect_var_hit(body, borrowed, found);
        }
        Expr::Bind { rhs, body, .. } => {
            collect_var_hit(rhs, borrowed, found);
            collect_var_hit(body, borrowed, found);
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    //! Locks the extracted Quantum-Monad / borrow interface. The escape-detection helpers
    //! ([`super::find_borrow_escape`] / [`super::first_borrowed_var`]) are exercised directly as
    //! pure functions over hand-built desugared blocks — escape detection (a borrowed name in a
    //! `return`, bare or buried in a returned register), the no-escape invariant (only
    //! *borrowed* names are flagged; non-borrowed returns pass), and the cleanup-classification
    //! policy (a borrowed name in a *consuming* position — `measure`/`reset`/`discard` — does
    //! *not* escape, since those are exactly how an ancilla is reclaimed, issue #180). One
    //! end-to-end check locks the #180 resolution (measure-only cleanup accepted) at the
    //! module level.

    use super::*;
    use crate::ast::Expr;
    use crate::lexer::{SimpleSpan, Sp};
    use crate::parse_program;
    use std::collections::BTreeSet;

    /// Wrap a node in a zero-span `Sp` (the span is irrelevant to escape detection).
    fn sp<T>(t: T) -> Sp<T> {
        (t, SimpleSpan::from(0..0))
    }

    fn var(name: &str) -> Sp<Expr> {
        sp(Expr::Var(name.to_string()))
    }

    /// `return <v>` — the only escape route the borrow check inspects.
    fn ret(v: Sp<Expr>) -> Sp<Expr> {
        sp(Expr::Return(Box::new(v)))
    }

    /// `x <- rhs; body` — a monadic bind on the desugared tree.
    fn bind(x: &str, rhs: Sp<Expr>, body: Sp<Expr>) -> Sp<Expr> {
        sp(Expr::Bind {
            rhs: Box::new(rhs),
            param: sp(x.to_string()),
            body: Box::new(body),
        })
    }

    /// `f(arg)` — covers `measure(a)` / `discard(a)` / `reset(a)` consuming positions.
    fn app(f: &str, arg: Sp<Expr>) -> Sp<Expr> {
        sp(Expr::App(Box::new(var(f)), Box::new(arg)))
    }

    fn borrowed(names: &[&str]) -> BTreeSet<String> {
        names.iter().map(|n| n.to_string()).collect()
    }

    // ── Escape detection ───────────────────────────────────────────────────────────

    #[test]
    fn return_of_borrowed_ancilla_is_an_escape() {
        // `return a` with `a` borrowed: the ancilla outlives its scope.
        let block = ret(var("a"));
        let got = find_borrow_escape(&block, &borrowed(&["a"]));
        assert_eq!(got.map(|(n, _)| n), Some("a".to_string()));
    }

    #[test]
    fn borrowed_ancilla_buried_in_returned_register_is_an_escape() {
        // `return (q, a)` — the ancilla is buried in a returned tuple, still an escape
        // (the `return (q `tensored` a)` case from the fixtures).
        let block = ret(sp(Expr::Tuple(vec![var("q"), var("a")])));
        let got = find_borrow_escape(&block, &borrowed(&["a"]));
        assert_eq!(got.map(|(n, _)| n), Some("a".to_string()));
    }

    // ── No-escape invariant: only *borrowed* names are flagged ─────────────────────

    #[test]
    fn return_of_a_non_borrowed_name_is_not_an_escape() {
        // `return q` where `q` is a data qubit (not borrowed) is a legitimate result.
        let block = ret(var("q"));
        assert!(find_borrow_escape(&block, &borrowed(&["a"])).is_none());
    }

    #[test]
    fn no_escape_when_nothing_is_borrowed() {
        // An empty borrow set can never flag an escape.
        let block = ret(var("a"));
        assert!(find_borrow_escape(&block, &borrowed(&[])).is_none());
    }

    // ── Cleanup classification: measure / reset / discard are valid cleanup (#180) ──
    //
    // A borrowed name in a *consuming* position does not escape — `measure(a)`, `reset(a)`,
    // `discard(a)` reclaim the ancilla (issue #180: measure/reset/discard are all valid
    // cleanup, superseding ADR-0003's structural-only terminal). Only a `return` payload
    // escapes.

    #[test]
    fn discard_of_ancilla_does_not_escape() {
        // `x <- discard(a); return x` — `a` is consumed by `discard`; the returned `x` is the
        // unit payload, not the ancilla.
        let block = bind("x", app("discard", var("a")), ret(var("x")));
        assert!(find_borrow_escape(&block, &borrowed(&["a"])).is_none());
    }

    #[test]
    fn measure_of_ancilla_does_not_escape() {
        // `x <- measure(a); return x` — `a` is consumed by `measure`; the returned `x` is the
        // measured `Bit`, not the ancilla. (#180: measure is valid cleanup.)
        let block = bind("x", app("measure", var("a")), ret(var("x")));
        assert!(find_borrow_escape(&block, &borrowed(&["a"])).is_none());
    }

    #[test]
    fn reset_of_ancilla_does_not_escape() {
        // `x <- reset(a); return x` — `a` is consumed by `reset`; the returned `x` is the
        // reset qubit handle, not the original ancilla.
        let block = bind("x", app("reset", var("a")), ret(var("x")));
        assert!(find_borrow_escape(&block, &borrowed(&["a"])).is_none());
    }

    #[test]
    fn borrowed_ancilla_consumed_then_a_different_value_returned_is_no_escape() {
        // `discard(a)` then `return q`: the ancilla is reclaimed, a data qubit is returned.
        let block = bind("_", app("discard", var("a")), ret(var("q")));
        assert!(find_borrow_escape(&block, &borrowed(&["a"])).is_none());
    }

    // ── first_borrowed_var: first hit in a returned value ──────────────────────────

    #[test]
    fn first_borrowed_var_finds_the_borrowed_component() {
        // `(q, a)` returned: the borrowed `a` is found even alongside a non-borrowed `q`.
        let v = sp(Expr::Tuple(vec![var("q"), var("a")]));
        assert_eq!(
            first_borrowed_var(&v, &borrowed(&["a"])),
            Some("a".to_string())
        );
        // A returned value with no borrowed name yields nothing.
        assert!(first_borrowed_var(&var("q"), &borrowed(&["a"])).is_none());
    }

    // ── End-to-end: #180 resolution locked at the module level ─────────────────────

    /// Whole-program check through the desugar pass (the `run { }` / borrow pipeline).
    fn check_run(src: &str) -> Result<(), Vec<TypeError>> {
        let decls = crate::desugar::desugar_decls(parse_program(src).expect("parse failed"))
            .expect("desugar reported errors");
        super::super::TypeChecker::new().check_decls(&decls)
    }

    #[test]
    fn borrow_measure_only_cleanup_is_accepted() {
        // Issue #180: `measure(a)` alone is valid borrow cleanup (not only structural
        // reset/discard). ADR-0003's structural-only terminal is superseded.
        let src = "fn f(): Q<Bit> = run {\n  borrow a: Qubit in {\n    measure(a)\n  }\n}";
        assert!(
            check_run(src).is_ok(),
            "measure-only cleanup must type-check"
        );
    }
}
