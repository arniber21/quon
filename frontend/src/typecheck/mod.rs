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

pub(crate) mod builtins;
pub(crate) mod circuit;
mod error;
mod exhaust;
mod linear;
mod monad;
mod unify;

#[cfg(test)]
mod tests;

pub use error::TypeError;

use crate::ast::{BinOp, Decl, Expr, Kind, LitPat, Name, NatExpr, Pat, Stmt, Type};
use crate::lexer::{SimpleSpan, Sp};
use crate::refinement::{Assumption, DepthError, RefinementCtx};
use crate::types::{CodeFamilyTy, Ty};
use builtins::Scheme;
use linear::Delta;
use std::collections::{BTreeSet, HashMap};
use unify::Table;

use crate::analysis::{
    ResolutionMap, ResolvedTarget, SymbolIndex, TypeAnnotations, is_quantum_builtin,
};
use quon_core::DepthExpr;

/// The unrestricted context Γ: names in scope mapped to their (monomorphic) types.
type Env = HashMap<Name, Ty>;

/// A resolved type alias: its kinded parameters and its right-hand side.
#[derive(Clone)]
struct AliasDef {
    params: Vec<(Name, Kind)>,
    body: Sp<Type>,
}

/// A captured self-recursive call site (issue #60): the value substitution it applies to the
/// current function's `Nat` parameters (keyed by parameter name), and the refinement assumptions
/// in force there. Collected while checking a body, then replayed after to verify a well-founded
/// decreasing measure exists — some `Nat` parameter `p` whose argument is provably `< p` (and
/// `≥ 0`) at *every* recursive call.
#[derive(Clone)]
struct RecCall {
    sigma: HashMap<String, DepthExpr>,
    assumptions: Vec<Assumption>,
}

/// A top-level function's value-dependency signature (issue #57). Records each parameter's
/// name and whether it is a `Nat` value parameter (so its call-site argument can be lowered to
/// a [`DepthExpr`] and substituted into the result type), together with the resolved return
/// type the substitution targets. A function with no `Nat` parameter is non-dependent and its
/// call sites take the ordinary application path.
#[derive(Clone)]
struct FnSig {
    /// One entry per declared parameter, in order: `(name, is_nat)`. `is_nat` is read off the
    /// *surface* `Type::Nat` (which resolves to `Ty::Int`, so it cannot be recovered from `ret`).
    params: Vec<(Name, bool)>,
    /// The resolved return type, whose embedded depth/width [`DepthExpr`]s get specialized.
    ret: Ty,
}

/// Kinded polymorphism signature for user functions with `<F: CodeFamily, d: Nat>` (ADR-0014).
#[derive(Clone)]
struct KindedFnSig {
    type_params: Vec<(Name, Kind)>,
    /// Resolved parameter types (with rigid `F` / `d`).
    param_tys: Vec<Ty>,
    ret: Ty,
}

/// The classical-fragment type checker. Holds the global signature environment, the
/// alias table, and the metavariable substitution shared across one checking run.
pub struct TypeChecker {
    /// Top-level function signatures, available to every body (enables mutual recursion).
    globals: Env,
    /// Value-dependency signatures for top-level functions (issue #57): which parameters are
    /// `Nat` and the return type their call-site arguments specialize. Keyed by function name.
    fn_sigs: HashMap<Name, FnSig>,
    /// Kinded type-parameter signatures (`<F: CodeFamily, d: Nat>`) for call-site specialization.
    kinded_fn_sigs: HashMap<Name, KindedFnSig>,
    /// In-scope kinded type parameters (`F: CodeFamily`, `d: Nat`) while resolving types.
    kind_env: HashMap<Name, Kind>,
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
    /// Stack of ambient circuit-register widths, one frame per `circuit { }` block we are
    /// inside. Gate placement (`H @ 0`, `H(q)`) embeds a gate into the innermost register, so
    /// its result width is read from the top of this stack (issue #11, SPEC §5.6).
    circuit_width: Vec<DepthExpr>,
    /// Annotated output width `m` of the innermost `circuit { }` block — caps how far gate
    /// placement may grow the register footprint (width-changing `Circuit<n,m,…>` with `m > n`).
    circuit_width_cap: Vec<DepthExpr>,
    /// The Z3-backed refinement bridge (issue #13). Used only to discharge *symbolic* depth
    /// obligations — verifying an annotation against an inferred depth, and reconciling
    /// `match` branches — when structural equality (`DepthExpr::equiv`) is not enough.
    /// Pure-constant depths never reach it.
    refine: RefinementCtx,
    /// The refinement assumptions in force at the current point — the equalities/disequalities a
    /// `match` arm's pattern implies about its `Nat` scrutinee (issue #59), plus any measure
    /// facts. Pushed on entering a refined arm and popped on exit, so width/depth obligations in
    /// [`expect_type`] discharge under them (e.g. `identity(0)`'s width `0 = n` when `n = 0` is
    /// known) without leaking into sibling arms or the enclosing scope.
    assumptions: Vec<Assumption>,
    /// The function whose body is currently being checked, as `(name, Nat-parameter names)`
    /// (issue #60). A fully-applied call to this name is a self-recursive call: its value
    /// substitution and the assumptions in force are captured into [`rec_calls`] for the
    /// well-founded-measure check run after the body.
    current_fn: Option<(Name, Vec<Name>)>,
    /// Self-recursive call sites captured while checking the current body (issue #60).
    rec_calls: Vec<RecCall>,
    /// Optional LSP annotation sink (issue #45).
    annotations: Option<*mut TypeAnnotations>,
    /// Optional LSP resolution sink (issue #45).
    resolutions: Option<*mut ResolutionMap>,
    /// Symbol index for resolution recording.
    symbol_index: Option<SymbolIndex>,
}

impl TypeChecker {
    pub fn new() -> Self {
        Self {
            globals: Env::new(),
            fn_sigs: HashMap::new(),
            kinded_fn_sigs: HashMap::new(),
            kind_env: HashMap::new(),
            aliases: HashMap::new(),
            table: Table::new(),
            numeric: Vec::new(),
            lambda_linears: Vec::new(),
            circuit_width: Vec::new(),
            circuit_width_cap: Vec::new(),
            refine: RefinementCtx::new(),
            assumptions: Vec::new(),
            current_fn: None,
            rec_calls: Vec::new(),
            annotations: None,
            resolutions: None,
            symbol_index: None,
        }
    }

    /// Enable LSP analysis hooks (issue #45).
    pub fn enable_analysis(&mut self, symbols: &SymbolIndex) {
        self.symbol_index = Some(symbols.clone());
    }

    pub fn set_sinks(
        &mut self,
        annotations: &mut TypeAnnotations,
        resolutions: &mut ResolutionMap,
    ) {
        self.annotations = Some(annotations as *mut TypeAnnotations);
        self.resolutions = Some(resolutions as *mut ResolutionMap);
    }

    fn record_annotation(&mut self, span: SimpleSpan, ty: &Ty) {
        if let Some(ptr) = self.annotations {
            // SAFETY: `analyze_program` owns the annotations for the checker lifetime.
            unsafe {
                (*ptr).record(span, ty.clone());
            }
        }
    }

    fn record_resolution(&mut self, span: SimpleSpan, target: ResolvedTarget) {
        if let Some(ptr) = self.resolutions {
            unsafe {
                (*ptr).record(span, target);
            }
        }
    }

    fn resolve_var_name(&self, name: &str, use_span: SimpleSpan) -> Option<ResolvedTarget> {
        let index = self.symbol_index.as_ref()?;
        let id = index.resolve_name_at(name, use_span.start)?;
        Some(ResolvedTarget::Symbol(id))
    }

    /// Reconcile an inferred type `got` against an expected type `expected`. For two circuits
    /// the *depth* is the symbolic obligation discharged by Z3 (issue #13): widths must match
    /// structurally and the Clifford class must be equal, but the depth is checked with
    /// [`verify_depth`] so a symbolic annotation (`Circuit<…, 2*n+2, …>`) is *verified* against
    /// the inferred depth rather than required to be the identical expression. Everything else
    /// falls through to the ordinary unifier. `expected` carries the user annotation, `got` the
    /// inferred type.
    fn expect_type(&mut self, expected: &Ty, got: &Ty, span: SimpleSpan) -> Result<(), TypeError> {
        if let (
            Ty::Circuit {
                n: en,
                m: em,
                d: ed,
                c: ec,
            },
            Ty::Circuit {
                n: gn,
                m: gm,
                d: gd,
                c: gc,
            },
        ) = (self.table.resolve(expected), self.table.resolve(got))
        {
            // Circuit subtyping (SPEC §3.3): width invariant (`=`), depth covariant (`≤`), class
            // covariant (`⊑`). Class first, so a class disagreement is named specifically (#12).
            //
            // Class is checked by *subsumption*, not equality (issue #58): a `Clifford` value
            // (`gc`) satisfies a `Universal` expectation (`ec`) since `Clifford ⊑ Universal`, but
            // a `Universal` value never satisfies a `Clifford` annotation. Class *inference* (the
            // `join`) is unchanged — this only relaxes the checking direction.
            if !gc.leq(&ec) {
                return Err(TypeError::CliffordMismatch {
                    expected: ec,
                    found: gc,
                    span,
                });
            }
            self.verify_width(&en, &gn, span)?;
            self.verify_width(&em, &gm, span)?;
            return self.verify_depth(&ed, &gd, span);
        }
        self.table.unify(expected, got, span)
    }

