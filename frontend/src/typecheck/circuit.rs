//! The **Circuit typing** module — the composition algebra, gate placement, Clifford
//! classification join, and DepthExpr assembly for `Circuit<n, m, d, C>` morphisms
//! (issues #11, #323; SPEC §3.3, §5.4–§5.8).
//!
//! ## Judgment form
//!
//! Quon's central type is the **Circuit** — a value of `Circuit<n, m, d, C>`, a unitary
//! quantum morphism consuming `n` qubits, producing `m` qubits, with gate **depth** bounded
//! by `d` (a **DepthExpr**) and **Clifford classification** `C` (`Clifford` | `Universal`).
//! This module owns the *Circuit* judgment: synthesizing the type of every circuit-forming
//! term and checking a `circuit { }` block against a declared `Circuit<…>` annotation.
//!
//! ```text
//!   Γ ; Δ ⊢ e ⇒ Circuit<n, m, d, C>     synthesis of a circuit morphism
//!   Γ ; Δ ⊢ e ⇐ Circuit<n, m, d, C>     checking a circuit block against an annotation
//! ```
//!
//! `Γ` (the unrestricted context, [`super::Env`]) holds classical names; `Δ` (the linear
//! context, [`super::Delta`]) tracks the qubit resources a circuit consumes exactly once.
//! The *classical* fragment, the **Quantum Monad** (`Q<τ>`, `<-` binds, `run { }`), and the
//! Z3-backed **refinement** bridge are *other slices* (#325, #326) and stay in
//! [`super::TypeChecker`]; only the Circuit judgment lives here.
//!
//! ## What this module owns
//!
//! * **Gate placement** (`H @ 0`, `H(q)`) — embedding a gate into the innermost circuit
//!   register ([`TypeChecker::place_gate`], [`TypeChecker::placement_width`]). The ambient
//!   register width is the top of the checker's `circuit_width` stack.
//! * **Sequential composition** `|>` — output width of `f` equals input width of `g`;
//!   depths add (`DepthExpr::seq`); classes join ([`TypeChecker::synth_compose`]).
//! * **Parallel composition** `par` — `k`-fold tensor; qubit counts scale by `k`, depth is
//!   the max of both layers (`DepthExpr::par`) ([`TypeChecker::synth_par`]).
//! * **Adjoint** / **controlled** — width swap / `+1` widths and `+1` depth
//!   (`DepthExpr::controlled`) ([`TypeChecker::synth_adjoint`],
//!   [`TypeChecker::synth_controlled`]).
//! * **Clifford classification join** — `Clifford ∘ Clifford = Clifford`, any `Universal`
//!   absorbs ([`CliffordClass::join`]); branch circuits that agree on widths join depth by
//!   `max` (ADR-0005) ([`TypeChecker::join_branch_types`]).
//! * **DepthExpr assembly** — `seq`/`par`/`repeat`/`controlled` over the circuit indices,
//!   plus the `identity`/`repeat`/`on_high`/`on_low`/`swap_reverse`/`for`/`fold` families
//!   that build a circuit's depth bound.
//! * **Gate registry** — gate metadata (arity, class, aliases) comes from
//!   [`quon_core::gates`], the single registry shared with backend / cancellation / emit
//!   (issue #209). [`gate_type`]/[`rotation_arity`]/[`is_specialisable_rotation`] are the
//!   static signatures; no new string tables live here.
//!
//! ## Module boundary (ADR-0028)
//!
//! This is a pure code-motion carve out of the [`super::TypeChecker`] monolith: the methods
//! below are *methods on [`super::TypeChecker`]* (they read/write the checker's `table`,
//! `refine`, `assumptions`, and `circuit_width`/`circuit_width_cap` stacks), kept as a
//! single `impl` block in this file. [`super::TypeChecker`] remains the bidirectional
//! facade that dispatches into them; the Q-monad and refinement slices are deliberately
//! not moved (issues #325, #326). Pure circuit application (`@` outside a block, ADR-0006)
//! and the bind auto-lift live with the Q-monad slice and stay in the facade.

use crate::ast::{CliffordClass, Expr, Pat, Stmt};
use crate::lexer::{SimpleSpan, Sp};
use crate::types::Ty;
use quon_core::DepthExpr;
use quon_core::gates::{GateClass, surface_gate};

use super::error::TypeError;
use super::{Delta, Env, flatten_app, is_clifford_angle, literal_index};

// ── Gate registry: static signatures ──────────────────────────────────────────

/// A circuit type with literal dimensions and unit depth — the shape of every gate.
fn gate_ty(n: u64, depth: u64, class: CliffordClass) -> Ty {
    Ty::Circuit {
        n: DepthExpr::Nat(n),
        m: DepthExpr::Nat(n),
        d: DepthExpr::Nat(depth),
        c: class,
    }
}

