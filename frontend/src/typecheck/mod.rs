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
mod unify;

#[cfg(test)]
mod tests;

pub use error::TypeError;

use crate::ast::{BinOp, Decl, Expr, Name, NatExpr, Pat, Type};
use crate::lexer::{SimpleSpan, Sp};
use crate::types::Ty;
use builtins::Scheme;
use std::collections::HashMap;
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
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            globals: Env::new(),
            aliases: HashMap::new(),
            table: Table::new(),
            numeric: Vec::new(),
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
        self.bind_fn_params(params, &mut env)?;
        let ty = self.synth(&env, body)?;
        self.finalize_numeric()?;
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

    fn bind_fn_params(
        &mut self,
        params: &[(Name, Sp<Type>)],
        env: &mut Env,
    ) -> Result<(), TypeError> {
        for (name, t) in params {
            let ty = self.resolve_type(t)?;
            env.insert(name.clone(), ty);
        }
        Ok(())
    }

    fn check_fn_body(
        &mut self,
        params: &[(Name, Sp<Type>)],
        ret: &Sp<Type>,
        body: &Sp<Expr>,
    ) -> Result<(), TypeError> {
        let mut env = self.globals.clone();
        self.bind_fn_params(params, &mut env)?;
        let ret_ty = self.resolve_type(ret)?;
        self.check(&env, body, &ret_ty)?;
        self.finalize_numeric()
    }

    // ── Synthesis (⇒) ──────────────────────────────────────────────────────────

    /// Synthesize the type of `expr` under `env`.
    fn synth(&mut self, env: &Env, expr: &Sp<Expr>) -> Result<Ty, TypeError> {
        let span = expr.1;
        match &expr.0 {
            Expr::Int(_) => Ok(Ty::Int),
            Expr::Float(_) => Ok(Ty::Float),
            Expr::Bool(_) => Ok(Ty::Bool),
            Expr::Unit => Ok(Ty::Unit),

            Expr::Var(name) => self.synth_var(env, name, span),

            Expr::App(f, x) => {
                let f_ty = self.synth(env, f)?;
                let (dom, cod) = self.as_function(&f_ty, f.1)?;
                self.check(env, x, &dom)?;
                Ok(cod)
            }

            Expr::BinOp { op, lhs, rhs } => self.synth_arith(env, *op, lhs, rhs, span),
            Expr::Neg(e) => {
                let t = self.synth(env, e)?;
                self.numeric(&t, e.1)
            }

            Expr::Tuple(es) => {
                let tys = es
                    .iter()
                    .map(|e| self.synth(env, e))
                    .collect::<Result<_, _>>()?;
                Ok(Ty::Tuple(tys))
            }

            Expr::List(es) => self.synth_list(env, es, span),

            Expr::Let { pat, rhs, body } => {
                let rhs_ty = self.synth(env, rhs)?;
                let mut inner = env.clone();
                self.bind_pat(pat, &rhs_ty, &mut inner)?;
                self.synth(&inner, body)
            }

            Expr::If { cond, then, else_ } => {
                self.check(env, cond, &Ty::Bool)?;
                let then_ty = self.synth(env, then)?;
                self.check(env, else_, &then_ty)?;
                Ok(then_ty)
            }

            Expr::Match { scrutinee, arms } => self.check_match(env, scrutinee, arms, None),

            Expr::Lam { params, body } => self.synth_lambda(env, params, body, span),

            Expr::Ascribe(e, ty) => {
                let resolved = self.resolve_type(ty)?;
                self.check(env, e, &resolved)?;
                Ok(resolved)
            }

            // Linear/quantum fragment — handled in issues #10–#15.
            _ => Err(self.unsupported(&expr.0, span)),
        }
    }

    fn synth_var(&mut self, env: &Env, name: &str, span: SimpleSpan) -> Result<Ty, TypeError> {
        // User bindings (locals and globals) shadow the prelude.
        if let Some(ty) = env.get(name) {
            return Ok(ty.clone());
        }
        if let Some(scheme) = builtins::lookup(name) {
            return Ok(self.instantiate(&scheme));
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

    fn synth_arith(
        &mut self,
        env: &Env,
        _op: BinOp,
        lhs: &Sp<Expr>,
        rhs: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let lt = self.synth(env, lhs)?;
        let rt = self.synth(env, rhs)?;
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
        es: &[Sp<Expr>],
        _span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        match es.split_first() {
            None => Ok(Ty::list(self.table.fresh())),
            Some((head, tail)) => {
                let elem = self.synth(env, head)?;
                for e in tail {
                    self.check(env, e, &elem)?;
                }
                Ok(Ty::list(elem))
            }
        }
    }

    fn synth_lambda(
        &mut self,
        env: &Env,
        params: &[(Sp<Pat>, Option<Sp<Type>>)],
        body: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        // Synthesis only works when every parameter is annotated; otherwise the domain
        // is unconstrained and the lambda needs an expected type (checking mode).
        let mut dom_tys = Vec::with_capacity(params.len());
        let mut inner = env.clone();
        for (pat, ann) in params {
            let Some(ann) = ann else {
                return Err(TypeError::AmbiguousLambda { span });
            };
            let ty = self.resolve_type(ann)?;
            self.bind_pat(pat, &ty, &mut inner)?;
            dom_tys.push(ty);
        }
        let cod = self.synth(&inner, body)?;
        // Curry: a nullary lambda is `Unit -> cod`; otherwise `T₁ -> … -> Tₙ -> cod`.
        if dom_tys.is_empty() {
            return Ok(Ty::func(Ty::Unit, cod));
        }
        Ok(dom_tys
            .into_iter()
            .rev()
            .fold(cod, |acc, t| Ty::func(t, acc)))
    }

    // ── Checking (⇐) ───────────────────────────────────────────────────────────

    /// Check `expr` against the expected type `expected` under `env`.
    fn check(&mut self, env: &Env, expr: &Sp<Expr>, expected: &Ty) -> Result<(), TypeError> {
        let span = expr.1;
        match &expr.0 {
            // Lambdas are the canonical checking form: push the domain into the parameters.
            Expr::Lam { params, body } => self.check_lambda(env, params, body, expected, span),

            Expr::If { cond, then, else_ } => {
                self.check(env, cond, &Ty::Bool)?;
                self.check(env, then, expected)?;
                self.check(env, else_, expected)
            }

            Expr::Match { scrutinee, arms } => {
                self.check_match(env, scrutinee, arms, Some(expected))?;
                Ok(())
            }

            Expr::Let { pat, rhs, body } => {
                let rhs_ty = self.synth(env, rhs)?;
                let mut inner = env.clone();
                self.bind_pat(pat, &rhs_ty, &mut inner)?;
                self.check(&inner, body, expected)
            }

            Expr::Tuple(es) => match self.table.resolve(expected) {
                Ty::Tuple(ts) if ts.len() == es.len() => {
                    for (e, t) in es.iter().zip(&ts) {
                        self.check(env, e, t)?;
                    }
                    Ok(())
                }
                Ty::Meta(_) => self.subsume(env, expr, expected),
                other => Err(TypeError::Mismatch {
                    expected: other,
                    found: Ty::Tuple(vec![self.table.fresh(); es.len()]),
                    span,
                }),
            },

            Expr::List(es) => match self.table.resolve(expected) {
                Ty::List(elem) => {
                    for e in es {
                        self.check(env, e, &elem)?;
                    }
                    Ok(())
                }
                Ty::Meta(_) => self.subsume(env, expr, expected),
                other => Err(TypeError::Mismatch {
                    expected: other,
                    found: Ty::list(self.table.fresh()),
                    span,
                }),
            },

            // Default: synthesize and demand the result subsume the expectation.
            _ => self.subsume(env, expr, expected),
        }
    }

    /// The subsumption rule: `Γ ⊢ e ⇒ τ'`, then `τ' = τ`.
    fn subsume(&mut self, env: &Env, expr: &Sp<Expr>, expected: &Ty) -> Result<(), TypeError> {
        let got = self.synth(env, expr)?;
        self.table.unify(expected, &got, expr.1)
    }

    /// Check a lambda against an expected (curried) function type by peeling one arrow per
    /// parameter: for `fn(p₁, …, pₙ) -> body ⇐ τ`, each `pᵢ` consumes one `Dᵢ -> …` layer
    /// and the body is checked against whatever type remains. A nullary lambda expects
    /// `Unit -> τ`.
    fn check_lambda(
        &mut self,
        env: &Env,
        params: &[(Sp<Pat>, Option<Sp<Type>>)],
        body: &Sp<Expr>,
        expected: &Ty,
        span: SimpleSpan,
    ) -> Result<(), TypeError> {
        let mut inner = env.clone();
        let mut current = expected.clone();

        if params.is_empty() {
            let (dom, cod) = self.as_function(&current, span)?;
            self.table.unify(&dom, &Ty::Unit, span)?;
            return self.check(&inner, body, &cod);
        }

        for (pat, ann) in params {
            let (dom, cod) = self.as_function(&current, span)?;
            if let Some(ann) = ann {
                let annotated = self.resolve_type(ann)?;
                self.table.unify(&annotated, &dom, ann.1)?;
            }
            self.bind_pat(pat, &dom, &mut inner)?;
            current = cod;
        }
        self.check(&inner, body, &current)
    }

    // ── match ──────────────────────────────────────────────────────────────────

    /// Check a `match`. When `expected` is `Some`, every arm is checked against it;
    /// otherwise arm bodies are synthesized and unified to a common type. Exhaustiveness
    /// and reachability are validated against the scrutinee's type either way.
    fn check_match(
        &mut self,
        env: &Env,
        scrutinee: &Sp<Expr>,
        arms: &[(Sp<Pat>, Sp<Expr>)],
        expected: Option<&Ty>,
    ) -> Result<Ty, TypeError> {
        let scrut_ty = self.synth(env, scrutinee)?;

        // The arms' result type: the expected type, or a fresh metavariable to join into.
        let result = match expected {
            Some(t) => t.clone(),
            None => self.table.fresh(),
        };

        for (pat, body) in arms {
            let mut inner = env.clone();
            self.check_pat(pat, &scrut_ty, &mut inner)?;
            self.check(&inner, body, &result)?;
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

        Ok(self.table.zonk(&result))
    }

    // ── Patterns ─────────────────────────────────────────────────────────────

    /// Bind the variables of an irrefutable `let`/lambda pattern. Refutable literal
    /// patterns are rejected here (they only make sense in a `match`).
    fn bind_pat(&mut self, pat: &Sp<Pat>, ty: &Ty, env: &mut Env) -> Result<(), TypeError> {
        self.check_pat(pat, ty, env)
    }

    /// Type-check a pattern against `ty`, binding its variables into `env`.
    fn check_pat(&mut self, pat: &Sp<Pat>, ty: &Ty, env: &mut Env) -> Result<(), TypeError> {
        let span = pat.1;
        match &pat.0 {
            Pat::Wildcard => Ok(()),
            Pat::Var(name) => {
                env.insert(name.clone(), ty.clone());
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
                        self.check_pat(p, t, env)?;
                    }
                    Ok(())
                }
                Ty::Tuple(ts) => Err(TypeError::ArityMismatch {
                    expected: ts.len(),
                    found: ps.len(),
                    span,
                }),
                Ty::Meta(_) => {
                    let fresh: Vec<Ty> = (0..ps.len()).map(|_| self.table.fresh()).collect();
                    self.table.unify(ty, &Ty::Tuple(fresh.clone()), span)?;
                    for (p, t) in ps.iter().zip(&fresh) {
                        self.check_pat(p, t, env)?;
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