    /// Verify an inferred symbolic depth against a user-supplied annotation (issue #13). Depth is
    /// an **upper bound** (SPEC §3.3): the obligation is `inferred ≤ annotated`, discharged under
    /// the active refinement assumptions (issue #59). `prove_le` short-circuits on holes,
    /// structural equivalence, and (assumption-free) constants; only genuinely symbolic
    /// obligations reach Z3. Maps a [`DepthError`] to a span-accurate [`TypeError`].
    fn verify_depth(
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
    fn verify_width(
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
                    name.0.clone(),
                    AliasDef {
                        params: params
                            .iter()
                            .map(|p| (p.name.0.clone(), p.kind_or_nat()))
                            .collect(),
                        body: ty.clone(),
                    },
                );
            }
        }

        // Pass 1.5: kind-check alias bodies under their params (same as fn signatures).
        for (decl, _) in decls {
            if let Decl::TypeAlias { params, ty, .. } = decl {
                self.push_kind_params(params);
                if let Err(e) = self.resolve_type(ty) {
                    errors.push(e);
                }
                self.pop_kind_params(params);
            }
        }

        // Pass 2: function signatures into Γ (so calls resolve regardless of order).
        for (decl, _) in decls {
            if let Decl::Fn {
                name,
                type_params,
                params,
                ret,
                ..
            } = decl
            {
                self.push_kind_params(type_params);
                match self.fn_type(params, ret) {
                    Ok(ty) => {
                        self.globals.insert(name.0.clone(), ty);
                        self.record_fn_sig(name, params, ret);
                        if let Err(e) = self.record_kinded_fn_sig(name, type_params, params, ret) {
                            errors.push(e);
                        }
                    }
                    Err(e) => errors.push(e),
                }
                self.pop_kind_params(type_params);
            }
        }

        // Pass 2.5: reject mutual recursion among functions (issue #60). Direct self-recursion is
        // checked for a decreasing measure per-body; a cycle through ≥ 2 distinct functions has no
        // such per-body witness, so it is rejected up front rather than silently accepted.
        self.check_no_mutual_recursion(decls, &mut errors);

        // Pass 3: check each body.
        for (decl, _) in decls {
            if let Decl::Fn {
                name,
                type_params,
                params,
                ret,
                body,
                ..
            } = decl
            {
                self.push_kind_params(type_params);
                if let Err(e) = self.check_fn_body(name, params, ret, body) {
                    errors.push(e);
                }
                self.pop_kind_params(type_params);
            }
        }

        // Pass 4: mixed QEC + bare-qubit entrypoint (ADR-0014).
        self.check_mixed_qec_entrypoint(decls, &mut errors);

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }

    /// Resolved top-level function type after [`check_decls`] (issue #16 lowering).
    pub fn fn_type_of(&self, name: &str) -> Option<&Ty> {
        self.globals.get(name)
    }

    /// Test/diagnostic hook: synthesize the body type of the *last* function in `decls`,
    /// ignoring its declared return type. Lets tests assert the exact inferred type.
    pub fn synth_last_body(&mut self, decls: &[Sp<Decl>]) -> Result<Ty, TypeError> {
        for (decl, _) in decls {
            if let Decl::TypeAlias { name, params, ty } = decl {
                self.aliases.insert(
                    name.0.clone(),
                    AliasDef {
                        params: params
                            .iter()
                            .map(|p| (p.name.0.clone(), p.kind_or_nat()))
                            .collect(),
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
                self.globals.insert(name.0.clone(), ty);
                self.record_fn_sig(name, params, ret);
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
    fn fn_type(
        &mut self,
        params: &[(Sp<Name>, Sp<Type>)],
        ret: &Sp<Type>,
    ) -> Result<Ty, TypeError> {
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

    /// Record a function's value-dependency signature (issue #57) into `fn_sigs`. `is_nat` is
    /// read from the *surface* `Type::Nat` (it resolves to `Ty::Int`, so the distinction is lost
    /// in `ret`). Only called once the signature is known to resolve, so it never double-reports.
    fn record_fn_sig(&mut self, name: &Sp<Name>, params: &[(Sp<Name>, Sp<Type>)], ret: &Sp<Type>) {
        if let Ok(ret_ty) = self.resolve_type(ret) {
            let sig_params = params
                .iter()
                .map(|(pn, pt)| (pn.0.clone(), matches!(pt.0, Type::Nat)))
                .collect();
            self.fn_sigs.insert(
                name.0.clone(),
                FnSig {
                    params: sig_params,
                    ret: ret_ty,
                },
            );
        }
    }

    /// Bind a function's parameters, routing each by linearity: a linear-resource parameter
    /// goes into `Δ` (and is returned in the introduced-names list so the body's scope exit
    /// can demand it was consumed), an unrestricted one goes into `Γ`. The binding span is
    /// the parameter's type annotation (parameter names carry no span of their own).
    fn bind_fn_params(
        &mut self,
        params: &[(Sp<Name>, Sp<Type>)],
        env: &mut Env,
        delta: &mut Delta,
    ) -> Result<Vec<(String, SimpleSpan)>, TypeError> {
        let mut introduced = Vec::new();
        for (name, t) in params {
            let ty = self.resolve_type(t)?;
            if ty.is_linear_resource() {
                delta.introduce(name.0.clone(), ty, name.1)?;
                introduced.push((name.0.clone(), name.1));
            } else {
                env.insert(name.0.clone(), ty);
            }
        }
        Ok(introduced)
    }

    fn check_fn_body(
        &mut self,
        name: &Sp<Name>,
        params: &[(Sp<Name>, Sp<Type>)],
        ret: &Sp<Type>,
        body: &Sp<Expr>,
    ) -> Result<(), TypeError> {
        // Track the current function and its `Nat` parameters so self-recursive calls in the body
        // can be captured for the well-founded-measure check (issue #60).
        let nat_params: Vec<Name> = params
            .iter()
            .filter(|(_, t)| matches!(t.0, Type::Nat))
            .map(|(n, _)| n.0.clone())
            .collect();
        self.current_fn = Some((name.0.clone(), nat_params));
        self.rec_calls.clear();

        let mut env = self.globals.clone();
        let mut delta = Delta::new();
        let introduced = self.bind_fn_params(params, &mut env, &mut delta)?;
        let ret_ty = self.resolve_type(ret)?;
        self.check(&env, &mut delta, body, &ret_ty)?;
        self.finalize_numeric()?;
        self.ensure_consumed(&delta, &introduced)?;
        self.check_termination(&name.0, body.1)
    }

    /// Verify that the recursive calls captured while checking the current body admit a
    /// well-founded decreasing measure (issue #60, SPEC §3.3 "Recursive circuit functions"): a
    /// `Nat` parameter `p` whose call-site argument is provably `< p` *and* `≥ 0` at every
    /// recursive call, under that call's refinement assumptions. The `≥ 0` half is what forces a
    /// base case — without the `n ≥ 1` a `match { 0 => … }` arm supplies, `n − 1 ≥ 0` is not
    /// provable at `n = 0`, so the unguarded `f(n) = f(n−1)` is correctly rejected. A function
    /// with no captured recursive call is non-recursive here and trivially passes.
    fn check_termination(&self, name: &Name, span: SimpleSpan) -> Result<(), TypeError> {
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

    /// Reject mutual recursion among *circuit* functions (issue #60). Builds the static call graph
    /// (application-head references between functions) and flags any circuit-returning function on
    /// a cycle through at least one *other* circuit function — i.e. some circuit function `v ≠ self`
    /// reachable from `self` that can reach `self` back. Pure self-loops are direct recursion and
    /// handled by [`check_termination`]; classical mutual recursion (e.g. `even`/`odd` over `Bool`)
    /// builds no circuit and is unaffected. Only genuine circuit cycles are reported here.
    fn check_no_mutual_recursion(&self, decls: &[Sp<Decl>], errors: &mut Vec<TypeError>) {
        use std::collections::HashSet;
        let fn_names: HashSet<&str> = decls
            .iter()
            .filter_map(|(d, _)| match d {
                Decl::Fn { name, .. } => Some(name.0.as_str()),
                _ => None,
            })
            .collect();
        // Whether a function's declared result is a circuit (the kind whose recursion must
        // terminate to bound depth). Reads the resolved return type recorded in `fn_sigs`.
        let returns_circuit = |name: &str| -> bool {
            self.fn_sigs
                .get(name)
                .map(|sig| matches!(sig.ret, Ty::Circuit { .. }))
                .unwrap_or(false)
        };
        let mut adj: HashMap<&str, HashSet<String>> = HashMap::new();
        for (d, _) in decls {
            if let Decl::Fn { name, body, .. } = d {
                let mut callees = HashSet::new();
                collect_called_fns(body, &fn_names, &mut callees);
                adj.insert(name.0.as_str(), callees);
            }
        }
        // The set of functions reachable from `start` over ≥ 1 call edge.
        let reaches = |start: &str| -> HashSet<String> {
            let mut seen = HashSet::new();
            let mut stack: Vec<String> = adj.get(start).into_iter().flatten().cloned().collect();
            while let Some(n) = stack.pop() {
                if seen.insert(n.clone())
                    && let Some(next) = adj.get(n.as_str())
                {
                    for m in next {
                        if !seen.contains(m) {
                            stack.push(m.clone());
                        }
                    }
                }
            }
            seen
        };
        for (d, span) in decls {
            if let Decl::Fn { name, .. } = d
                && returns_circuit(&name.0)
            {
                let forward = reaches(&name.0);
                let mutual = forward.iter().any(|v| {
                    v.as_str() != name.0 && returns_circuit(v) && reaches(v).contains(&name.0)
                });
                if mutual {
                    errors.push(TypeError::MutualRecursion {
                        name: name.0.clone(),
                        span: *span,
                    });
                }
            }
        }
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

            Expr::TypeApp { callee, args } => self.synth_type_app(env, delta, callee, args, span),

            Expr::App(f, x) => self.synth_app(env, delta, f, x, span),

            // ── Circuit fragment (issue #11) ────────────────────────────────────
            // `c @ x` is gate *placement* inside a `circuit { }` block (an ambient register is
            // in scope), and circuit *application* to a register otherwise (issue #14).
            Expr::GateApp { gate, qubits } => {
                let g = self.synth(env, delta, gate)?;
                if self.circuit_width.is_empty() {
                    self.apply_circuit(env, delta, g, qubits)
                } else {
                    self.place_gate(env, delta, g, qubits)
                }
            }
            Expr::Compose(l, r) => self.synth_compose(env, delta, l, r, span),
            Expr::Par(body, count) => self.synth_par(env, delta, body, count),
            Expr::Adjoint(c) => self.synth_adjoint(env, delta, c),
            Expr::Controlled(c) => self.synth_controlled(env, delta, c),
            Expr::For { pat, iter, body } => self.synth_for(env, delta, pat, iter, body, span),
            Expr::CircuitBlock(stmts) => {
                // Free synthesis for circuit *values* used outside a declared
                // annotation — e.g. `controlled(circuit { H |> T })` (issue #182).
                // Bare gate composition does not need an ambient register; gate
                // placement (`@`) grows a zero-width ambient as needed.
                self.circuit_width.push(DepthExpr::Nat(0));
                self.circuit_width_cap.push(DepthExpr::Nat(u64::MAX / 4));
                let result = self.synth_block_body(env, delta, stmts, span);
                self.circuit_width.pop();
                self.circuit_width_cap.pop();
                result
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
                let introduced =
                    self.bind_pat_with_rhs(pat, Some(rhs), &rhs_ty, &mut inner, delta)?;
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

            // ── Quantum monad (issue #14, SPEC §3.5) ────────────────────────────
            Expr::Return(v) => self.synth_return(env, delta, v),
            Expr::Bind { rhs, param, body } => self.synth_bind(env, delta, rhs, param, body),

            // ── Borrow blocks (issue #15, SPEC §3.4) ────────────────────────────
            Expr::Borrow { bindings, body } => self.synth_borrow(env, delta, bindings, body, span),

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
            if let Some(target) = self.resolve_var_name(name, span) {
                self.record_resolution(span, target);
            }
            self.record_annotation(span, ty);
            return Ok(ty.clone());
        }
        // A linear resource: consuming it removes it from `Δ`; a second use is no-cloning.
        if let Some(result) = delta.try_consume(name, span) {
            if let Ok(ref ty) = result {
                if let Some(target) = self.resolve_var_name(name, span) {
                    self.record_resolution(span, target);
                }
                self.record_annotation(span, ty);
            }
            return result;
        }
        if let Some(scheme) = builtins::lookup(name) {
            // QEC ops special-cased in `synth_app` are not first-class; reject let-binding them
            // with a clear diagnostic rather than a later opaque mismatch on the lying scheme.
            if matches!(
                name,
                "memory_round" | "measure_logical_z" | "measure_logical_x" | "logical_cx"
            ) {
                return Err(TypeError::Unsupported {
                    construct: "let-bound QEC builtin",
                    span,
                });
            }
            let ty = self.instantiate(&scheme);
            self.record_resolution(span, ResolvedTarget::Builtin(name.to_string()));
            self.record_annotation(span, &ty);
            return Ok(ty);
        }
        // A gate primitive synthesizes a fresh circuit value (or, for rotations, the function
        // that produces one). Issue #12 specialises rotation classes from the static angle.
        if let Some(ty) = circuit::gate_type(name) {
            self.record_resolution(span, ResolvedTarget::Gate(name.to_string()));
            self.record_annotation(span, &ty);
            return Ok(ty);
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
            self.record_resolution(span, ResolvedTarget::QuantumBuiltin(name.to_string()));
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
            Ty::QReg(n) => {
                // Flattening needs a statically known size: a symbolic `QReg<n>` cannot be
                // split into a fixed-arity tuple (that would require `split`, issue #14).
                let size = n.as_const().ok_or(TypeError::Unsupported {
                    construct: "destructure of a symbolic-size register",
                    span,
                })?;
                Ok(Ty::Tuple(vec![Ty::Qubit; size as usize]))
            }
            other => Err(TypeError::Mismatch {
                expected: Ty::QReg(DepthExpr::Var("n".into())),
                found: other,
                span,
            }),
        }
    }

    /// Tensor introduction via `tensored` (SPEC §5): `a `tensored` b` concatenates two
    /// registers into `QReg<n+m>`. Each operand is a register (`QReg<k>`) or a single qubit
    /// (treated as `QReg<1>`), consumed exactly once; `Δ` threads left-to-right so reusing an
    /// operand surfaces as [`TypeError::LinearUsedTwice`].
    fn synth_tensored(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        a: &Sp<Expr>,
        b: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let wa = self.tensor_width(env, delta, a)?;
        let wb = self.tensor_width(env, delta, b)?;
        Ok(Ty::QReg(wa.seq(wb).normalize()))
    }

    /// Register split (SPEC §5): `split(k, q)` consumes `QReg<n>` and yields
    /// `(QReg<k>, QReg<n-k>)`. The remainder may be discarded with `_` (SPEC §3.4).
    fn synth_split(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        k: &Sp<Expr>,
        q: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let k_w = self.expr_to_depth(env, delta, k)?;
        let q_ty = self.synth(env, delta, q)?;
        let n_w = match self.table.resolve(&q_ty) {
            Ty::QReg(w) => w,
            other => {
                return Err(TypeError::Mismatch {
                    expected: Ty::QReg(DepthExpr::Var("n".into())),
                    found: other,
                    span: q.1,
                });
            }
        };
        Ok(Ty::Tuple(vec![
            Ty::QReg(k_w.clone()),
            Ty::QReg(n_w.minus(k_w).normalize()),
        ]))
    }

    /// Matrix/list indexing: `index(base, i)` (parser desugars `base[i]`).
    fn synth_index(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        base: &Sp<Expr>,
        idx: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        self.check(env, delta, idx, &Ty::Int)?;
        let base_ty = self.synth(env, delta, base)?;
        match self.table.resolve(&base_ty) {
            Ty::Matrix(_, _, elem) => Ok(Ty::list(*elem)),
            Ty::List(elem) => Ok(*elem),
            other => Err(TypeError::Mismatch {
                expected: Ty::Matrix(
                    DepthExpr::Var("n".into()),
                    DepthExpr::Var("m".into()),
                    Box::new(self.table.fresh()),
                ),
                found: other,
                span: base.1,
            }),
        }
    }

    /// The register width of a `tensored` operand: a `QReg<n>` yields `n`, a bare `Qubit`
    /// yields `1`. Any other type is a mismatch.
    fn tensor_width(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        e: &Sp<Expr>,
    ) -> Result<DepthExpr, TypeError> {
        let t = self.synth(env, delta, e)?;
        match self.table.resolve(&t) {
            Ty::QReg(w) => Ok(w),
            Ty::Qubit => Ok(DepthExpr::Nat(1)),
            other => Err(TypeError::Mismatch {
                expected: Ty::QReg(DepthExpr::Var("n".into())),
                found: other,
                span: e.1,
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
            return Ok(Ty::QReg(DepthExpr::Nat(tys.len() as u64)));
        }
        Ok(Ty::Tuple(tys))
    }

    // ── Circuit fragment (issue #11, SPEC §3.3, §5.6–§5.8) ──────────────────────

    /// Synthesize an application. Three cases share the `App` node: circuit *combinators*
    /// recognised by their call spine (`identity`, `repeat`, `on_high`/`on_low`, and
    /// `fold` over a circuit accumulator); gate *placement* when the callee is a circuit
    /// value (`H(q)`); and ordinary function application otherwise.
    fn synth_app(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        f: &Sp<Expr>,
        x: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let (head, args) = flatten_app(f, x);

        // QEC constructors: `repetition_code<3>()` / `surface_code<3>()` / `surface_code_x<3>()`.
        if let Expr::TypeApp {
            callee,
            args: targs,
        } = &head.0
            && let Expr::Var(name) = &callee.0
            && matches!(
                name.as_str(),
                "repetition_code" | "surface_code" | "surface_code_x"
            )
        {
            let ctor_ty = self.synth_qec_ctor_type(name, targs, callee.1)?;
            // Expect a nullary call (`()` → Unit argument).
            if args.len() != 1 {
                return Err(TypeError::ArityMismatch {
                    expected: 1,
                    found: args.len(),
                    span,
                });
            }
            self.check(env, delta, args[0], &Ty::Unit)?;
            let (_, cod) = self.as_function(&ctor_ty, span)?;
            return Ok(cod);
        }

        if let Expr::Var(name) = &head.0 {
            match (name.as_str(), args.len()) {
                ("repetition_code" | "surface_code" | "surface_code_x", _) => {
                    return Err(TypeError::QecCtorRequiresDistance {
                        name: match name.as_str() {
                            "surface_code" => "surface_code",
                            "surface_code_x" => "surface_code_x",
                            _ => "repetition_code",
                        },
                        span: head.1,
                    });
                }
                ("identity", 1) => return self.synth_identity(env, delta, args[0]),
                ("repeat", 2) => return self.synth_repeat(env, delta, args[0], args[1]),
                ("on_high" | "on_low", 2) => {
                    return self.synth_on_sub(env, delta, args[0], args[1]);
                }
                ("swap_reverse", 1) => return self.synth_swap_reverse(env, delta, args[0]),
                ("tensored", 2) => return self.synth_tensored(env, delta, args[0], args[1]),
                ("split", 2) => return self.synth_split(env, delta, args[0], args[1]),
                ("measure_all", 1) => return self.synth_measure_all(env, delta, args[0]),
                ("map_q", 2) => return self.synth_map_q(env, delta, args[0], args[1]),
                ("sequence_q", 1) => return self.synth_sequence_q(env, delta, args[0]),
                ("index", 2) => return self.synth_index(env, delta, args[0], args[1]),
                ("fold", 3) => return self.synth_fold(env, delta, args[0], args[1], args[2], span),
                (rot, 1) if circuit::is_specialisable_rotation(rot) => {
                    return self.synth_rotation(env, delta, args[0]);
                }
                ("memory_round", 1) => return self.synth_memory_round(env, delta, args[0], span),
                ("measure_logical_z" | "measure_logical_x", 1) => {
                    return self.synth_measure_logical(env, delta, args[0], span);
                }
                ("logical_cx", 2) => {
                    return self.synth_logical_cx(env, delta, args[0], args[1], span);
                }
                _ => {}
            }
        }
        // A `qreg(n)` allocation: size-dependent, so its result register width is read from the
        // argument value (like `identity`). Yields a fresh register in the quantum monad.
        if let Expr::Var(name) = &head.0
            && name == "qreg"
            && args.len() == 1
        {
            let size = self.expr_to_depth(env, delta, args[0])?;
            return Ok(Ty::Q(Box::new(Ty::QReg(size))));
        }
        // Kinded user functions: specialize `F`/`d` from QecBlock arguments.
        if let Expr::Var(name) = &head.0
            && let Some(ksig) = self.kinded_fn_sigs.get(name).cloned()
            && args.len() == ksig.param_tys.len()
            && !ksig.type_params.is_empty()
        {
            return self.synth_kinded_app(env, delta, &ksig, &args);
        }
        // Value-dependent application (issue #57): a *fully-applied* call to a top-level function
        // with at least one `Nat` value parameter specializes that parameter to the call-site
        // argument in the result type. e.g. `qft(n-1)` of `qft(n): Circuit<n,n,n*n,…>` yields
        // `Circuit<n-1, n-1, (n-1)*(n-1), …>`. The substitution is computed *before* the ordinary
        // application machinery checks the arguments and threads the linear context, then applied
        // to the un-specialized result. Under-applied calls fall through to the default path.
        if let Expr::Var(name) = &head.0
            && let Some(sig) = self.fn_sigs.get(name).cloned()
            && args.len() == sig.params.len()
            && sig.params.iter().any(|(_, is_nat)| *is_nat)
        {
            let mut sigma: HashMap<String, DepthExpr> = HashMap::new();
            for ((pname, is_nat), arg) in sig.params.iter().zip(args.iter()) {
                if *is_nat {
                    let d = self.depth_of(arg).map_err(|_| TypeError::NonDependentArg {
                        func: name.clone(),
                        param: pname.clone(),
                        span: arg.1,
                    })?;
                    sigma.insert(pname.clone(), d);
                }
            }
            // A fully-applied call to the function we are currently checking is a self-recursive
            // call (issue #60): record its value substitution and the assumptions in force so the
            // post-body measure check can verify a well-founded decrease. The signature stands in
            // as the inductive hypothesis — the body is never inlined, so this cannot loop.
            if let Some((cur, _)) = &self.current_fn
                && cur == name
            {
                self.rec_calls.push(RecCall {
                    sigma: sigma.clone(),
                    assumptions: self.assumptions.clone(),
                });
            }
            // Run the ordinary application to check arguments and consume the linear context.
            let f_ty = self.synth(env, delta, f)?;
            let (dom, cod) = self.as_function(&f_ty, f.1)?;
            self.check(env, delta, x, &dom)?;
            return Ok(Self::subst_depth_in_ty(&self.table.zonk(&cod), &sigma));
        }
        // Default: a circuit callee is gate placement inside a `circuit { }` block, circuit
        // application to a register otherwise; anything else is ordinary application.
        let f_ty = self.synth(env, delta, f)?;
        if matches!(self.table.resolve(&f_ty), Ty::Circuit { .. }) {
            return if self.circuit_width.is_empty() {
                self.apply_circuit(env, delta, f_ty, x)
            } else {
                self.place_gate(env, delta, f_ty, x)
            };
        }
        let (dom, cod) = self.as_function(&f_ty, f.1)?;
        self.check(env, delta, x, &dom)?;
        Ok(cod)
    }

    /// Convert a classical `Int`-typed index/count expression into a symbolic depth value,
    /// type-checking it as `Int` first. A runtime variable promotes to a `DepthExpr::Var`
    /// (SPEC §3.2: "runtime Int promoted to symbolic Nat"); only `+`/`*` survive structurally.
    fn expr_to_depth(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        e: &Sp<Expr>,
    ) -> Result<DepthExpr, TypeError> {
        self.check(env, delta, e, &Ty::Int)?;
        self.depth_of(e)
    }

    /// Specialize every depth/width [`DepthExpr`] embedded in `ty` by the value substitution
    /// `σ` (issue #57). Reaches every nested position — register/circuit/matrix indices — and
    /// recurses through `Q`, `List`, `Tuple`, and both function arrows. The Clifford class and
    /// the leaf scalar types carry no depth, so they pass through unchanged.
    fn subst_depth_in_ty(ty: &Ty, sigma: &HashMap<String, DepthExpr>) -> Ty {
        match ty {
            Ty::QReg(n) => Ty::QReg(n.subst(sigma)),
            Ty::Circuit { n, m, d, c } => Ty::Circuit {
                n: n.subst(sigma),
                m: m.subst(sigma),
                d: d.subst(sigma),
                c: c.clone(),
            },
            Ty::Matrix(r, c, t) => Ty::Matrix(
                r.subst(sigma),
                c.subst(sigma),
                Box::new(Self::subst_depth_in_ty(t, sigma)),
            ),
            Ty::Q(t) => Ty::Q(Box::new(Self::subst_depth_in_ty(t, sigma))),
            Ty::List(t) => Ty::List(Box::new(Self::subst_depth_in_ty(t, sigma))),
            Ty::Tuple(ts) => Ty::Tuple(
                ts.iter()
                    .map(|t| Self::subst_depth_in_ty(t, sigma))
                    .collect(),
            ),
            Ty::Fn(a, b) => Ty::Fn(
                Box::new(Self::subst_depth_in_ty(a, sigma)),
                Box::new(Self::subst_depth_in_ty(b, sigma)),
            ),
            Ty::Linear(a, b) => Ty::Linear(
                Box::new(Self::subst_depth_in_ty(a, sigma)),
                Box::new(Self::subst_depth_in_ty(b, sigma)),
            ),
            Ty::QecBlock { family, distance } => Ty::QecBlock {
                family: family.clone(),
                distance: distance.subst(sigma),
            },
            Ty::Qubit
            | Ty::Bit
            | Ty::Bool
            | Ty::Int
            | Ty::Float
            | Ty::Unit
            | Ty::Var(_)
            | Ty::Meta(_) => ty.clone(),
        }
    }

    /// The structural depth form of an `Int` expression, without re-type-checking it.
    fn depth_of(&self, e: &Sp<Expr>) -> Result<DepthExpr, TypeError> {
        match &e.0 {
            Expr::Int(k) if *k >= 0 => Ok(DepthExpr::Nat(*k as u64)),
            Expr::Var(name) => Ok(DepthExpr::Var(name.clone())),
            Expr::BinOp {
                op: BinOp::Add,
                lhs,
                rhs,
            } => Ok(self.depth_of(lhs)?.seq(self.depth_of(rhs)?)),
            Expr::BinOp {
                op: BinOp::Mul,
                lhs,
                rhs,
            } => Ok(DepthExpr::repeat(self.depth_of(lhs)?, self.depth_of(rhs)?)),
            Expr::BinOp {
                op: BinOp::Sub,
                lhs,
                rhs,
            } => Ok(self.depth_of(lhs)?.minus(self.depth_of(rhs)?)),
            Expr::BinOp {
                op: BinOp::Div,
                lhs,
                rhs,
            } => Ok(self.depth_of(lhs)?.quot(self.depth_of(rhs)?)),
            Expr::BinOp {
                op: BinOp::Pow,
                lhs,
                rhs,
            } => Ok(self.depth_of(lhs)?.power(self.depth_of(rhs)?)),
            _ => Err(TypeError::Unsupported {
                construct: "non-linear index/depth expression",
                span: e.1,
            }),
        }
    }

    fn synth_arith(
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

            // A `circuit { }` block checks against its declared `Circuit<n,m,d,C>` type: the
            // block's input width is `n`, and the body's inferred indices must match.
            Expr::CircuitBlock(stmts) => {
                self.check_circuit_block(env, delta, stmts, expected, span)
            }

            Expr::Let { pat, rhs, body } => {
                let rhs_ty = self.synth(env, delta, rhs)?;
                let mut inner = env.clone();
                let introduced =
                    self.bind_pat_with_rhs(pat, Some(rhs), &rhs_ty, &mut inner, delta)?;
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
                Ty::QReg(n) if n.as_const() == Some(es.len() as u64) => {
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
        self.expect_type(expected, &got, expr.1)
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
    fn bind_pat(
        &mut self,
        pat: &Sp<Pat>,
        ty: &Ty,
        env: &mut Env,
        delta: &mut Delta,
    ) -> Result<Vec<(String, SimpleSpan)>, TypeError> {
        self.bind_pat_with_rhs(pat, None, ty, env, delta)
    }

    fn bind_pat_with_rhs(
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
    fn check_pat_into(
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
    pub(crate) fn resolve_type(&mut self, ty: &Sp<Type>) -> Result<Ty, TypeError> {
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
            Type::QReg(n) => Ty::QReg(self.nat_to_depth(n)?),
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
                self.nat_to_depth(n)?,
                self.nat_to_depth(m)?,
                Box::new(self.resolve_type_at(t, visiting)?),
            ),
            Type::Circuit { n, m, d, c } => Ty::Circuit {
                n: self.nat_to_depth(n)?,
                m: self.nat_to_depth(m)?,
                d: self.nat_to_depth(d)?,
                c: c.clone(),
            },
            Type::QecBlock { family, distance } => Ty::QecBlock {
                family: self.resolve_code_family(family)?,
                distance: self.nat_to_depth(distance)?,
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
        if let Some(index) = &self.symbol_index {
            for sym in &index.symbols {
                if sym.kind == crate::analysis::SymbolKind::TypeAlias && sym.name == name {
                    self.record_resolution(span, ResolvedTarget::TypeAlias(sym.id));
                    break;
                }
            }
        }
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
            .map(|(p, _)| p.as_str())
            .zip(args.iter().map(|a| &a.0))
            .collect();
        // Validate kinded args before expanding (tags or rigid vars only).
        for ((_, kind), arg) in def.params.iter().zip(args.iter()) {
            match kind {
                Kind::CodeFamily => {
                    self.nat_as_code_family(&arg.0, arg.1)?;
                }
                Kind::Nat => {
                    self.ensure_nat_kind(&arg.0, arg.1)?;
                }
            }
        }
        let expanded = subst_nat_in_type(&def.body, &subst);
        visiting.push(name.to_string());
        let resolved = self.resolve_type_at(&expanded, visiting);
        visiting.pop();
        resolved
    }

    /// Resolve a code-family type expression (`Repetition`, `Surface`, or in-scope `F: CodeFamily`).
    fn resolve_code_family(&self, ty: &Sp<Type>) -> Result<CodeFamilyTy, TypeError> {
        match &ty.0 {
            Type::Var(name) => self.code_family_of_name(name, ty.1),
            _ => Err(TypeError::KindMismatch {
                expected: "CodeFamily",
                found: "non-family type",
                span: ty.1,
            }),
        }
    }

    /// Interpret a Nat-expr alias argument as a code family (tags are bare names).
    fn nat_as_code_family(&self, n: &NatExpr, span: SimpleSpan) -> Result<CodeFamilyTy, TypeError> {
        match n {
            NatExpr::Var(name) => self.code_family_of_name(name, span),
            _ => Err(TypeError::KindMismatch {
                expected: "CodeFamily",
                found: "Nat expression",
                span,
            }),
        }
    }

    fn code_family_of_name(&self, name: &str, span: SimpleSpan) -> Result<CodeFamilyTy, TypeError> {
        match name {
            "Repetition" => Ok(CodeFamilyTy::Repetition),
            "Surface" => Ok(CodeFamilyTy::Surface),
            other => match self.kind_env.get(other) {
                Some(Kind::CodeFamily) => Ok(CodeFamilyTy::Var(other.to_string())),
                Some(Kind::Nat) => Err(TypeError::KindMismatch {
                    expected: "CodeFamily",
                    found: "Nat",
                    span,
                }),
                None => Err(TypeError::UnknownCodeFamily {
                    name: other.to_string(),
                    span,
                }),
            },
        }
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
    /// Rejects `CodeFamily` tags and `F: CodeFamily` params in Nat position.
    fn nat_to_depth(&self, n: &Sp<NatExpr>) -> Result<DepthExpr, TypeError> {
        self.ensure_nat_kind(&n.0, n.1)?;
        nat_to_depth(&n.0).ok_or(TypeError::Unsupported {
            construct: "depth expression",
            span: n.1,
        })
    }

    /// Walk a Nat expression and ensure every variable has kind `Nat` (not `CodeFamily`).
    fn ensure_nat_kind(&self, n: &NatExpr, span: SimpleSpan) -> Result<(), TypeError> {
        match n {
            NatExpr::Lit(_) | NatExpr::Hole => Ok(()),
            NatExpr::Var(name) => self.require_nat_kind(name, span),
            NatExpr::Add(a, b)
            | NatExpr::Sub(a, b)
            | NatExpr::Mul(a, b)
            | NatExpr::Div(a, b)
            | NatExpr::Exp(a, b) => {
                self.ensure_nat_kind(&a.0, a.1)?;
                self.ensure_nat_kind(&b.0, b.1)
            }
        }
    }

    fn require_nat_kind(&self, name: &str, span: SimpleSpan) -> Result<(), TypeError> {
        match name {
            "Repetition" | "Surface" => Err(TypeError::KindMismatch {
                expected: "Nat",
                found: "CodeFamily",
                span,
            }),
            other => match self.kind_env.get(other) {
                Some(Kind::CodeFamily) => Err(TypeError::KindMismatch {
                    expected: "Nat",
                    found: "CodeFamily",
                    span,
                }),
                Some(Kind::Nat) | None => Ok(()),
            },
        }
    }

    // ── QEC helpers (issue #247) ───────────────────────────────────────────────

    fn push_kind_params(&mut self, type_params: &[crate::ast::TypeParam]) {
        for p in type_params {
            self.kind_env.insert(p.name.0.clone(), p.kind_or_nat());
        }
    }

    fn pop_kind_params(&mut self, type_params: &[crate::ast::TypeParam]) {
        for p in type_params {
            self.kind_env.remove(&p.name.0);
        }
    }

    fn record_kinded_fn_sig(
        &mut self,
        name: &Sp<Name>,
        type_params: &[crate::ast::TypeParam],
        params: &[(Sp<Name>, Sp<Type>)],
        ret: &Sp<Type>,
    ) -> Result<(), TypeError> {
        if type_params.is_empty() {
            return Ok(());
        }
        let tps: Vec<(Name, Kind)> = type_params
            .iter()
            .map(|p| (p.name.0.clone(), p.kind_or_nat()))
            .collect();
        let param_tys = params
            .iter()
            .map(|(_, t)| self.resolve_type(t))
            .collect::<Result<Vec<_>, _>>()?;
        let ret_ty = self.resolve_type(ret)?;
        self.kinded_fn_sigs.insert(
            name.0.clone(),
            KindedFnSig {
                type_params: tps,
                param_tys,
                ret: ret_ty,
            },
        );
        Ok(())
    }

    fn synth_type_app(
        &mut self,
        _env: &Env,
        _delta: &mut Delta,
        callee: &Sp<Expr>,
        args: &[Sp<NatExpr>],
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let Expr::Var(name) = &callee.0 else {
            return Err(TypeError::Unsupported {
                construct: "type application on non-variable",
                span,
            });
        };
        match name.as_str() {
            "repetition_code" | "surface_code" | "surface_code_x" => {
                self.synth_qec_ctor_type(name, args, span)
            }
            _ => Err(TypeError::Unsupported {
                construct: "type application",
                span,
            }),
        }
    }

    fn synth_qec_ctor_type(
        &self,
        name: &str,
        targs: &[Sp<NatExpr>],
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        if targs.len() != 1 {
            return Err(TypeError::ArityMismatch {
                expected: 1,
                found: targs.len(),
                span,
            });
        }
        let distance = self.nat_to_depth(&targs[0])?;
        let Some(d) = distance.as_const() else {
            return Err(TypeError::NonLiteralQecDistance { span: targs[0].1 });
        };
        let (family, family_name) = match name {
            "repetition_code" => (CodeFamilyTy::Repetition, "repetition"),
            "surface_code" | "surface_code_x" => (CodeFamilyTy::Surface, "surface"),
            _ => unreachable!("caller filters ctor names"),
        };
        validate_qec_distance(family_name, d, targs[0].1)?;
        Ok(Ty::func(
            Ty::Unit,
            Ty::Q(Box::new(Ty::QecBlock {
                family,
                distance: DepthExpr::Nat(d),
            })),
        ))
    }

    fn synth_memory_round(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        arg: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let ty = self.synth(env, delta, arg)?;
        match self.table.resolve(&ty) {
            Ty::QecBlock { family, distance } => {
                Ok(Ty::Q(Box::new(Ty::QecBlock { family, distance })))
            }
            other => Err(TypeError::Mismatch {
                expected: Ty::QecBlock {
                    family: CodeFamilyTy::Var("F".into()),
                    distance: DepthExpr::Var("d".into()),
                },
                found: other,
                span,
            }),
        }
    }

    fn synth_measure_logical(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        arg: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let ty = self.synth(env, delta, arg)?;
        match self.table.resolve(&ty) {
            Ty::QecBlock { .. } => Ok(Ty::Q(Box::new(Ty::Bit))),
            other => Err(TypeError::Mismatch {
                expected: Ty::QecBlock {
                    family: CodeFamilyTy::Var("F".into()),
                    distance: DepthExpr::Var("d".into()),
                },
                found: other,
                span,
            }),
        }
    }

    fn synth_logical_cx(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        a: &Sp<Expr>,
        b: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let ta = self.synth(env, delta, a)?;
        let tb = self.synth(env, delta, b)?;
        let (fa, da) = match self.table.resolve(&ta) {
            Ty::QecBlock { family, distance } => (family, distance),
            other => {
                return Err(TypeError::Mismatch {
                    expected: Ty::QecBlock {
                        family: CodeFamilyTy::Surface,
                        distance: DepthExpr::Var("d".into()),
                    },
                    found: other,
                    span: a.1,
                });
            }
        };
        let (fb, db) = match self.table.resolve(&tb) {
            Ty::QecBlock { family, distance } => (family, distance),
            other => {
                return Err(TypeError::Mismatch {
                    expected: Ty::QecBlock {
                        family: CodeFamilyTy::Surface,
                        distance: DepthExpr::Var("d".into()),
                    },
                    found: other,
                    span: b.1,
                });
            }
        };
        if fa != CodeFamilyTy::Surface || fb != CodeFamilyTy::Surface {
            let bad = if fa != CodeFamilyTy::Surface { fa } else { fb };
            return Err(TypeError::LogicalCxRequiresSurface { family: bad, span });
        }
        if !da.equiv(&db) {
            return Err(TypeError::LogicalCxDistanceMismatch {
                expected: da,
                found: db,
                span,
            });
        }
        let block = Ty::QecBlock {
            family: CodeFamilyTy::Surface,
            distance: da,
        };
        Ok(Ty::Q(Box::new(Ty::Tuple(vec![block.clone(), block]))))
    }

    fn synth_kinded_app(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        ksig: &KindedFnSig,
        args: &[&Sp<Expr>],
    ) -> Result<Ty, TypeError> {
        let mut fam_subst: HashMap<String, CodeFamilyTy> = HashMap::new();
        let mut depth_subst: HashMap<String, DepthExpr> = HashMap::new();
        // Check arguments left-to-right, collecting F/d bindings from QecBlock params.
        for (expected, arg) in ksig.param_tys.iter().zip(args.iter()) {
            let got = self.synth(env, delta, arg)?;
            collect_kinded_bindings(
                expected,
                &self.table.resolve(&got),
                &mut fam_subst,
                &mut depth_subst,
                arg.1,
            )?;
            let specialized = subst_kinded_ty(expected, &fam_subst, &depth_subst);
            self.table.unify(&specialized, &got, arg.1)?;
        }
        // Consume via ordinary application on the (possibly multi-arg) spine for linearity of
        // remaining structure — arguments already synthesized above; re-check would double-consume
        // linear resources. Specialize the declared return type instead.
        Ok(subst_kinded_ty(&ksig.ret, &fam_subst, &depth_subst))
    }

    fn check_mixed_qec_entrypoint(&self, decls: &[Sp<Decl>], errors: &mut Vec<TypeError>) {
        use std::collections::HashSet;
        let fn_names: HashSet<&str> = decls
            .iter()
            .filter_map(|(d, _)| match d {
                Decl::Fn { name, .. } => Some(name.0.as_str()),
                _ => None,
            })
            .collect();
        let bodies: HashMap<&str, &Sp<Expr>> = decls
            .iter()
            .filter_map(|(d, _)| match d {
                Decl::Fn { name, body, .. } => Some((name.0.as_str(), body)),
                _ => None,
            })
            .collect();
        for (decl, _) in decls {
            let Decl::Fn {
                name,
                params,
                ret,
                body,
                ..
            } = decl
            else {
                continue;
            };
            // Entrypoint heuristic: `main`, or any nullary `Q<_>` function (common in fixtures).
            let is_main = name.0 == "main";
            let ret_is_q = matches!(ret.0, Type::Q(_));
            let is_entrypoint = is_main || (ret_is_q && params.is_empty());
            if !is_entrypoint {
                continue;
            }
            let mut has_qec = false;
            let mut has_bare = false;
            scan_qec_usage(body, &mut has_qec, &mut has_bare);
            scan_type_qec(&ret.0, &mut has_qec, &mut has_bare);
            for (_, t) in params {
                scan_type_qec(&t.0, &mut has_qec, &mut has_bare);
            }
            // Follow same-module callees (transitive) and consult resolved types.
            let mut stack: Vec<String> = {
                let mut direct = HashSet::new();
                collect_called_fns(body, &fn_names, &mut direct);
                direct.into_iter().collect()
            };
            let mut seen = HashSet::new();
            while let Some(callee) = stack.pop() {
                if !seen.insert(callee.clone()) {
                    continue;
                }
                if let Some(ty) = self.globals.get(&callee) {
                    if ty.mentions_qec_block() {
                        has_qec = true;
                    }
                    if ty.mentions_bare_qubit() {
                        has_bare = true;
                    }
                }
                if let Some(callee_body) = bodies.get(callee.as_str()) {
                    scan_qec_usage(callee_body, &mut has_qec, &mut has_bare);
                    let mut nested = HashSet::new();
                    collect_called_fns(callee_body, &fn_names, &mut nested);
                    for n in nested {
                        if !seen.contains(&n) {
                            stack.push(n);
                        }
                    }
                }
            }
            if has_qec && has_bare {
                errors.push(TypeError::MixedQecEntrypoint { span: name.1 });
            }
        }
    }
}

fn validate_qec_distance(
    family: &'static str,
    distance: u64,
    span: SimpleSpan,
) -> Result<(), TypeError> {
    let ok = match family {
        "repetition" => distance >= 2,
        "surface" => distance >= 3 && distance % 2 == 1,
        _ => true,
    };
    if ok {
        Ok(())
    } else {
        Err(TypeError::InvalidQecDistance {
            family,
            distance,
            span,
        })
    }
}

fn collect_kinded_bindings(
    expected: &Ty,
    got: &Ty,
    fam: &mut HashMap<String, CodeFamilyTy>,
    depth: &mut HashMap<String, DepthExpr>,
    span: SimpleSpan,
) -> Result<(), TypeError> {
    match (expected, got) {
        (
            Ty::QecBlock {
                family: ef,
                distance: ed,
            },
            Ty::QecBlock {
                family: gf,
                distance: gd,
            },
        ) => {
            if let CodeFamilyTy::Var(f) = ef {
                bind_family(fam, f, gf, span)?;
            }
            if let DepthExpr::Var(d) = ed {
                bind_depth(depth, d, gd, span)?;
            }
            Ok(())
        }
        (Ty::Q(a), Ty::Q(b)) | (Ty::List(a), Ty::List(b)) => {
            collect_kinded_bindings(a, b, fam, depth, span)
        }
        (Ty::Fn(a1, b1), Ty::Fn(a2, b2)) | (Ty::Linear(a1, b1), Ty::Linear(a2, b2)) => {
            collect_kinded_bindings(a1, a2, fam, depth, span)?;
            collect_kinded_bindings(b1, b2, fam, depth, span)
        }
        (Ty::Tuple(xs), Ty::Tuple(ys)) if xs.len() == ys.len() => {
            for (x, y) in xs.iter().zip(ys) {
                collect_kinded_bindings(x, y, fam, depth, span)?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn bind_family(
    fam: &mut HashMap<String, CodeFamilyTy>,
    name: &str,
    value: &CodeFamilyTy,
    span: SimpleSpan,
) -> Result<(), TypeError> {
    match fam.get(name) {
        Some(prev) if prev != value => Err(TypeError::Mismatch {
            expected: Ty::QecBlock {
                family: prev.clone(),
                distance: DepthExpr::Var("d".into()),
            },
            found: Ty::QecBlock {
                family: value.clone(),
                distance: DepthExpr::Var("d".into()),
            },
            span,
        }),
        Some(_) => Ok(()),
        None => {
            fam.insert(name.to_string(), value.clone());
            Ok(())
        }
    }
}

fn bind_depth(
    depth: &mut HashMap<String, DepthExpr>,
    name: &str,
    value: &DepthExpr,
    span: SimpleSpan,
) -> Result<(), TypeError> {
    match depth.get(name) {
        Some(prev) if !prev.equiv(value) => Err(TypeError::Mismatch {
            expected: Ty::QecBlock {
                family: CodeFamilyTy::Var("F".into()),
                distance: prev.clone(),
            },
            found: Ty::QecBlock {
                family: CodeFamilyTy::Var("F".into()),
                distance: value.clone(),
            },
            span,
        }),
        Some(_) => Ok(()),
        None => {
            depth.insert(name.to_string(), value.clone());
            Ok(())
        }
    }
}

fn subst_kinded_ty(
    ty: &Ty,
    fam: &HashMap<String, CodeFamilyTy>,
    depth: &HashMap<String, DepthExpr>,
) -> Ty {
    match ty {
        Ty::QecBlock { family, distance } => {
            let family = match family {
                CodeFamilyTy::Var(v) => fam.get(v).cloned().unwrap_or_else(|| family.clone()),
                other => other.clone(),
            };
            let distance = distance.subst(depth);
            Ty::QecBlock { family, distance }
        }
        Ty::Q(t) => Ty::Q(Box::new(subst_kinded_ty(t, fam, depth))),
        Ty::List(t) => Ty::List(Box::new(subst_kinded_ty(t, fam, depth))),
        Ty::Fn(a, b) => Ty::Fn(
            Box::new(subst_kinded_ty(a, fam, depth)),
            Box::new(subst_kinded_ty(b, fam, depth)),
        ),
        Ty::Linear(a, b) => Ty::Linear(
            Box::new(subst_kinded_ty(a, fam, depth)),
            Box::new(subst_kinded_ty(b, fam, depth)),
        ),
        Ty::Tuple(ts) => Ty::Tuple(ts.iter().map(|t| subst_kinded_ty(t, fam, depth)).collect()),
        Ty::QReg(n) => Ty::QReg(n.subst(depth)),
        Ty::Circuit { n, m, d, c } => Ty::Circuit {
            n: n.subst(depth),
            m: m.subst(depth),
            d: d.subst(depth),
            c: c.clone(),
        },
        Ty::Matrix(r, c, t) => Ty::Matrix(
            r.subst(depth),
            c.subst(depth),
            Box::new(subst_kinded_ty(t, fam, depth)),
        ),
        other => other.clone(),
    }
}

fn scan_qec_usage(expr: &Sp<Expr>, has_qec: &mut bool, has_bare: &mut bool) {
    match &expr.0 {
        Expr::Var(n) => match n.as_str() {
            "repetition_code" | "surface_code" | "surface_code_x" | "memory_round"
            | "measure_logical_z" | "measure_logical_x" | "logical_cx" => *has_qec = true,
            "qreg" | "qubit" | "init_one" | "init_plus" | "measure" | "measure_x" | "measure_y"
            | "measure_all" | "destructure" | "split" | "reset" | "discard" => *has_bare = true,
            _ => {}
        },
        Expr::TypeApp { callee, .. } => scan_qec_usage(callee, has_qec, has_bare),
        Expr::App(a, b)
        | Expr::Compose(a, b)
        | Expr::Par(a, b)
        | Expr::GateApp { gate: a, qubits: b } => {
            scan_qec_usage(a, has_qec, has_bare);
            scan_qec_usage(b, has_qec, has_bare);
        }
        Expr::BinOp { lhs, rhs, .. } => {
            scan_qec_usage(lhs, has_qec, has_bare);
            scan_qec_usage(rhs, has_qec, has_bare);
        }
        Expr::Neg(a)
        | Expr::Adjoint(a)
        | Expr::Controlled(a)
        | Expr::Return(a)
        | Expr::Ascribe(a, _)
        | Expr::Lam { body: a, .. } => scan_qec_usage(a, has_qec, has_bare),
        Expr::Let { rhs, body, .. }
        | Expr::Bind { rhs, body, .. }
        | Expr::For {
            iter: rhs, body, ..
        } => {
            scan_qec_usage(rhs, has_qec, has_bare);
            scan_qec_usage(body, has_qec, has_bare);
        }
        Expr::If { cond, then, else_ } => {
            scan_qec_usage(cond, has_qec, has_bare);
            scan_qec_usage(then, has_qec, has_bare);
            scan_qec_usage(else_, has_qec, has_bare);
        }
        Expr::Match { scrutinee, arms } => {
            scan_qec_usage(scrutinee, has_qec, has_bare);
            for (_, arm) in arms {
                scan_qec_usage(arm, has_qec, has_bare);
            }
        }
        Expr::Tuple(es) | Expr::List(es) => {
            for e in es {
                scan_qec_usage(e, has_qec, has_bare);
            }
        }
        Expr::CircuitBlock(ss) | Expr::RunBlock(ss) => {
            for s in ss {
                match &s.0 {
                    Stmt::Bind { rhs, .. } | Stmt::Let { rhs, .. } | Stmt::Expr(rhs) => {
                        scan_qec_usage(rhs, has_qec, has_bare);
                    }
                }
            }
        }
        Expr::Borrow { body, .. } => {
            for s in body {
                match &s.0 {
                    Stmt::Bind { rhs, .. } | Stmt::Let { rhs, .. } | Stmt::Expr(rhs) => {
                        scan_qec_usage(rhs, has_qec, has_bare);
                    }
                }
            }
        }
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unit => {}
    }
}

fn scan_type_qec(ty: &Type, has_qec: &mut bool, has_bare: &mut bool) {
    match ty {
        Type::QecBlock { .. } => *has_qec = true,
        Type::Qubit | Type::QReg(_) => *has_bare = true,
        Type::Q(t) | Type::List(t) => scan_type_qec(&t.0, has_qec, has_bare),
        Type::Fn(a, b) | Type::Linear(a, b) => {
            scan_type_qec(&a.0, has_qec, has_bare);
            scan_type_qec(&b.0, has_qec, has_bare);
        }
        Type::Tuple(ts) => {
            for t in ts {
                scan_type_qec(&t.0, has_qec, has_bare);
            }
        }
        Type::Matrix(_, _, t) => scan_type_qec(&t.0, has_qec, has_bare),
        _ => {}
    }
}

// ── Free helpers ───────────────────────────────────────────────────────────────

/// Whether `callee` is exactly the bare variable `name` (used to recognise special-form
/// applications like `destructure(q)` before the generic application rule fires).
fn is_named_call(callee: &Sp<Expr>, name: &str) -> bool {
    matches!(&callee.0, Expr::Var(n) if n == name)
}

/// Flatten a curried application `App(App(…App(head, a₁)…), aₙ)` whose outermost node is
/// `App(f, x)` into its head and left-to-right argument list. Used to dispatch circuit
/// combinators (`repeat(k, c)`, `fold(xs, z, s)`) by name and arity.
fn flatten_app<'a>(f: &'a Sp<Expr>, x: &'a Sp<Expr>) -> (&'a Sp<Expr>, Vec<&'a Sp<Expr>>) {
    let mut args = Vec::new();
    let mut cur = f;
    while let Expr::App(g, y) = &cur.0 {
        args.push(y.as_ref());
        cur = g;
    }
    args.reverse();
    args.push(x);
    (cur, args)
}

/// Collect, into `out`, the names of top-level functions (those in `fns`) that appear in
/// *application-head* position anywhere in `e` — the static call edges out of a function body
/// (issue #60). A bare reference that is not applied is not a call; everything reachable is walked
/// (circuit/run blocks, `for`/`borrow` bodies, match arms, lambda bodies, …) so a recursive call
/// buried in a `circuit { }` block or a `|>` chain is still seen.
fn collect_called_fns(
    e: &Sp<Expr>,
    fns: &std::collections::HashSet<&str>,
    out: &mut std::collections::HashSet<String>,
) {
    match &e.0 {
        Expr::App(f, x) => {
            let (head, args) = flatten_app(f, x);
            if let Expr::Var(n) = &head.0
                && fns.contains(n.as_str())
            {
                out.insert(n.clone());
            }
            collect_called_fns(head, fns, out);
            for a in args {
                collect_called_fns(a, fns, out);
            }
        }
        Expr::TypeApp { callee, .. } => collect_called_fns(callee, fns, out),
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unit | Expr::Var(_) => {}
        Expr::Lam { body, .. } => collect_called_fns(body, fns, out),
        Expr::BinOp { lhs, rhs, .. } => {
            collect_called_fns(lhs, fns, out);
            collect_called_fns(rhs, fns, out);
        }
        Expr::Neg(a)
        | Expr::Adjoint(a)
        | Expr::Controlled(a)
        | Expr::Return(a)
        | Expr::Ascribe(a, _) => collect_called_fns(a, fns, out),
        Expr::Compose(a, b) | Expr::Par(a, b) | Expr::GateApp { gate: a, qubits: b } => {
            collect_called_fns(a, fns, out);
            collect_called_fns(b, fns, out);
        }
        Expr::Let { rhs, body, .. } | Expr::Bind { rhs, body, .. } => {
            collect_called_fns(rhs, fns, out);
            collect_called_fns(body, fns, out);
        }
        Expr::If { cond, then, else_ } => {
            collect_called_fns(cond, fns, out);
            collect_called_fns(then, fns, out);
            collect_called_fns(else_, fns, out);
        }
        Expr::Match { scrutinee, arms } => {
            collect_called_fns(scrutinee, fns, out);
            for (_, b) in arms {
                collect_called_fns(b, fns, out);
            }
        }
        Expr::For { iter, body, .. } => {
            collect_called_fns(iter, fns, out);
            collect_called_fns(body, fns, out);
        }
        Expr::Tuple(es) | Expr::List(es) => {
            for e in es {
                collect_called_fns(e, fns, out);
            }
        }
        Expr::CircuitBlock(stmts) | Expr::RunBlock(stmts) | Expr::Borrow { body: stmts, .. } => {
            for s in stmts {
                collect_called_fns_stmt(s, fns, out);
            }
        }
    }
}

/// `collect_called_fns` for a statement (`pat <- e`, `let p = e`, or a trailing expression).
fn collect_called_fns_stmt(
    s: &Sp<Stmt>,
    fns: &std::collections::HashSet<&str>,
    out: &mut std::collections::HashSet<String>,
) {
    match &s.0 {
        Stmt::Bind { rhs, .. } | Stmt::Let { rhs, .. } => collect_called_fns(rhs, fns, out),
        Stmt::Expr(e) => collect_called_fns(e, fns, out),
    }
}

/// A non-negative integer literal used as a qubit index, for static bounds checking.
fn literal_index(e: &Sp<Expr>) -> Option<u64> {
    match &e.0 {
        Expr::Int(k) if *k >= 0 => Some(*k as u64),
        _ => None,
    }
}

/// Whether a rotation angle is a *static* multiple of `π/2` — the condition under which
/// `Rx`/`Ry`/`Rz` specialise to `Clifford` (SPEC §3.7). A runtime angle (or one this small
/// evaluator cannot fold) is conservatively treated as non-Clifford.
fn is_clifford_angle(e: &Sp<Expr>) -> bool {
    match eval_angle(e) {
        Some(v) => {
            let quarters = v / std::f64::consts::FRAC_PI_2;
            (quarters - quarters.round()).abs() < 1e-9
        }
        None => false,
    }
}

/// Constant-fold a `Float` angle expression, resolving the physics constants `PI`/`TAU`/`E`.
/// Returns `None` for anything not statically computable (e.g. a runtime variable).
fn eval_angle(e: &Sp<Expr>) -> Option<f64> {
    match &e.0 {
        Expr::Float(x) => Some(*x),
        Expr::Int(k) => Some(*k as f64),
        Expr::Var(name) => match name.as_str() {
            "PI" => Some(std::f64::consts::PI),
            "TAU" => Some(std::f64::consts::TAU),
            "E" => Some(std::f64::consts::E),
            _ => None,
        },
        Expr::Neg(a) => Some(-eval_angle(a)?),
        Expr::BinOp { op, lhs, rhs } => {
            let (a, b) = (eval_angle(lhs)?, eval_angle(rhs)?);
            Some(match op {
                BinOp::Add => a + b,
                BinOp::Sub => a - b,
                BinOp::Mul => a * b,
                BinOp::Div if b != 0.0 => a / b,
                BinOp::Div => return None,
                BinOp::Pow => a.powf(b),
            })
        }
        _ => None,
    }
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
        NatExpr::Sub(a, b) => nat_to_depth(&a.0)?.minus(nat_to_depth(&b.0)?),
        NatExpr::Div(a, b) => nat_to_depth(&a.0)?.quot(nat_to_depth(&b.0)?),
        NatExpr::Exp(a, b) => nat_to_depth(&a.0)?.power(nat_to_depth(&b.0)?),
        NatExpr::Hole => DepthExpr::Hole,
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
        Type::QecBlock { family, distance } => Type::QecBlock {
            family: Box::new(subst_nat_in_type(family, subst)),
            distance: subst_nat(distance, subst),
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

impl Default for TypeChecker {
    fn default() -> Self {
        Self::new()
    }
}