fn to_clifford_class(class: GateClass) -> CliffordClass {
    match class {
        GateClass::Clifford => CliffordClass::Clifford,
        GateClass::Universal => CliffordClass::Universal,
    }
}

/// The type of a gate primitive `name`, if it is one.
///
/// Non-parametric gates synthesize directly to a `Circuit` value; parametric rotations
/// synthesize to a function `Float -> Circuit<…>` (applied to their angle at the use site).
/// The class here is the gate's *intrinsic* classification; [`rotation_arity`] marks the
/// rotations whose class issue #12 specialises from the static angle.
pub fn gate_type(name: &str) -> Option<Ty> {
    let info = surface_gate(name)?;
    let class = to_clifford_class(info.class);
    let arity = info.arity as u64;
    if info.parametric {
        Some(Ty::func(Ty::Float, gate_ty(arity, 1, class)))
    } else {
        Some(gate_ty(arity, 1, class))
    }
}

/// For a parametric gate, the qubit arity of the circuit it produces (so the checker can
/// place it), or `None` if `name` is not a parametric gate. Single-qubit rotations have
/// arity 1; two-qubit ones arity 2.
pub fn rotation_arity(name: &str) -> Option<u64> {
    surface_gate(name)
        .filter(|g| g.parametric)
        .map(|g| g.arity as u64)
}

/// Whether `name` is a single-qubit rotation whose class issue #12 specialises to `Clifford`
/// at static multiples of `π/2`.
pub fn is_specialisable_rotation(name: &str) -> bool {
    surface_gate(name)
        .is_some_and(|g| g.parametric && g.arity == 1 && matches!(g.id, "Rx" | "Ry" | "Rz"))
}

// ── Circuit judgment: composition algebra, placement, branch join ─────────────
//
// All of the following are methods on `TypeChecker`, moved here from the facade monolith as
// a pure code-motion carve (ADR-0028). They read and write the checker's shared state — the
// metavariable `table`, the `refine`ment bridge, the active `assumptions`, and the
// `circuit_width`/`circuit_width_cap` register stacks — and call back into the facade's
// generic `synth`/`check`/`expr_to_depth`/`depth_of` helpers (child-module access). Only the
// methods the facade dispatches into are `pub(super)`; intra-circuit helpers stay private.

impl super::TypeChecker {
    /// View `ty` as a circuit, returning its four indices.
    fn as_circuit(
        &self,
        ty: &Ty,
        span: SimpleSpan,
    ) -> Result<(DepthExpr, DepthExpr, DepthExpr, CliffordClass), TypeError> {
        match self.table.resolve(ty) {
            Ty::Circuit { n, m, d, c } => Ok((n, m, d, c)),
            other => Err(TypeError::NotACircuit { found: other, span }),
        }
    }

    /// The ambient circuit-register width, i.e. the `n` of the innermost enclosing
    /// `circuit { }` block — the register a bare gate placement targets.
    fn ambient_width(&self, span: SimpleSpan) -> Result<DepthExpr, TypeError> {
        self.circuit_width
            .last()
            .cloned()
            .ok_or(TypeError::Unsupported {
                construct: "gate placement outside a circuit block",
                span,
            })
    }

    /// Place a gate (`gate_ty : Circuit<g,g,d,C>`) onto qubit targets within the ambient
    /// register, yielding `Circuit<w,w,d,C>` (SPEC §5.6). The number of targets must equal
    /// the gate's arity, and each literal index must lie in `0..w`.
    pub(super) fn place_gate(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        gate_ty: Ty,
        qubits: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let (gn, _, gd, gc) = self.as_circuit(&gate_ty, qubits.1)?;
        let arity = gn.as_const().unwrap_or(1);
        let targets: Vec<&Sp<Expr>> = match &qubits.0 {
            Expr::Tuple(es) => es.iter().collect(),
            _ => vec![qubits],
        };
        if targets.len() as u64 != arity {
            return Err(TypeError::GateTargetArity {
                expected: arity,
                found: targets.len(),
                span: qubits.1,
            });
        }
        let width = self.ambient_width(qubits.1)?;
        let placement = self.placement_width(&width, &targets)?;
        for t in &targets {
            self.check(env, delta, t, &Ty::Int)?;
            if let (Some(w), Some(idx)) = (placement.as_const(), literal_index(t))
                && idx >= w
            {
                return Err(TypeError::IndexOutOfBounds {
                    index: idx,
                    width: w,
                    span: t.1,
                });
            }
        }
        if let Some(cap) = self.circuit_width_cap.last()
            && let (Some(p), Some(m)) = (placement.as_const(), cap.as_const())
            && p > m
        {
            return Err(TypeError::IndexOutOfBounds {
                index: targets
                    .iter()
                    .filter_map(|t| literal_index(t))
                    .max()
                    .unwrap_or(0),
                width: m,
                span: qubits.1,
            });
        }
        if let Some(top) = self.circuit_width.last_mut() {
            *top = top.clone().par(placement.clone()).normalize();
        }
        Ok(Ty::Circuit {
            n: width,
            m: placement,
            d: gd,
            c: gc,
        })
    }

