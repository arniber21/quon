//! Bidirectional type checker — classical (unrestricted) fragment (issue #9, SPEC §3.8).
//!
//! Judgment form (this issue uses only the unrestricted context):
//!
//! ```text
//!   Γ ⊢ e ⇒ τ      synthesis: read the type off the term bottom-up
//!   Γ ⊢ e ⇐ τ      checking:  push an expected type top-down
//! ```
//!
//! Where `Γ` (here `Env`) maps names to types. The linear context `Δ`, the quantum
//! forms (`circuit`/`run`/`borrow`/gates), and the symbolic depth/Clifford machinery
//! arrive with later issues (#10–#15); every such form is reported here as a clean
//! [`TypeError::Unsupported`] rather than mishandled or panicked on, so the checker is
//! *total* over the whole parser output — which the fuzz target relies on.
//!
//! ## Design
//!
//! * **Bidirectional, not full inference.** User functions are fully annotated, so the
//!   only polymorphism is the classical prelude (`map`, `fold`, `zip`, …). Those are
//!   [`Scheme`]s instantiated with fresh metavariables at each use; everything else flows
//!   through synthesis and checking. There is no let-generalization.
//! * **One unifier.** Application, branch joining, and subsumption all bottom out in
//!   [`Table::unify`]. Metavariables are zonked away before a type is returned to a caller.
//! * **Exhaustiveness** is delegated to the [`exhaust`] usefulness algorithm.

mod builtins;
mod error;
mod exhaust;
mod linear;
mod unify;

#[cfg(test)]
mod tests;

pub use error::TypeError;

use crate::ast::{BinOp, Decl, Expr, Name, NatExpr, Pat, Type};
use crate::lexer::{SimpleSpan, Sp};
use crate::types::Ty;
use builtins::Scheme;
use linear::Delta;
use std::collections::{BTreeSet, HashMap};
use unify::Table;

use quon_core::DepthExpr;

/// The unrestricted context Γ: names in scope mapped to their (monomorphic) types.
type Env = HashMap<Name, Ty>;

/// A resolved type alias: its formal `Nat` parameters and its right-hand side.
#[derive(Clone)]
struct AliasDef {
    params: Vec<Name>,
    body: Sp<Type>,
}