    /// The register footprint a gate placement needs: at least the ambient width, and wide
    /// enough to cover every target index.
    ///
    /// Growth only applies when the ambient width is a *constant* (a width-changing encoder
    /// like `Circuit<1, 3, …>`): there a literal index beyond the current footprint genuinely
    /// widens the register. When the ambient width is *symbolic* (`Circuit<n, …>`), a gate on a
    /// literal wire stays within the declared `n`-wide register — growing it to `max(idx+1, n)`
    /// would be unsound (`max(1, n) ≠ n` for `n = 0`) and would break composition with any
    /// other `n`-wide circuit, so the ambient width is preserved.
    fn placement_width(
        &self,
        ambient: &DepthExpr,
        targets: &[&Sp<Expr>],
    ) -> Result<DepthExpr, TypeError> {
        let max_idx = targets
            .iter()
            .filter_map(|t| literal_index(t))
            .max()
            .unwrap_or(0);
        match ambient.as_const() {
            Some(a) => Ok(DepthExpr::Nat(a.max(max_idx.saturating_add(1)))),
            None => Ok(ambient.clone()),
        }
    }

    /// Circuit *application* `c @ r` outside a `circuit { }` block (issue #14, SPEC §5.9
    /// `apply`). The circuit `c : Circuit<n,m,d,C>` is applied to a register source `r`,
    /// consuming it and producing the output register of width `m`:
    ///
    /// * `r : QReg<n>`        ⟶ `QReg<m>`        — pure unitary application
    /// * `r : Qubit` (n=1)    ⟶ `Qubit`          — single-qubit gate on a qubit handle
    /// * `r : Q<QReg<n>>`     ⟶ `Q<QReg<m>>`     — the register source is itself monadic
    ///
    /// Unitary application has no measurement, so the pure cases stay outside `Q` (the monad
    /// threads the global state, SPEC §3.5); only a monadic source lifts the result into `Q`.
    pub(super) fn apply_circuit(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        circ: Ty,
        reg: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let (n, m, _, _) = self.as_circuit(&circ, reg.1)?;
        let reg_ty = self.synth(env, delta, reg)?;
        // A register source nested in `Q<…>` threads the monad: the result is `Q<QReg<m>>`.
        let (monadic, source) = match self.table.resolve(&reg_ty) {
            Ty::Q(inner) => (true, *inner),
            other => (false, other),
        };
        let out = match source {
            Ty::QReg(w) => {
                self.expect_width(&n, &w, reg.1)?;
                Ty::QReg(m)
            }
            // A single-qubit gate applied to a qubit handle returns a qubit handle.
            Ty::Qubit => {
                self.expect_width(&n, &DepthExpr::Nat(1), reg.1)?;
                if m.equiv(&DepthExpr::Nat(1)) {
                    Ty::Qubit
                } else {
                    Ty::QReg(m)
                }
            }
            other => {
                return Err(TypeError::Mismatch {
                    expected: Ty::QReg(n),
                    found: other,
                    span: reg.1,
                });
            }
        };
        Ok(if monadic { Ty::Q(Box::new(out)) } else { out })
    }

    /// A circuit's input width must match the register it is applied to.
    fn expect_width(
        &self,
        expected: &DepthExpr,
        found: &DepthExpr,
        span: SimpleSpan,
    ) -> Result<(), TypeError> {
        if self
            .refine
            .prove_eq(&self.assumptions, expected, found)
            .is_ok()
        {
            Ok(())
        } else {
            Err(TypeError::QubitCountMismatch {
                expected: expected.to_string(),
                found: found.to_string(),
                span,
            })
        }
    }

    /// `f |> g` (SPEC §3.3): `f`'s output width must equal `g`'s input width; depths add,
    /// classes join.
    pub(super) fn synth_compose(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        l: &Sp<Expr>,
        r: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let lt = self.synth(env, delta, l)?;
        let rt = self.synth(env, delta, r)?;
        let (ln, lm, ld, lc) = self.as_circuit(&lt, l.1)?;
        let (rn, rm, rd, rc) = self.as_circuit(&rt, r.1)?;
        if self.refine.prove_eq(&self.assumptions, &lm, &rn).is_err() {
            return Err(TypeError::QubitCountMismatch {
                expected: lm.to_string(),
                found: rn.to_string(),
                span,
            });
        }
        let composed = Ty::Circuit {
            n: ln.clone(),
            m: rm.clone(),
            d: ld.seq(rd),
            c: lc.join(&rc),
        };
        // Only update the ambient register for compositions of *placements*
        // on that register (`ln` matches ambient). Bare gate values such as
        // `H |> T` under `controlled(...)` have their own width and must not
        // shrink the surrounding circuit's ambient (issue #182).
        if let Some(top) = self.circuit_width.last_mut()
            && self.refine.prove_eq(&self.assumptions, top, &ln).is_ok()
            && !top.equiv(&rm)
        {
            *top = rm;
        }
        Ok(composed)
    }

    /// `par { body } * k` (SPEC §5.8): `k`-fold tensor of `body` with itself — qubit counts
    /// scale by `k`, depth is unchanged (identical layers run in parallel).
    pub(super) fn synth_par(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        body: &Sp<Expr>,
        count: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let bt = self.synth(env, delta, body)?;
        let (bn, bm, bd, bc) = self.as_circuit(&bt, body.1)?;
        let k = self.expr_to_depth(env, delta, count)?;
        Ok(Ty::Circuit {
            n: DepthExpr::repeat(k.clone(), bn),
            m: DepthExpr::repeat(k, bm),
            d: bd,
            c: bc,
        })
    }

    /// `adjoint(c)` (SPEC §3.3): the unitary inverse swaps input/output widths.
    pub(super) fn synth_adjoint(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        c: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let ct = self.synth(env, delta, c)?;
        let (n, m, d, cl) = self.as_circuit(&ct, c.1)?;
        Ok(Ty::Circuit {
            n: m,
            m: n,
            d,
            c: cl,
        })
    }

    /// `controlled(c)` (SPEC §3.3): adds a control qubit — widths and depth each gain one.
    pub(super) fn synth_controlled(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        c: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let ct = self.synth(env, delta, c)?;
        let (n, m, d, cl) = self.as_circuit(&ct, c.1)?;
        Ok(Ty::Circuit {
            n: n.seq(DepthExpr::Nat(1)),
            m: m.seq(DepthExpr::Nat(1)),
            d: d.controlled(),
            c: cl,
        })
    }

    /// `swap_reverse(n) : Circuit<n,n,n/2,Clifford>` (SPEC §5.7).
    pub(super) fn synth_swap_reverse(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        n: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let w = self.expr_to_depth(env, delta, n)?;
        Ok(Ty::Circuit {
            n: w.clone(),
            m: w.clone(),
            d: w.quot(DepthExpr::Nat(2)),
            c: CliffordClass::Clifford,
        })
    }

    /// `identity(n) : Circuit<n,n,0,Clifford>` (SPEC §5.7).
    pub(super) fn synth_identity(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        n: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let size = self.expr_to_depth(env, delta, n)?;
        Ok(Ty::Circuit {
            n: size.clone(),
            m: size,
            d: DepthExpr::Nat(0),
            c: CliffordClass::Clifford,
        })
    }

    /// `repeat(k, c) : Circuit<n,n,k*d,C>` (SPEC §5.7) — `k`-fold sequential repetition.
    pub(super) fn synth_repeat(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        k: &Sp<Expr>,
        c: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let k_depth = self.expr_to_depth(env, delta, k)?;
        let ct = self.synth(env, delta, c)?;
        let (n, m, d, cl) = self.as_circuit(&ct, c.1)?;
        Ok(Ty::Circuit {
            n,
            m,
            d: DepthExpr::repeat(k_depth, d),
            c: cl,
        })
    }

    /// `on_high(c, n)` / `on_low(c, n)` (SPEC §5.7): embed a `k`-qubit circuit into an
    /// `n`-qubit register; depth and class are preserved.
    pub(super) fn synth_on_sub(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        c: &Sp<Expr>,
        n: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        let ct = self.synth(env, delta, c)?;
        let (_, _, d, cl) = self.as_circuit(&ct, c.1)?;
        let width = self.expr_to_depth(env, delta, n)?;
        Ok(Ty::Circuit {
            n: width.clone(),
            m: width,
            d,
            c: cl,
        })
    }

    /// A single-qubit rotation `Rx`/`Ry`/`Rz` applied to its angle (issue #12, SPEC §3.7,
    /// §5.4): the result is `Circuit<1,1,1,C>` where `C` is `Clifford` exactly when the angle
    /// is a *static* multiple of `π/2` (`0, π/2, π, 3π/2, …`) and `Universal` otherwise.
    pub(super) fn synth_rotation(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        angle: &Sp<Expr>,
    ) -> Result<Ty, TypeError> {
        self.check(env, delta, angle, &Ty::Float)?;
        let class = if is_clifford_angle(angle) {
            CliffordClass::Clifford
        } else {
            CliffordClass::Universal
        };
        Ok(Ty::Circuit {
            n: DepthExpr::Nat(1),
            m: DepthExpr::Nat(1),
            d: DepthExpr::Nat(1),
            c: class,
        })
    }