/// The classical-fragment type checker. Holds the global signature environment, the
/// alias table, and the metavariable substitution shared across one checking run.
pub struct TypeChecker {
    /// Top-level function signatures, available to every body (enables mutual recursion).
    globals: Env,
    /// Type aliases declared with `type`.
    aliases: HashMap<Name, AliasDef>,
    /// Metavariable substitution for this run.
    table: Table,
    /// Deferred "must be `Int` or `Float`" obligations on metavariables, discharged at the
    /// end of each body. Arithmetic on an unsolved metavariable records one of these instead
    /// of eagerly defaulting, so a later unification (e.g. against a list's element type) can
    /// still pin it — without losing the numeric check if it ends up non-numeric.
    numeric: Vec<(u32, SimpleSpan)>,
    /// Stack of enclosing-scope linear-resource names, one frame per lambda we are inside.
    /// A lambda body must not consume a resource from an outer scope (a function value may
    /// run zero or many times), so a reference to any name in these frames is a
    /// [`TypeError::LinearCapture`] rather than a successful linear use.
    lambda_linears: Vec<BTreeSet<String>>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            globals: Env::new(),
            aliases: HashMap::new(),
            table: Table::new(),
            numeric: Vec::new(),
            lambda_linears: Vec::new(),
        }
    }

    /// Type-checks a whole program: collects aliases and function signatures, then checks
    /// each function body against its declared return type.
    ///
    /// Errors are collected per declaration (the first error in a body aborts *that* body,
    /// but checking continues with the next), so one broken function does not mask the rest.
    pub fn check_decls(&mut self, decls: &[Sp<Decl>]) -> Result<(), Vec<TypeError>> {
        let mut errors = Vec::new();

        // Pass 1: aliases. Needed before any signature is resolved.
        for (decl, _) in decls {
            if let Decl::TypeAlias { name, params, ty } = decl {
                self.aliases.insert(
                    name.clone(),
                    AliasDef {
                        params: params.clone(),
                        body: ty.clone(),
                    },
                );
            }
        }

        // Pass 2: function signatures into Γ (so calls resolve regardless of order).
        for (decl, _) in decls {
            if let Decl::Fn {
                name, params, ret, ..
            } = decl
            {
                match self.fn_type(params, ret) {
                    Ok(ty) => {
                        self.globals.insert(name.clone(), ty);
                    }
                    Err(e) => errors.push(e),
                }
            }
        }

        // Pass 3: check each body.
        for (decl, _) in decls {
            if let Decl::Fn {
                params, ret, body, ..
            } = decl
                && let Err(e) = self.check_fn_body(params, ret, body)
            {
                errors.push(e);
            }
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Test/diagnostic hook: synthesize the body type of the *last* function in `decls`,
    /// ignoring its declared return type. Lets tests assert the exact inferred type.
    pub fn synth_last_body(&mut self, decls: &[Sp<Decl>]) -> Result<Ty, TypeError> {
        for (decl, _) in decls {
            if let Decl::TypeAlias { name, params, ty } = decl {
                self.aliases.insert(
                    name.clone(),
                    AliasDef {
                        params: params.clone(),
                        body: ty.clone(),
                    },
                );
            }
        }
        for (decl, _) in decls {
            if let Decl::Fn {
                name, params, ret, ..
            } = decl
                && let Ok(ty) = self.fn_type(params, ret)
            {
                self.globals.insert(name.clone(), ty);
            }
        }
        let last_fn = decls.iter().rev().find_map(|(d, _)| match d {
            Decl::Fn {
                params, body, ret, ..
            } => Some((params, body, ret)),
            _ => None,
        });
        let (params, body, _ret) = last_fn.ok_or_else(|| TypeError::Unsupported {
            construct: "program with no function",
            span: (0..0).into(),
        })?;
        let mut env = self.globals.clone();
        let mut delta = Delta::new();
        let introduced = self.bind_fn_params(params, &mut env, &mut delta)?;
        let ty = self.synth(&env, &mut delta, body)?;
        self.finalize_numeric()?;
        self.ensure_consumed(&delta, &introduced)?;
        Ok(self.table.zonk(&ty))
    }

    // ── Declarations ─────────────────────────────────────────────────────────

    /// The function's own (curried) type: `T₁ -> T₂ -> … -> Tₙ -> R`. The arity convention
    /// matches the call syntax, which the parser lowers to nested single-argument `App`s:
    /// `f()` is `App(f, ())` (so a nullary `f` has type `Unit -> R`), `f(x)` is `App(f, x)`,
    /// and `f(x, y)` is `App(App(f, x), y)`.
    fn fn_type(&mut self, params: &[(Name, Sp<Type>)], ret: &Sp<Type>) -> Result<Ty, TypeError> {
        let ret_ty = self.resolve_type(ret)?;
        if params.is_empty() {
            return Ok(Ty::func(Ty::Unit, ret_ty));
        }
        let mut ty = ret_ty;
        for (_, t) in params.iter().rev() {
            let pty = self.resolve_type(t)?;
            ty = Ty::func(pty, ty);
        }
        Ok(ty)
    }

    /// Bind a function's parameters, routing each by linearity: a linear-resource parameter
    /// goes into `Δ` (and is returned in the introduced-names list so the body's scope exit
    /// can demand it was consumed), an unrestricted one goes into `Γ`. The binding span is
    /// the parameter's type annotation (parameter names carry no span of their own).
    fn bind_fn_params(
        &mut self,
        params: &[(Name, Sp<Type>)],
        env: &mut Env,
        delta: &mut Delta,
    ) -> Result<Vec<(String, SimpleSpan)>, TypeError> {
        let mut introduced = Vec::new();
        for (name, t) in params {
            let ty = self.resolve_type(t)?;
            if ty.is_linear_resource() {
                delta.introduce(name.clone(), ty, t.1)?;
                introduced.push((name.clone(), t.1));
            } else {
                env.insert(name.clone(), ty);
            }
        }
        Ok(introduced)
    }

    fn check_fn_body(
        &mut self,
        params: &[(Name, Sp<Type>)],
        ret: &Sp<Type>,
        body: &Sp<Expr>,
    ) -> Result<(), TypeError> {
        let mut env = self.globals.clone();
        let mut delta = Delta::new();
        let introduced = self.bind_fn_params(params, &mut env, &mut delta)?;
        let ret_ty = self.resolve_type(ret)?;
        self.check(&env, &mut delta, body, &ret_ty)?;
        self.finalize_numeric()?;
        self.ensure_consumed(&delta, &introduced)
    }

    /// Scope-exit check: every linear name a scope introduced must have been consumed by the
    /// time its body finished. A name still live in `Δ` is a dropped resource (no-dropping).
    fn ensure_consumed(
        &self,
        delta: &Delta,
        introduced: &[(String, SimpleSpan)],
    ) -> Result<(), TypeError> {
        for (name, span) in introduced {
            if delta.is_available(name) {
                return Err(TypeError::LinearUnconsumed {
                    name: name.clone(),
                    span: *span,
                });
            }
        }
        Ok(())
    }

    // ── Synthesis (⇒) ──────────────────────────────────────────────────────────

    /// Synthesize the type of `expr` under `env`/`delta`. `delta` is threaded *mutably*: a
    /// linear resource referenced here is physically removed from it, so sequential sub-terms
    /// (e.g. the two operands of an application) automatically receive disjoint slices of the
    /// linear context and a re-use surfaces as [`TypeError::LinearUsedTwice`].
    fn synth(&mut self, env: &Env, delta: &mut Delta, expr: &Sp<Expr>) -> Result<Ty, TypeError> {
        let span = expr.1;
        match &expr.0 {
            Expr::Int(_) => Ok(Ty::Int),
            Expr::Float(_) => Ok(Ty::Float),
            Expr::Bool(_) => Ok(Ty::Bool),
            Expr::Unit => Ok(Ty::Unit),

            Expr::Var(name) => self.synth_var(env, delta, name, span),

            // `destructure(q)` is the tensor-elimination form (SPEC §3.4): it consumes a
            // `QReg<n>` and yields an n-tuple of fresh `Qubit`s. Intercepted before the
            // generic application rule because its result arity depends on the argument type.
            Expr::App(f, x) if is_named_call(f, "destructure") => {
                self.synth_destructure(env, delta, x, span)
            }

            Expr::App(f, x) => {
                let f_ty = self.synth(env, delta, f)?;
                let (dom, cod) = self.as_function(&f_ty, f.1)?;
                self.check(env, delta, x, &dom)?;
                Ok(cod)
            }

            Expr::BinOp { op, lhs, rhs } => self.synth_arith(env, delta, *op, lhs, rhs, span),
            Expr::Neg(e) => {
                let t = self.synth(env, delta, e)?;
                self.numeric(&t, e.1)
            }

            Expr::Tuple(es) => self.synth_tuple(env, delta, es),

            Expr::List(es) => self.synth_list(env, delta, es, span),

            Expr::Let { pat, rhs, body } => {
                let rhs_ty = self.synth(env, delta, rhs)?;
                let mut inner = env.clone();
                let introduced = self.bind_pat(pat, &rhs_ty, &mut inner, delta)?;
                let ty = self.synth(&inner, delta, body)?;
                self.ensure_consumed(delta, &introduced)?;
                Ok(ty)
            }

            Expr::If { cond, then, else_ } => self.branch_if(env, delta, cond, then, else_, None),

            Expr::Match { scrutinee, arms } => self.check_match(env, delta, scrutinee, arms, None),

            Expr::Lam { params, body } => self.synth_lambda(env, delta, params, body, span),

            Expr::Ascribe(e, ty) => {
                let resolved = self.resolve_type(ty)?;
                self.check(env, delta, e, &resolved)?;
                Ok(resolved)
            }

            // Linear/quantum fragment — handled in issues #11–#15.
            _ => Err(self.unsupported(&expr.0, span)),
        }
    }

    fn synth_var(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        name: &str,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        // User bindings (locals and globals) shadow the prelude. Unrestricted names live in
        // `Γ` and leave `Δ` untouched.
        if let Some(ty) = env.get(name) {
            return Ok(ty.clone());
        }
        // A linear resource: consuming it removes it from `Δ`; a second use is no-cloning.
        if let Some(result) = delta.try_consume(name, span) {
            return result;
        }
        if let Some(scheme) = builtins::lookup(name) {
            return Ok(self.instantiate(&scheme));
        }
        // A linear resource owned by an enclosing scope, referenced inside a lambda: a closure
        // cannot consume it exactly once, so this is a capture error, not a successful use.
        if self.lambda_linears.iter().any(|frame| frame.contains(name)) {
            return Err(TypeError::LinearCapture {
                name: name.to_string(),
                span,
            });
        }
        // Recognise quantum-prelude names for a clearer message than "unbound".
        if is_quantum_builtin(name) {
            return Err(TypeError::Unsupported {
                construct: "quantum prelude",
                span,
            });
        }
        Err(TypeError::UnboundVariable {
            name: name.to_string(),
            span,
        })
    }

    /// Tensor elimination: `destructure(arg)` consumes `arg : QReg<n>` and produces an
    /// n-tuple of `Qubit`s (each an independent linear resource, SPEC §4.6). The size `n`
    /// must be statically known from the register type.
    fn synth_destructure(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        arg: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let arg_ty = self.synth(env, delta, arg)?;
        match self.table.resolve(&arg_ty) {
            Ty::QReg(n) => Ok(Ty::Tuple(vec![Ty::Qubit; n as usize])),
            other => Err(TypeError::Mismatch {
                expected: Ty::QReg(0),
                found: other,
                span,
            }),
        }
    }

    /// Synthesize a tuple. An all-`Qubit` tuple is the tensor *introduction* form and
    /// synthesizes to `QReg<n>` (SPEC §3.4); any other tuple keeps its component types.
    /// Components are synthesized left-to-right, threading `Δ`, so `(q, q)` fails at the
    /// second `q` (no-cloning) before a register is ever formed.
    fn synth_tuple(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        es: &[Sp<Expr>],
    ) -> Result<Ty, TypeError> {
        let tys: Vec<Ty> = es
            .iter()
            .map(|e| self.synth(env, delta, e))
            .collect::<Result<_, _>>()?;
        if !tys.is_empty()
            && tys
                .iter()
                .all(|t| matches!(self.table.resolve(t), Ty::Qubit))
        {
            return Ok(Ty::QReg(tys.len() as u64));
        }
        Ok(Ty::Tuple(tys))
    }

    fn synth_arith(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        _op: BinOp,
        lhs: &Sp<Expr>,
        rhs: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let lt = self.synth(env, delta, lhs)?;
        let rt = self.synth(env, delta, rhs)?;
        self.table.unify(&lt, &rt, span)?;
        self.numeric(&lt, span)
    }

    /// Resolve `t` and require it to be `Int` or `Float`. An unsolved metavariable defers:
    /// the obligation is recorded and the metavariable is returned unchanged, so surrounding
    /// context can still solve it. [`Self::finalize_numeric`] discharges the obligation
    /// (defaulting a still-unsolved variable to `Int`) once the body is fully checked.
    fn numeric(&mut self, t: &Ty, span: SimpleSpan) -> Result<Ty, TypeError> {
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
    fn finalize_numeric(&mut self) -> Result<(), TypeError> {
        for (id, span) in std::mem::take(&mut self.numeric) {
            match self.table.resolve(&Ty::Meta(id)) {
                Ty::Int | Ty::Float => {}
                Ty::Meta(_) => self.table.unify(&Ty::Meta(id), &Ty::Int, span)?,
                other => return Err(TypeError::NotNumeric { found: other, span }),
            }
        }
        Ok(())
    }

    fn synth_list(
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

    fn synth_lambda(
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
    fn in_lambda_scope<R>(
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

    // ── Checking (⇐) ───────────────────────────────────────────────────────────

    /// Check `expr` against the expected type `expected` under `env`/`delta`.
    fn check(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        expr: &Sp<Expr>,
        expected: &Ty,
    ) -> Result<(), TypeError> {
        let span = expr.1;
        match &expr.0 {
            // Lambdas are the canonical checking form: push the domain into the parameters.
            Expr::Lam { params, body } => {
                self.check_lambda(env, delta, params, body, expected, span)
            }

            Expr::If { cond, then, else_ } => {
                self.branch_if(env, delta, cond, then, else_, Some(expected))?;
                Ok(())
            }

            Expr::Match { scrutinee, arms } => {
                self.check_match(env, delta, scrutinee, arms, Some(expected))?;
                Ok(())
            }

            Expr::Let { pat, rhs, body } => {
                let rhs_ty = self.synth(env, delta, rhs)?;
                let mut inner = env.clone();
                let introduced = self.bind_pat(pat, &rhs_ty, &mut inner, delta)?;
                self.check(&inner, delta, body, expected)?;
                self.ensure_consumed(delta, &introduced)
            }

            Expr::Tuple(es) => match self.table.resolve(expected) {
                Ty::Tuple(ts) if ts.len() == es.len() => {
                    for (e, t) in es.iter().zip(&ts) {
                        self.check(env, delta, e, t)?;
                    }
                    Ok(())
                }
                // Tensor introduction in checking mode: a tuple may inhabit `QReg<n>` when
                // each of its `n` components is a `Qubit`.
                Ty::QReg(n) if n as usize == es.len() => {
                    for e in es {
                        self.check(env, delta, e, &Ty::Qubit)?;
                    }
                    Ok(())
                }
                Ty::Meta(_) => self.subsume(env, delta, expr, expected),
                other => Err(TypeError::Mismatch {
                    expected: other,
                    found: Ty::Tuple(vec![self.table.fresh(); es.len()]),
                    span,
                }),
            },

            Expr::List(es) => match self.table.resolve(expected) {
                Ty::List(elem) => {
                    for e in es {
                        self.check(env, delta, e, &elem)?;
                    }
                    Ok(())
                }
                Ty::Meta(_) => self.subsume(env, delta, expr, expected),
                other => Err(TypeError::Mismatch {
                    expected: other,
                    found: Ty::list(self.table.fresh()),
                    span,
                }),
            },

            // Default: synthesize and demand the result subsume the expectation.
            _ => self.subsume(env, delta, expr, expected),
        }
    }

    /// The subsumption rule: `Γ ; Δ ⊢ e ⇒ τ'`, then `τ' = τ`.
    fn subsume(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        expr: &Sp<Expr>,
        expected: &Ty,
    ) -> Result<(), TypeError> {
        let got = self.synth(env, delta, expr)?;
        self.table.unify(expected, &got, expr.1)
    }

    /// Type an `if`/`then`/`else`, shared by synthesis (`expected = None`) and checking
    /// (`expected = Some(τ)`). The condition is checked first, threading `Δ` (it may consume
    /// resources both branches then share). Each branch runs against a *clone* of the
    /// post-condition `Δ`; the residuals must agree, so a resource spent on one path but not
    /// the other is rejected ([`TypeError::LinearBranchMismatch`]). The merged residual
    /// becomes the surrounding `Δ`.
    fn branch_if(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        cond: &Sp<Expr>,
        then: &Sp<Expr>,
        else_: &Sp<Expr>,
        expected: Option<&Ty>,
    ) -> Result<Ty, TypeError> {
        self.check(env, delta, cond, &Ty::Bool)?;
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
                self.check(env, &mut d_else, else_, &then_ty)?;
                then_ty
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

    /// Check a lambda against an expected (curried) function type by peeling one arrow per
    /// parameter: for `fn(p₁, …, pₙ) -> body ⇐ τ`, each `pᵢ` consumes one `Dᵢ -> …` layer
    /// and the body is checked against whatever type remains. A nullary lambda expects
    /// `Unit -> τ`.
    fn check_lambda(
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
    fn check_match(
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

        // The arms' result type: the expected type, or a fresh metavariable to join into.
        let result = match expected {
            Some(t) => t.clone(),
            None => self.table.fresh(),
        };

        // Each arm gets a clone of `Δ`; its pattern may bind further linear resources, which
        // (like any scope) must be consumed within the arm. The residual ambient resources
        // are collected for the join.
        let mut arm_deltas: Vec<(Delta, SimpleSpan)> = Vec::with_capacity(arms.len());
        for (pat, body) in arms {
            let mut inner = env.clone();
            let mut d_arm = delta.clone();
            let introduced = self.check_pat(pat, &scrut_ty, &mut inner, &mut d_arm)?;
            self.check(&inner, &mut d_arm, body, &result)?;
            self.ensure_consumed(&d_arm, &introduced)?;
            arm_deltas.push((d_arm, body.1));
        }

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
    fn bind_pat(
        &mut self,
        pat: &Sp<Pat>,
        ty: &Ty,
        env: &mut Env,
        delta: &mut Delta,
    ) -> Result<Vec<(String, SimpleSpan)>, TypeError> {
        let mut introduced = Vec::new();
        self.check_pat_into(pat, ty, env, delta, &mut introduced)?;
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
    fn check_pat_into(
        &mut self,
        pat: &Sp<Pat>,
        ty: &Ty,
        env: &mut Env,
        delta: &mut Delta,
        introduced: &mut Vec<(String, SimpleSpan)>,
    ) -> Result<(), TypeError> {
        let span = pat.1;
        match &pat.0 {
            // A wildcard over a linear resource is a silent discard — rejected (no-dropping).
            Pat::Wildcard => {
                let resolved = self.table.resolve(ty);
                if resolved.is_linear_resource() {
                    return Err(TypeError::LinearDiscard {
                        name: resolved.to_string(),
                        span,
                    });
                }
                Ok(())
            }
            Pat::Var(name) => {
                let resolved = self.table.resolve(ty);
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
                    crate::ast::LitPat::Int(_) => Ty::Int,
                    crate::ast::LitPat::Bool(_) => Ty::Bool,
                };
                self.table.unify(ty, &lit_ty, span)
            }
            Pat::Tuple(ps) => match self.table.resolve(ty) {
                Ty::Tuple(ts) if ts.len() == ps.len() => {
                    for (p, t) in ps.iter().zip(&ts) {
                        self.check_pat_into(p, t, env, delta, introduced)?;
                    }
                    Ok(())
                }
                Ty::Tuple(ts) => Err(TypeError::ArityMismatch {
                    expected: ts.len(),
                    found: ps.len(),
                    span,
                }),
                // A `QReg<n>` destructured by an n-tuple pattern: each slot is a `Qubit`.
                Ty::QReg(n) if n as usize == ps.len() => {
                    for p in ps {
                        self.check_pat_into(p, &Ty::Qubit, env, delta, introduced)?;
                    }
                    Ok(())
                }
                Ty::QReg(n) => Err(TypeError::ArityMismatch {
                    expected: n as usize,
                    found: ps.len(),
                    span,
                }),
                Ty::Meta(_) => {
                    let fresh: Vec<Ty> = (0..ps.len()).map(|_| self.table.fresh()).collect();
                    self.table.unify(ty, &Ty::Tuple(fresh.clone()), span)?;
                    for (p, t) in ps.iter().zip(&fresh) {
                        self.check_pat_into(p, t, env, delta, introduced)?;
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
    fn as_function(&mut self, ty: &Ty, span: SimpleSpan) -> Result<(Ty, Ty), TypeError> {
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

    fn instantiate(&mut self, scheme: &Scheme) -> Ty {
        scheme.instantiate(&mut self.table)
    }

    fn unsupported(&self, expr: &Expr, span: SimpleSpan) -> TypeError {
        TypeError::Unsupported {
            construct: construct_name(expr),
            span,
        }
    }

    // ── Surface type resolution ────────────────────────────────────────────────

    /// Lower a surface [`Type`] into a checker [`Ty`]. Classical types resolve fully;
    /// quantum types resolve structurally when their `Nat` indices are literal.
    fn resolve_type(&mut self, ty: &Sp<Type>) -> Result<Ty, TypeError> {
        self.resolve_type_at(ty, &mut Vec::new())
    }

    /// `visiting` is the stack of alias names currently being expanded on this path. It is
    /// the cycle guard: a recursive alias (`type A = A<…>`, directly or transitively) is
    /// rejected the moment it re-enters, *before* its arguments can grow without bound — a
    /// plain depth counter is not enough because each expansion can multiply the type's size.
    fn resolve_type_at(
        &mut self,
        ty: &Sp<Type>,
        visiting: &mut Vec<String>,
    ) -> Result<Ty, TypeError> {
        let span = ty.1;
        Ok(match &ty.0 {
            Type::Bool => Ty::Bool,
            Type::Int => Ty::Int,
            Type::Float => Ty::Float,
            Type::Unit => Ty::Unit,
            Type::Bit => Ty::Bit,
            // `Nat` at the value level is a static natural — typed as `Int` for arithmetic.
            Type::Nat => Ty::Int,
            Type::Qubit => Ty::Qubit,
            Type::QReg(n) => Ty::QReg(self.eval_nat(n)?),
            Type::List(t) => Ty::list(self.resolve_type_at(t, visiting)?),
            Type::Tuple(ts) => Ty::Tuple(
                ts.iter()
                    .map(|t| self.resolve_type_at(t, visiting))
                    .collect::<Result<_, _>>()?,
            ),
            Type::Fn(a, b) => Ty::func(
                self.resolve_type_at(a, visiting)?,
                self.resolve_type_at(b, visiting)?,
            ),
            Type::Linear(a, b) => Ty::Linear(
                Box::new(self.resolve_type_at(a, visiting)?),
                Box::new(self.resolve_type_at(b, visiting)?),
            ),
            Type::Q(t) => Ty::Q(Box::new(self.resolve_type_at(t, visiting)?)),
            Type::Matrix(n, m, t) => Ty::Matrix(
                self.eval_nat(n)?,
                self.eval_nat(m)?,
                Box::new(self.resolve_type_at(t, visiting)?),
            ),
            Type::Circuit { n, m, d, c } => Ty::Circuit {
                n: self.eval_nat(n)?,
                m: self.eval_nat(m)?,
                d: self.nat_to_depth(d)?,
                c: c.clone(),
            },
            Type::Var(name) => self.resolve_named(name, &[], span, visiting)?,
            Type::Named { name, args } => self.resolve_named(name, args, span, visiting)?,
        })
    }

    /// Resolve a (possibly parameterized) type-name reference: expand an alias, or treat
    /// an unknown bare name as a rigid type variable.
    fn resolve_named(
        &mut self,
        name: &str,
        args: &[Sp<NatExpr>],
        span: SimpleSpan,
        visiting: &mut Vec<String>,
    ) -> Result<Ty, TypeError> {
        let Some(def) = self.aliases.get(name).cloned() else {
            // No alias by this name. With no arguments it is a free type variable.
            if args.is_empty() {
                return Ok(Ty::Var(name.to_string()));
            }
            return Err(TypeError::UnboundVariable {
                name: name.to_string(),
                span,
            });
        };
        // A name already on the expansion path is a cyclic alias — reject before it blows up.
        if visiting.iter().any(|n| n == name) {
            return Err(TypeError::Unsupported {
                construct: "recursive type alias",
                span,
            });
        }
        if def.params.len() != args.len() {
            return Err(TypeError::AliasArity {
                name: name.to_string(),
                expected: def.params.len(),
                found: args.len(),
                span,
            });
        }
        let subst: HashMap<&str, &NatExpr> = def
            .params
            .iter()
            .map(|p| p.as_str())
            .zip(args.iter().map(|a| &a.0))
            .collect();
        let expanded = subst_nat_in_type(&def.body, &subst);
        visiting.push(name.to_string());
        let resolved = self.resolve_type_at(&expanded, visiting);
        visiting.pop();
        resolved
    }

    /// Evaluate a `Nat` expression to a concrete `u64`, when it is closed and literal.
    /// Symbolic naturals are rejected — the classical fragment has no value-dependent types.
    fn eval_nat(&self, n: &Sp<NatExpr>) -> Result<u64, TypeError> {
        eval_nat(&n.0).ok_or(TypeError::Unsupported {
            construct: "symbolic Nat in type",
            span: n.1,
        })
    }

    /// Convert a surface depth annotation to a [`DepthExpr`]. Best-effort for the classical
    /// fragment: literals/vars/`+`/`*` map directly; richer forms defer to issue #13.
    fn nat_to_depth(&self, n: &Sp<NatExpr>) -> Result<DepthExpr, TypeError> {
        nat_to_depth(&n.0).ok_or(TypeError::Unsupported {
            construct: "depth expression",
            span: n.1,
        })
    }
}

// ── Free helpers ───────────────────────────────────────────────────────────────

/// Whether `callee` is exactly the bare variable `name` (used to recognise special-form
/// applications like `destructure(q)` before the generic application rule fires).
fn is_named_call(callee: &Sp<Expr>, name: &str) -> bool {
    matches!(&callee.0, Expr::Var(n) if n == name)
}

fn eval_nat(n: &NatExpr) -> Option<u64> {
    Some(match n {
        NatExpr::Lit(v) => *v,
        NatExpr::Var(_) | NatExpr::Hole => return None,
        NatExpr::Add(a, b) => eval_nat(&a.0)?.checked_add(eval_nat(&b.0)?)?,
        NatExpr::Mul(a, b) => eval_nat(&a.0)?.checked_mul(eval_nat(&b.0)?)?,
        NatExpr::Sub(a, b) => eval_nat(&a.0)?.saturating_sub(eval_nat(&b.0)?),
        NatExpr::Div(a, b) => {
            let d = eval_nat(&b.0)?;
            if d == 0 {
                return None;
            }
            eval_nat(&a.0)? / d
        }
        NatExpr::Exp(a, b) => {
            let exp = u32::try_from(eval_nat(&b.0)?).ok()?;
            eval_nat(&a.0)?.checked_pow(exp)?
        }
    })
}

fn nat_to_depth(n: &NatExpr) -> Option<DepthExpr> {
    Some(match n {
        NatExpr::Lit(v) => DepthExpr::Nat(*v),
        NatExpr::Var(name) => DepthExpr::Var(name.clone()),
        NatExpr::Add(a, b) => nat_to_depth(&a.0)?.seq(nat_to_depth(&b.0)?),
        NatExpr::Mul(a, b) => DepthExpr::repeat(nat_to_depth(&a.0)?, nat_to_depth(&b.0)?),
        // Sub/Div/Exp/Hole have no DepthExpr form yet — handled with Z3 in issue #13.
        _ => return None,
    })
}

/// Substitute `Nat` arguments for an alias's formal parameters throughout a type.
fn subst_nat_in_type(ty: &Sp<Type>, subst: &HashMap<&str, &NatExpr>) -> Sp<Type> {
    let span = ty.1;
    let out = match &ty.0 {
        Type::QReg(n) => Type::QReg(subst_nat(n, subst)),
        Type::List(t) => Type::List(Box::new(subst_nat_in_type(t, subst))),
        Type::Tuple(ts) => Type::Tuple(ts.iter().map(|t| subst_nat_in_type(t, subst)).collect()),
        Type::Fn(a, b) => Type::Fn(
            Box::new(subst_nat_in_type(a, subst)),
            Box::new(subst_nat_in_type(b, subst)),
        ),
        Type::Linear(a, b) => Type::Linear(
            Box::new(subst_nat_in_type(a, subst)),
            Box::new(subst_nat_in_type(b, subst)),
        ),
        Type::Q(t) => Type::Q(Box::new(subst_nat_in_type(t, subst))),
        Type::Matrix(n, m, t) => Type::Matrix(
            subst_nat(n, subst),
            subst_nat(m, subst),
            Box::new(subst_nat_in_type(t, subst)),
        ),
        Type::Circuit { n, m, d, c } => Type::Circuit {
            n: subst_nat(n, subst),
            m: subst_nat(m, subst),
            d: subst_nat(d, subst),
            c: c.clone(),
        },
        Type::Named { name, args } => Type::Named {
            name: name.clone(),
            args: args.iter().map(|a| subst_nat(a, subst)).collect(),
        },
        // A bare `Var` may itself be a parameter name standing in a Nat position.
        Type::Var(name) => match subst.get(name.as_str()) {
            Some(rep) => nat_to_type(rep, span),
            None => Type::Var(name.clone()),
        },
        other => other.clone(),
    };
    (out, span)
}

fn subst_nat(n: &Sp<NatExpr>, subst: &HashMap<&str, &NatExpr>) -> Sp<NatExpr> {
    let span = n.1;
    let out = match &n.0 {
        NatExpr::Var(name) => match subst.get(name.as_str()) {
            Some(rep) => (*rep).clone(),
            None => NatExpr::Var(name.clone()),
        },
        NatExpr::Lit(v) => NatExpr::Lit(*v),
        NatExpr::Hole => NatExpr::Hole,
        NatExpr::Add(a, b) => {
            NatExpr::Add(Box::new(subst_nat(a, subst)), Box::new(subst_nat(b, subst)))
        }
        NatExpr::Sub(a, b) => {
            NatExpr::Sub(Box::new(subst_nat(a, subst)), Box::new(subst_nat(b, subst)))
        }
        NatExpr::Mul(a, b) => {
            NatExpr::Mul(Box::new(subst_nat(a, subst)), Box::new(subst_nat(b, subst)))
        }
        NatExpr::Div(a, b) => {
            NatExpr::Div(Box::new(subst_nat(a, subst)), Box::new(subst_nat(b, subst)))
        }
        NatExpr::Exp(a, b) => {
            NatExpr::Exp(Box::new(subst_nat(a, subst)), Box::new(subst_nat(b, subst)))
        }
    };
    (out, span)
}

/// A `Nat` standing in for a bare type variable becomes a `Var`/`Named` type again so it
/// can be re-resolved (the value lives in the alias-argument position).
fn nat_to_type(n: &NatExpr, span: SimpleSpan) -> Type {
    match n {
        NatExpr::Var(name) => Type::Var(name.clone()),
        _ => Type::Named {
            name: "_nat".into(),
            args: vec![(n.clone(), span)],
        },
    }
}

/// A short human name for an unsupported expression form, for the diagnostic.
fn construct_name(expr: &Expr) -> &'static str {
    match expr {
        Expr::CircuitBlock(_) => "circuit block",
        Expr::Compose(..) => "|> composition",
        Expr::Par(..) => "par",
        Expr::Adjoint(_) => "adjoint",
        Expr::Controlled(_) => "controlled",
        Expr::GateApp { .. } => "gate application",
        Expr::RunBlock(_) => "run block",
        Expr::Bind { .. } => "monadic bind",
        Expr::Return(_) => "return",
        Expr::Borrow { .. } => "borrow",
        Expr::For { .. } => "for",
        _ => "expression",
    }
}

/// Names from the quantum/linear prelude (gates, allocation, measurement, combinators).
/// Used only to upgrade an "unbound variable" message to "not yet type-checked".
fn is_quantum_builtin(name: &str) -> bool {
    const NAMES: &[&str] = &[
        // allocation / registers / measurement
        "qreg",
        "qubit",
        "destructure",
        "split",
        "tensored",
        "measure",
        "measure_x",
        "measure_y",
        "measure_all",
        "reset",
        "discard",
        "apply",
        "apply_dyn",
        "init_one",
        "init_plus",
        "map_q",
        "sequence_q",
        "return",
        // circuit combinators
        "identity",
        "adjoint",
        "controlled",
        "repeat",
        "on_high",
        "on_low",
        "swap_reverse",
        // single-qubit gates
        "I",
        "X",
        "Y",
        "Z",
        "H",
        "S",
        "S_dag",
        "T",
        "T_dag",
        "Rx",
        "Ry",
        "Rz",
        "SX",
        "SX_dag",
        // two-qubit gates
        "CNOT",
        "CX",
        "CY",
        "CZ",
        "SWAP",
        "iSWAP",
        "ECR",
        "Rzz",
        "Rxx",
        "Ryy",
        "CRz",
        "CRx",
        "CP",
    ];
    NAMES.contains(&name)
}

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}