    /// A circuit-producing `for` loop (SPEC §5.8). The body is a circuit placed once per
    /// iteration; whether the iterations run in parallel (depth = body depth) or in sequence
    /// (depth = count × body depth) follows the iterator: `qubits`/`diag` are parallel,
    /// `range`/`pairs` sequential. True data-dependency analysis is deferred (issue #13).
    pub(super) fn synth_for(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        pat: &Sp<Pat>,
        iter: &Sp<Expr>,
        body: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let (count, sequential, elem) = self.iter_info(iter, span)?;
        let saved_ambient = self.circuit_width.last().cloned();
        // The loop variable(s) are classical qubit indices; bind them into Γ for the body.
        let mut inner = env.clone();
        let mut bound = Vec::new();
        self.check_pat_into(pat, None, &elem, &mut inner, delta, &mut bound)?;
        let body_ty = self.synth(&inner, delta, body)?;
        if let (Some(saved), Some(top)) = (saved_ambient.as_ref(), self.circuit_width.last_mut()) {
            *top = saved.clone();
        }
        let (_bn, bm, bd, bc) = self.as_circuit(&body_ty, body.1)?;
        let depth = if sequential {
            DepthExpr::repeat(count.clone(), bd)
        } else {
            bd
        };
        let reg_width = if sequential {
            saved_ambient.unwrap_or(bm)
        } else {
            count.clone()
        };
        Ok(Ty::Circuit {
            n: reg_width.clone(),
            m: reg_width,
            d: depth,
            c: bc,
        })
    }

    /// Recognise a `for`-loop iterator and report `(count, is_sequential, element_type)`.
    fn iter_info(
        &mut self,
        iter: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<(DepthExpr, bool, Ty), TypeError> {
        if let Expr::App(f, x) = &iter.0
            && let Expr::Var(name) = &f.0
        {
            match name.as_str() {
                // qubit-indexed iterators: a single `Int` index per step.
                "qubits" | "diag" => return Ok((self.depth_of(x)?, false, Ty::Int)),
                "range" => return Ok((self.depth_of(x)?, true, Ty::Int)),
                // ordered pairs `(i, j)` of indices.
                "pairs" => {
                    let w = self.depth_of(x)?;
                    return Ok((
                        DepthExpr::repeat(w.clone(), w),
                        true,
                        Ty::Tuple(vec![Ty::Int, Ty::Int]),
                    ));
                }
                _ => {}
            }
        }
        Err(TypeError::Unsupported {
            construct: "circuit for-loop iterator",
            span,
        })
    }

    /// `fold` whose accumulator is a circuit (SPEC §3.6): `fold(xs, identity(n), step)` runs
    /// a depth-`s` layer once per element, so its depth is `len(xs) * s`. When the
    /// accumulator is *not* a circuit this is the ordinary classical fold.
    pub(super) fn synth_fold(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        list: &Sp<Expr>,
        init: &Sp<Expr>,
        step: &Sp<Expr>,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let list_ty = self.synth(env, delta, list)?;
        let elem = self.table.fresh();
        self.table
            .unify(&list_ty, &Ty::list(elem.clone()), list.1)?;

        let init_ty = self.synth(env, delta, init)?;
        if let Ty::Circuit { n, m, c: c0, .. } = self.table.resolve(&init_ty) {
            // Circuit accumulator: type the step with a *zero-depth* accumulator so the body's
            // depth is exactly the per-iteration increment `s`; total depth is `len * s`.
            let len = self.list_length(list).ok_or(TypeError::Unsupported {
                construct: "fold whose length is not statically known",
                span,
            })?;
            let zero_acc = Ty::Circuit {
                n: n.clone(),
                m: m.clone(),
                d: DepthExpr::Nat(0),
                c: c0.clone(),
            };
            let (_, sm, sd, sc) = self.synth_fold_step(env, delta, step, &zero_acc, &elem)?;
            return Ok(Ty::Circuit {
                n,
                m: sm,
                d: DepthExpr::repeat(len, sd),
                c: c0.join(&sc),
            });
        }
        // Classical fold: `step : B -> A -> B`, result `B`.
        let b = init_ty;
        let step_ty = Ty::func(b.clone(), Ty::func(elem, b.clone()));
        self.check(env, delta, step, &step_ty)?;
        Ok(b)
    }

    /// Type a fold's step lambda `fn(acc, x) -> body` with a fixed accumulator type, returning
    /// the body's circuit indices (the per-iteration layer).
    fn synth_fold_step(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        step: &Sp<Expr>,
        acc_ty: &Ty,
        elem_ty: &Ty,
    ) -> Result<(DepthExpr, DepthExpr, DepthExpr, CliffordClass), TypeError> {
        let Expr::Lam { params, body } = &step.0 else {
            return Err(TypeError::Unsupported {
                construct: "fold step that is not a two-parameter lambda",
                span: step.1,
            });
        };
        if params.len() != 2 {
            return Err(TypeError::Unsupported {
                construct: "fold step that is not a two-parameter lambda",
                span: step.1,
            });
        }
        let mut inner = env.clone();
        let mut lam_delta = Delta::new();
        let mut introduced = Vec::new();
        self.check_pat_into(
            &params[0].0,
            None,
            acc_ty,
            &mut inner,
            &mut lam_delta,
            &mut introduced,
        )?;
        self.check_pat_into(
            &params[1].0,
            None,
            elem_ty,
            &mut inner,
            &mut lam_delta,
            &mut introduced,
        )?;
        let body_ty = self.in_lambda_scope(delta, &mut lam_delta, &introduced, |me, d| {
            me.synth(&inner, d, body)
        })?;
        self.as_circuit(&body_ty, body.1)
    }

    /// The statically-known length of a list-producing expression: `range(k) ⇒ k`,
    /// `take(p, xs) ⇒ p`, a literal list ⇒ its element count.
    fn list_length(&self, list: &Sp<Expr>) -> Option<DepthExpr> {
        match &list.0 {
            Expr::List(es) => Some(DepthExpr::Nat(es.len() as u64)),
            Expr::App(f, x) => {
                let (head, args) = flatten_app(f, x);
                match (&head.0, args.as_slice()) {
                    (Expr::Var(n), [k]) if n == "range" => self.depth_of(k).ok(),
                    (Expr::Var(n), [p, _]) if n == "take" => self.depth_of(p).ok(),
                    _ => None,
                }
            }
            _ => None,
        }
    }

    /// Check a `circuit { }` block against its declared `Circuit<n,m,d,C>` type. The block's
    /// input width `n` becomes the ambient register for gate placement; the body's inferred
    /// indices, depth, and class are then unified against the annotation.
    pub(super) fn check_circuit_block(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        stmts: &[Sp<Stmt>],
        expected: &Ty,
        span: SimpleSpan,
    ) -> Result<(), TypeError> {
        let (en, em, ..) = self.as_circuit(expected, span)?;
        self.circuit_width.push(en);
        self.circuit_width_cap.push(em);
        let result = self.synth_block_body(env, delta, stmts, span);
        self.circuit_width.pop();
        self.circuit_width_cap.pop();
        let inferred = result?;
        // The block's input width seeds gate placement; its inferred indices, class, and depth
        // are then reconciled with the annotation — widths/class structurally, depth via Z3.
        self.expect_type(expected, &inferred, span)
    }

    /// Synthesize the circuit a block body denotes: leading `let`s bind into scope, the final
    /// expression is the circuit. (`<-` monadic binds belong to `run { }`, issue #14.)
    pub(super) fn synth_block_body(
        &mut self,
        env: &Env,
        delta: &mut Delta,
        stmts: &[Sp<Stmt>],
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        let Some((last, leading)) = stmts.split_last() else {
            return Err(TypeError::Unsupported {
                construct: "empty circuit block",
                span,
            });
        };
        let mut inner = env.clone();
        let mut introduced = Vec::new();
        for stmt in leading {
            match &stmt.0 {
                Stmt::Let { pat, rhs } => {
                    let rhs_ty = self.synth(&inner, delta, rhs)?;
                    let mut bound = self.bind_pat(pat, &rhs_ty, &mut inner, delta)?;
                    introduced.append(&mut bound);
                }
                _ => {
                    return Err(TypeError::Unsupported {
                        construct: "non-let statement inside a circuit block",
                        span: stmt.1,
                    });
                }
            }
        }
        let Stmt::Expr(e) = &last.0 else {
            return Err(TypeError::Unsupported {
                construct: "circuit block not ending in an expression",
                span: last.1,
            });
        };
        let circ = self.synth(&inner, delta, e)?;
        self.ensure_consumed(delta, &introduced)?;
        Ok(circ)
    }

    /// Join the result types of two alternative branches in synthesis mode. Circuits that
    /// agree on widths and would otherwise differ only in depth join by `max` (`DepthExpr::par`)
    /// — a classically-selected circuit has the worst-case depth of its arms (SPEC §3.3), so a
    /// correction like `if b then X else identity(1)` is well-typed at `max(1, 0) = 1`. The
    /// Clifford class joins (Universal absorbs Clifford). Everything else must unify outright.
    ///
    /// Depth *verification* against a user annotation stays strict (issue #13, `verify_depth`);
    /// this `max` is inference of a branch's depth, not a relaxation of annotation checking.
    pub(super) fn join_branch_types(
        &mut self,
        a: &Ty,
        b: &Ty,
        span: SimpleSpan,
    ) -> Result<Ty, TypeError> {
        if let (
            Ty::Circuit {
                n: an,
                m: am,
                d: ad,
                c: ac,
            },
            Ty::Circuit {
                n: bn,
                m: bm,
                d: bd,
                c: bc,
            },
        ) = (self.table.resolve(a), self.table.resolve(b))
            && an.equiv(&bn)
            && am.equiv(&bm)
        {
            let d = if ad.equiv(&bd) {
                ad
            } else {
                ad.par(bd).normalize()
            };
            return Ok(Ty::Circuit {
                n: an,
                m: am,
                d,
                c: ac.join(&bc),
            });
        }
        self.table.unify(a, b, span)?;
        Ok(self.table.zonk(a))
    }
}

#[cfg(test)]
mod tests {
    //! Locks the extracted Circuit-typing interface. The composition algebra is exercised
    //! directly as pure functions — `DepthExpr::seq` (sequential `|>`, depth adds),
    //! `DepthExpr::par` (parallel `par`, depth = max), `DepthExpr::controlled` (`+1`),
    //! `DepthExpr::repeat` (`k*d`), and `CliffordClass::join` (Clifford ∘ Clifford, Universal
    //! absorbs) — plus the gate-registry signatures [`gate_type`]/[`rotation_arity`]/
    //! [`is_specialisable_rotation`]. These are the exact helpers the moved circuit methods
    //! build on, so a regression here flags a change in the carved interface itself.

    use super::*;
    use crate::ast::CliffordClass;
    use proptest::prelude::*;

    // ── Clifford classification join (ADR-0005: branch depth max; Universal absorbs) ──

    #[test]
    fn clifford_join_is_lattice_supremum() {
        use CliffordClass::*;
        assert_eq!(Clifford.join(&Clifford), Clifford);
        assert_eq!(Clifford.join(&Universal), Universal);
        assert_eq!(Universal.join(&Clifford), Universal);
        assert_eq!(Universal.join(&Universal), Universal);
        // `Infer` is the Clifford bottom (issue #12): it never poisons a join.
        assert_eq!(Clifford.join(&Infer), Clifford);
        assert_eq!(Infer.join(&Universal), Universal);
    }

    // ── Sequential composition `|>`: depths add ──────────────────────────────────────

    #[test]
    fn seq_depth_adds() {
        let d = DepthExpr::Nat(2).seq(DepthExpr::Nat(3));
        assert_eq!(d.as_const(), Some(5));
        assert!(d.equiv(&DepthExpr::Nat(5)));
        // `x + 0 ≡ x` (normalize drops the additive identity).
        assert!(
            DepthExpr::Nat(7)
                .seq(DepthExpr::Nat(0))
                .equiv(&DepthExpr::Nat(7))
        );
    }

    // ── Parallel composition `par`: depth is the max ─────────────────────────────────

    #[test]
    fn par_depth_is_max() {
        let d = DepthExpr::Nat(2).par(DepthExpr::Nat(3));
        assert_eq!(d.as_const(), Some(3));
        assert!(d.equiv(&DepthExpr::Nat(3)));
        // `max(x, 0) ≡ x`; `max(x, x) ≡ x` (normalize dedupes).
        assert!(
            DepthExpr::Nat(4)
                .par(DepthExpr::Nat(0))
                .equiv(&DepthExpr::Nat(4))
        );
        assert!(
            DepthExpr::Nat(4)
                .par(DepthExpr::Nat(4))
                .equiv(&DepthExpr::Nat(4))
        );
    }

    // ── Controlled: depth + 1 ────────────────────────────────────────────────────────

    #[test]
    fn controlled_adds_one() {
        // `controlled(c)` adds a control qubit — widths and depth each gain one.
        let d = DepthExpr::Nat(2).controlled();
        assert_eq!(d.as_const(), Some(3));
        assert!(d.equiv(&DepthExpr::Nat(3)));
    }

    // ── Repeat: k * d (sequential repetition, SPEC §5.7) ─────────────────────────────

    #[test]
    fn repeat_multiplies() {
        let d = DepthExpr::repeat(DepthExpr::Nat(2), DepthExpr::Nat(3));
        assert_eq!(d.as_const(), Some(6));
        assert!(d.equiv(&DepthExpr::Nat(6)));
        // `1 * d ≡ d` (normalize drops the multiplicative identity).
        assert!(DepthExpr::repeat(DepthExpr::Nat(1), DepthExpr::Nat(7)).equiv(&DepthExpr::Nat(7)));
        // `0 * d ≡ 0` (absorbing).
        assert!(DepthExpr::repeat(DepthExpr::Nat(0), DepthExpr::Nat(7)).equiv(&DepthExpr::Nat(0)));
    }

    // ── DepthExpr normalize/equiv oracle (quon_core) over the composition algebra ─────

    proptest! {
        // The `normalize`/`equiv` oracle in `quon_core` decides the constant + AC cases the
        // circuit composition rules produce. For all constant depths, the assembled expr must
        // agree with the literal arithmetic the judgment promises.

        #[test]
        fn seq_adds_constants(a in 0u64..64, b in 0u64..64) {
            let d = DepthExpr::Nat(a).seq(DepthExpr::Nat(b));
            prop_assert_eq!(d.as_const(), Some(a.saturating_add(b)));
            prop_assert!(d.equiv(&DepthExpr::Nat(a.saturating_add(b))));
        }

        #[test]
        fn par_takes_max_constants(a in 0u64..64, b in 0u64..64) {
            let d = DepthExpr::Nat(a).par(DepthExpr::Nat(b));
            prop_assert_eq!(d.as_const(), Some(a.max(b)));
            prop_assert!(d.equiv(&DepthExpr::Nat(a.max(b))));
        }

        #[test]
        fn controlled_adds_one_constant(a in 0u64..64) {
            let d = DepthExpr::Nat(a).controlled();
            prop_assert_eq!(d.as_const(), Some(a.saturating_add(1)));
            prop_assert!(d.equiv(&DepthExpr::Nat(a.saturating_add(1))));
        }

        #[test]
        fn repeat_multiplies_constants(k in 0u64..16, d in 0u64..64) {
            let e = DepthExpr::repeat(DepthExpr::Nat(k), DepthExpr::Nat(d));
            prop_assert_eq!(e.as_const(), Some(k.saturating_mul(d)));
            prop_assert!(e.equiv(&DepthExpr::Nat(k.saturating_mul(d))));
        }

        /// A classically-selected branch (`if b then X else identity(1)`, ADR-0005) joins
        /// depths by `max`; the assemble-then-normalize result must equal `max(ad, bd)`.
        #[test]
        fn branch_join_is_max(ad in 0u64..64, bd in 0u64..64) {
            let joined = DepthExpr::Nat(ad).par(DepthExpr::Nat(bd)).normalize();
            prop_assert!(joined.equiv(&DepthExpr::Nat(ad.max(bd))));
        }
    }

    // ── Gate-registry signatures (the static surface this module still owns) ─────────

    #[test]
    fn gate_type_signatures_match_registry() {
        // A non-parametric Clifford gate: H : Circuit<1,1,1,Clifford>.
        let h = gate_type("H").expect("H is a gate");
        let Ty::Circuit { n, m, d, c } = h else {
            panic!("H synthesizes to a Circuit, got {h:?}");
        };
        assert_eq!(n.as_const(), Some(1));
        assert_eq!(m.as_const(), Some(1));
        assert_eq!(d.as_const(), Some(1));
        assert_eq!(c, CliffordClass::Clifford);

        // CNOT : Circuit<2,2,1,Clifford>.
        let cnot = gate_type("CNOT").expect("CNOT is a gate");
        let Ty::Circuit { n, m, d, c } = cnot else {
            panic!("CNOT synthesizes to a Circuit, got {cnot:?}");
        };
        assert_eq!(n.as_const(), Some(2));
        assert_eq!(m.as_const(), Some(2));
        assert_eq!(d.as_const(), Some(1));
        assert_eq!(c, CliffordClass::Clifford);
    }

    #[test]
    fn rotation_arity_and_specialisation() {
        // Single-qubit rotations are parametric with arity 1 and specialisable.
        assert_eq!(rotation_arity("Rz"), Some(1));
        assert!(is_specialisable_rotation("Rz"));
        assert!(is_specialisable_rotation("Rx"));
        assert!(is_specialisable_rotation("Ry"));
        // A non-parametric gate is not a rotation arity source / not specialisable.
        assert_eq!(rotation_arity("H"), None);
        assert!(!is_specialisable_rotation("H"));
        // Unknown names resolve to nothing.
        assert!(gate_type("not_a_gate").is_none());
        assert_eq!(rotation_arity("not_a_gate"), None);
        assert!(!is_specialisable_rotation("not_a_gate"));
    }
}
