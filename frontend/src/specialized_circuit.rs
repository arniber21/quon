//! `SpecializedCircuit` — the Melior-free first-order gate DAG between
//! `elaborate` and `lower` (issue #206).
//!
//! Parametric specialization ([`elaborate`](crate::elaborate)) already produces
//! a first-order gate tree — `Compose` / `GateApp` / `Adjoint` over concrete
//! qubit indices and literal rotation angles — but the interface stayed surface
//! `Expr`, and the inverse / placement / `flatten_app` helpers were duplicated
//! across `elaborate.rs` and `lower.rs`. This module introduces the typed
//! boundary:
//!
//! - **Interface** ([`SpecializedCircuit`]): the elaborator's output and lower's
//!   only input — a gate DAG with resolved in/out widths, depth, and Clifford
//!   class. No classical parameters remain.
//! - **Implementation** ([`SpecializedCircuit::specialize`],
//!   [`SpecializedCircuit::adjoint`], [`collect_gate_placements`],
//!   [`reverse_and_invert`]): specialization, adjoint/inverse normalization, and
//!   placement — all Melior-free, all living once here.
//! - **Adapter** (in `lower.rs`): Melior builders that consume a
//!   `SpecializedCircuit` and emit `quantum.circ`. Nothing in this module
//!   imports `melior` or `mlir_bridge`.
//!
//! Reusing `elaborate`'s existing `Compose`/`GateApp`/`Adjoint` `Expr` tree
//! (ADR-0038) keeps churn minimal: the tree builder
//! [`elaborate::elaborate_circuit_body`] is unchanged; this module wraps its
//! output with the width/depth/Clifford metadata lower previously re-derived ad
//! hoc at each call site.

use std::collections::HashMap;

use chumsky::span::SimpleSpan;
use quon_core::DepthExpr;
use thiserror::Error;

use crate::ast::{CliffordClass, Expr, Stmt};
use crate::elaborate::{self, ElabCtx, ElabError, ParametricDef};
use crate::lexer::Sp;
use crate::types::Ty;

/// A placed gate: `(gate expression, qubit-target expression)`.
///
/// The gate is either a bare `Expr::Var(name)` (e.g. `H`, `CNOT`) or an
/// `Expr::App(Expr::Var(name), Expr::Float(angle))` rotation (`Rz(θ)`). The
/// qubit target is an `Expr::Int(i)` or `Expr::Tuple` of literal indices —
/// fully concrete by the time specialization runs.
pub type GatePlacement = (Sp<Expr>, Sp<Expr>);

/// Errors raised while building or normalizing a [`SpecializedCircuit`].
#[derive(Debug, Error)]
pub enum SpecializationError {
    /// Propagated from the elaborator (classical evaluation or body unfolding).
    #[error(transparent)]
    Elab(#[from] ElabError),
    /// The specialized value did not have the expected `Circuit<..>` shape
    /// (arity mismatch, non-circuit return, or a body that is not a gate
    /// sequence).
    #[error("{0}")]
    BadShape(&'static str),
    /// A width could not be reduced to a constant after substituting the
    /// call-site arguments (e.g. a symbolic `Nat` survived specialization).
    #[error("could not statically determine the qubit width of a specialized circuit")]
    UnresolvedWidth,
}

/// A fully specialized, monomorphic first-order gate DAG — the Melior-free
/// interface between `elaborate` and `lower`.
///
/// `body` is the elaborated gate tree (`Compose` / `GateApp` / `Adjoint` / empty
/// `CircuitBlock`); `in_qubits` / `out_qubits` / `depth` / `clifford` are the
/// resolved `Circuit<n,m,d,C>` indices after substituting the call-site
/// arguments. Constructed by [`SpecializedCircuit::specialize`] (a parametric
/// call site) or [`SpecializedCircuit::anonymous`] (a compound circuit
/// expression with no declared type); consumed by `lower`'s MLIR emission.
#[derive(Clone, Debug)]
pub struct SpecializedCircuit {
    /// The elaborated gate tree. No classical parameters remain.
    pub body: Sp<Expr>,
    pub in_qubits: i64,
    pub out_qubits: i64,
    pub depth: DepthExpr,
    pub clifford: bool,
}

impl SpecializedCircuit {
    /// Specialize the parametric circuit function `def` at concrete argument
    /// values `args` (each classically-evaluable under `classical_env`),
    /// returning the fully monomorphic gate DAG plus a canonical
    /// `"name(v0,v1,…)"` cache key for the caller's memoization.
    ///
    /// This is the pure (Melior-free) core of what `lower::specialize_named_fn`
    /// previously inlined: evaluate the classical arguments, elaborate the body
    /// under the resulting environment, and read the `Circuit<n,m,d,c>` indices
    /// off `def.ret_ty` after substituting the now-concrete widths. Lower does
    /// the memoization and MLIR emission around this.
    /// Canonical `"name(v0,v1,…)"` memoization key for a `(name, args)` call
    /// site, evaluating the classical arguments under `classical_env`. Computed
    /// before specialization so `lower` can short-circuit on a cache hit
    /// without re-elaborating the body.
    pub fn cache_key(
        name: &str,
        def: &ParametricDef,
        args: &[Sp<Expr>],
        classical_env: &HashMap<String, elaborate::Value>,
    ) -> Result<String, SpecializationError> {
        if def.params.len() != args.len() {
            return Err(SpecializationError::BadShape(
                "parametric circuit call arity mismatch",
            ));
        }
        let mut fuel = elaborate::fresh_fuel();
        let mut key = name.to_string();
        key.push('(');
        for (i, arg) in args.iter().enumerate() {
            let value = elaborate::eval_classical(arg, classical_env, &mut fuel)?;
            if i > 0 {
                key.push(',');
            }
            key.push_str(&format!("{value:?}"));
        }
        key.push(')');
        Ok(key)
    }

    /// Specialize the parametric circuit function `def` at concrete argument
    /// values `args` (each classically-evaluable under `classical_env`),
    /// returning the fully monomorphic gate DAG.
    ///
    /// This is the pure (Melior-free) core of what `lower::specialize_named_fn`
    /// previously inlined: evaluate the classical arguments, elaborate the body
    /// under the resulting environment, and read the `Circuit<n,m,d,c>` indices
    /// off `def.ret_ty` after substituting the now-concrete widths. Lower does
    /// the memoization (via [`cache_key`](Self::cache_key)) and MLIR emission
    /// around this.
    pub fn specialize(
        def: &ParametricDef,
        args: &[Sp<Expr>],
        classical_env: &HashMap<String, elaborate::Value>,
        ctx: &ElabCtx,
    ) -> Result<Self, SpecializationError> {
        if def.params.len() != args.len() {
            return Err(SpecializationError::BadShape(
                "parametric circuit call arity mismatch",
            ));
        }
        let mut fuel = elaborate::fresh_fuel();
        let mut callee_env: HashMap<String, elaborate::Value> = HashMap::new();
        for (param, arg) in def.params.iter().zip(args.iter()) {
            let value = elaborate::eval_classical(arg, classical_env, &mut fuel)?;
            callee_env.insert(param.clone(), value);
        }
        Self::from_def_and_env(def, &callee_env, ctx, &mut fuel)
    }

    /// Elaborate `def.body` under `callee_env` and read its `Circuit<..>`
    /// indices. Shared by [`specialize`](Self::specialize) and any caller that
    /// has already evaluated the arguments.
    fn from_def_and_env(
        def: &ParametricDef,
        callee_env: &HashMap<String, elaborate::Value>,
        ctx: &ElabCtx,
        fuel: &mut u32,
    ) -> Result<Self, SpecializationError> {
        let body = elaborate::elaborate_circuit_body(&def.body, callee_env, ctx, fuel)?;

        let Ty::Circuit { n, m, d, c } = def.ret_ty.clone() else {
            return Err(SpecializationError::BadShape(
                "specialized function does not return a Circuit",
            ));
        };
        let nat_env: HashMap<String, DepthExpr> = callee_env
            .iter()
            .filter_map(|(k, v)| match v {
                elaborate::Value::Int(i) if *i >= 0 => {
                    Some((k.clone(), DepthExpr::Nat(*i as u64)))
                }
                _ => None,
            })
            .collect();
        let in_qubits = n
            .subst(&nat_env)
            .as_const()
            .ok_or(SpecializationError::UnresolvedWidth)? as i64;
        let out_qubits = m
            .subst(&nat_env)
            .as_const()
            .ok_or(SpecializationError::UnresolvedWidth)? as i64;
        let depth = d.subst(&nat_env);
        let clifford = matches!(c, CliffordClass::Clifford);

        Ok(Self {
            body,
            in_qubits,
            out_qubits,
            depth,
            clifford,
        })
    }

    /// Wrap an already-elaborated gate tree as a width-`N` identity-shaped
    /// circuit, sizing `N` from the highest qubit index it places a gate on
    /// and depth from the gate count — the conservative estimate `lower`'s
    /// `resolve_circuit_callee` previously computed inline for an anonymous
    /// compound circuit expression (`hadamard_all(3) |> repeat(..)` etc.) with
    /// no declared `Circuit<n,..>` type of its own.
    pub fn anonymous(body: Sp<Expr>) -> Result<Self, SpecializationError> {
        let width = max_qubit_index(&body)
            .map(|max| max + 1)
            .ok_or(SpecializationError::UnresolvedWidth)? as i64;
        let gate_count = count_gates(&body);
        Ok(Self {
            body,
            in_qubits: width,
            out_qubits: width,
            depth: DepthExpr::Nat(gate_count as u64),
            clifford: false,
        })
    }

    /// The adjoint circuit: reverse gate order and invert each gate, swapping
    /// the in/out widths (`Circuit<n,m>† : Circuit<m,n>`). Depth and Clifford
    /// class are preserved. This is the typed adjoint normalization; the
    /// AST-level kernel is [`reverse_and_invert`].
    pub fn adjoint(&self) -> Result<Self, ElabError> {
        let body = reverse_and_invert(&self.body)?;
        Ok(Self {
            body,
            in_qubits: self.out_qubits,
            out_qubits: self.in_qubits,
            depth: self.depth.clone(),
            clifford: self.clifford,
        })
    }

    /// The flat gate placement list (left-to-right execution order), validating
    /// that `body` is a pure gate sequence.
    pub fn placements(&self) -> Result<Vec<GatePlacement>, ElabError> {
        collect_gate_placements(&self.body)
    }
}

// ── Shared helpers (Melior-free) ──────────────────────────────────────────────
//
// `inverse_gate_name`, `collect_gate_placements`, and `flatten_app` were
// previously duplicated across `elaborate.rs` and `lower.rs`; they now live once
// here. `max_qubit_index` / `count_gates` / `qubit_targets` / `literal_usize`
// move here from `lower.rs` for the same reason — none touch MLIR.

/// Canonical inverse gate name, falling back to `name` for self-inverse /
/// unrecognized gates. Single home for the `inverse_gate_name` previously
/// duplicated in `elaborate.rs` and `lower.rs`.
pub(crate) fn inverse_gate_name(name: &str) -> String {
    quon_core::gates::inverse_or_self(name)
}

/// Flatten a curried `App` chain `(f x1 x2 … xk)` into `(head, [x1, …, xk])`,
/// reading left-to-right. Shared by `elaborate`, `lower`, and this module.
pub(crate) fn flatten_app<'a>(
    f: &'a Sp<Expr>,
    x: &'a Sp<Expr>,
) -> (&'a Sp<Expr>, Vec<&'a Sp<Expr>>) {
    let mut args = vec![x];
    let mut head = f;
    while let Expr::App(inner_f, inner_x) = &head.0 {
        args.push(inner_x);
        head = inner_f;
    }
    args.reverse();
    (head, args)
}

/// Collect the flat gate placement list of a fully elaborated circuit
/// expression, left-to-right. `Compose` flattens recursively; a `CircuitBlock`
/// unwraps to its trailing expression (an empty block is the identity sentinel
/// and yields no placements); a `GateApp` is a single placement. Anything else
/// is not a gate sequence.
pub(crate) fn collect_gate_placements(
    expr: &Sp<Expr>,
) -> Result<Vec<GatePlacement>, ElabError> {
    match &expr.0 {
        Expr::CircuitBlock(stmts) => {
            let Some(Stmt::Expr(last)) = stmts.last().map(|s| &s.0) else {
                return Ok(Vec::new());
            };
            collect_gate_placements(last)
        }
        Expr::Compose(lhs, rhs) => {
            let mut out = collect_gate_placements(lhs)?;
            out.extend(collect_gate_placements(rhs)?);
            Ok(out)
        }
        Expr::GateApp { gate, qubits } => Ok(vec![(*gate.clone(), *qubits.clone())]),
        _ => Err(ElabError::unsupported(
            "adjoint of a non-gate-sequence circuit",
            expr.1,
        )),
    }
}

/// Build the adjoint of an already-elaborated, fully concrete circuit body:
/// reverse gate order and invert each gate. A named gate maps to its registry
/// inverse ([`inverse_gate_name`]); a rotation `Rz(θ)` maps to `Rz(-θ)` (the
/// adjoint negates the angle, not the gate name). This is the AST-level kernel
/// shared by [`SpecializedCircuit::adjoint`] and `elaborate`'s adjoint path.
pub(crate) fn reverse_and_invert(expr: &Sp<Expr>) -> Result<Sp<Expr>, ElabError> {
    if matches!(&expr.0, Expr::CircuitBlock(stmts) if stmts.is_empty()) {
        return Ok(expr.clone());
    }
    let placements = collect_gate_placements(expr)?;
    let span = expr.1;
    let mut result: Option<Sp<Expr>> = None;
    for (gate, qubits) in placements.into_iter().rev() {
        let inv_gate = match &gate.0 {
            Expr::Var(name) => (Expr::Var(inverse_gate_name(name)), gate.1),
            Expr::App(f, x) => {
                let (head, args) = flatten_app(f, x);
                let (Expr::Var(name), [angle]) = (&head.0, args.as_slice()) else {
                    return Err(ElabError::unsupported(
                        "adjoint of an unrecognized rotation gate",
                        gate.1,
                    ));
                };
                (
                    Expr::App(
                        Box::new((Expr::Var(name.clone()), head.1)),
                        Box::new(negate_angle(angle)),
                    ),
                    gate.1,
                )
            }
            _ => {
                return Err(ElabError::unsupported(
                    "adjoint of a non-primitive gate",
                    gate.1,
                ));
            }
        };
        let step = (
            Expr::GateApp {
                gate: Box::new(inv_gate),
                qubits: Box::new(qubits),
            },
            span,
        );
        result = Some(match result {
            None => step,
            Some(acc) => (Expr::Compose(Box::new(acc), Box::new(step)), span),
        });
    }
    result.ok_or(ElabError::unsupported("adjoint of an empty circuit", span))
}

/// Negate a literal rotation angle (`Rz(θ)† = Rz(-θ)`). The angle is already a
/// literal by the time adjoint normalization runs (elaboration evaluates
/// classical expressions fully).
fn negate_angle(angle: &Sp<Expr>) -> Sp<Expr> {
    let value = match &angle.0 {
        Expr::Float(f) => *f,
        Expr::Int(n) => *n as f64,
        _ => 0.0,
    };
    (Expr::Float(-value), angle.1)
}

/// The highest qubit index a fully elaborated circuit expression places a gate
/// on, or `None` if it places none.
pub(crate) fn max_qubit_index(expr: &Sp<Expr>) -> Option<usize> {
    match &expr.0 {
        Expr::Compose(lhs, rhs) => max_qubit_index(lhs)
            .into_iter()
            .chain(max_qubit_index(rhs))
            .max(),
        Expr::GateApp { qubits, .. } => qubit_targets(qubits).into_iter().max(),
        Expr::Adjoint(inner) => max_qubit_index(inner),
        _ => None,
    }
}

/// The number of gate placements in a fully elaborated circuit expression — a
/// crude but safe depth over-estimate for a synthesized anonymous function.
pub(crate) fn count_gates(expr: &Sp<Expr>) -> usize {
    match &expr.0 {
        Expr::Compose(lhs, rhs) => count_gates(lhs) + count_gates(rhs),
        Expr::GateApp { .. } => 1,
        Expr::Adjoint(inner) => count_gates(inner),
        _ => 0,
    }
}

/// Literal qubit indices an elaborated gate acts on.
pub(crate) fn qubit_targets(qubits: &Sp<Expr>) -> Vec<usize> {
    match &qubits.0 {
        Expr::Tuple(es) => es.iter().filter_map(|e| literal_usize(&e.0)).collect(),
        other => literal_usize(other).into_iter().collect(),
    }
}

fn literal_usize(expr: &Expr) -> Option<usize> {
    match expr {
        Expr::Int(n) if *n >= 0 => Some(*n as usize),
        _ => None,
    }
}

fn no_span() -> SimpleSpan {
    SimpleSpan::from(0..0)
}

// ── Tests ─────────────────────────────────────────────────────────────────────
//
// These exercise specialization, adjoint normalization, and placement purely
// through this Melior-free module — no `melior` / `mlir_bridge` imports. The
// module itself has no Melior dependency; `cargo build -p frontend
// --no-default-features --features analyze` compiles it without linking MLIR.

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn span() -> SimpleSpan {
        no_span()
    }

    fn int(n: i64) -> Sp<Expr> {
        (Expr::Int(n), span())
    }

    fn float(f: f64) -> Sp<Expr> {
        (Expr::Float(f), span())
    }

    fn var(name: &str) -> Sp<Expr> {
        (Expr::Var(name.to_string()), span())
    }

    /// `name @ q` (single-qubit gate).
    fn gate1(name: &str, q: i64) -> Sp<Expr> {
        (
            Expr::GateApp {
                gate: Box::new(var(name)),
                qubits: Box::new(int(q)),
            },
            span(),
        )
    }

    /// `name @ (q0, q1)` (two-qubit gate).
    fn gate2(name: &str, q0: i64, q1: i64) -> Sp<Expr> {
        (
            Expr::GateApp {
                gate: Box::new(var(name)),
                qubits: Box::new((Expr::Tuple(vec![int(q0), int(q1)]), span())),
            },
            span(),
        )
    }

    /// `Rz(angle) @ q`.
    fn rz(angle: f64, q: i64) -> Sp<Expr> {
        let rotation = (
            Expr::App(Box::new(var("Rz")), Box::new(float(angle))),
            span(),
        );
        (
            Expr::GateApp {
                gate: Box::new(rotation),
                qubits: Box::new(int(q)),
            },
            span(),
        )
    }

    fn compose(lhs: Sp<Expr>, rhs: Sp<Expr>) -> Sp<Expr> {
        (Expr::Compose(Box::new(lhs), Box::new(rhs)), span())
    }

    fn empty() -> Sp<Expr> {
        (Expr::CircuitBlock(Vec::new()), span())
    }

    // ── placement ──

    #[test]
    fn placements_flatten_compose() {
        // H@0 |> CNOT@(0,1) |> X@1  →  three placements, left-to-right.
        let tree = compose(compose(gate1("H", 0), gate2("CNOT", 0, 1)), gate1("X", 1));
        let ps = collect_gate_placements(&tree).expect("placements");
        assert_eq!(ps.len(), 3);
        assert!(matches!(&ps[0].0.0, Expr::Var(n) if n == "H"));
        assert!(matches!(&ps[1].0.0, Expr::Var(n) if n == "CNOT"));
        assert!(matches!(&ps[2].0.0, Expr::Var(n) if n == "X"));
    }

    #[test]
    fn placements_empty_circuit_is_identity() {
        assert!(collect_gate_placements(&empty()).expect("empty").is_empty());
    }

    #[test]
    fn placements_circuit_block_unwraps_trailing_expr() {
        // A non-empty circuit block unwraps to its trailing expression.
        let block = (
            Expr::CircuitBlock(vec![(Stmt::Expr(gate1("H", 0)), span())]),
            span(),
        );
        let ps = collect_gate_placements(&block).expect("placements");
        assert_eq!(ps.len(), 1);
    }

    #[test]
    fn placements_reject_non_gate() {
        let bad = var("H"); // a bare Var, not a GateApp
        assert!(collect_gate_placements(&bad).is_err());
    }

    // ── flatten_app ──

    #[test]
    fn flatten_app_curried_rotation() {
        // Rz(0.5) is App(Var("Rz"), Float(0.5)).
        let rz_expr = (Expr::App(Box::new(var("Rz")), Box::new(float(0.5))), span());
        let Expr::App(f, x) = &rz_expr.0 else {
            unreachable!("constructed as App");
        };
        let (head, args) = flatten_app(f, x);
        assert!(matches!(&head.0, Expr::Var(n) if n == "Rz"));
        assert_eq!(args.len(), 1);
        assert!(matches!(&args[0].0, Expr::Float(f) if (*f - 0.5).abs() < 1e-12));
    }

    // ── inverse / adjoint ──

    #[test]
    fn inverse_gate_name_resolves_pairs() {
        // Self-inverse gates map to themselves; S ↔ Sdg.
        assert_eq!(inverse_gate_name("H"), "H");
        assert_eq!(inverse_gate_name("X"), "X");
        assert_eq!(inverse_gate_name("CNOT"), "CNOT");
        assert_eq!(inverse_gate_name("S"), "S_dag");
        assert_eq!(inverse_gate_name("S_dag"), "S");
        assert_eq!(inverse_gate_name("Sdag"), "S");
        // Parametric rotations are self-inverse at the name level (angle negates).
        assert_eq!(inverse_gate_name("Rz"), "Rz");
    }

    #[test]
    fn adjoint_reverses_and_inverts_named_gates() {
        // (H@0 |> S@1)† = Sdag@1 |> H@0.
        let body = compose(gate1("H", 0), gate1("S", 1));
        let adj = reverse_and_invert(&body).expect("adjoint");
        let ps = collect_gate_placements(&adj).expect("placements");
        assert_eq!(ps.len(), 2);
        assert!(matches!(&ps[0].0.0, Expr::Var(n) if n == "S_dag"));
        assert!(matches!(&ps[1].0.0, Expr::Var(n) if n == "H"));
    }

    #[test]
    fn adjoint_negates_rotation_angle() {
        // (Rz(0.5)@0)† = Rz(-0.5)@0.
        let body = rz(0.5, 0);
        let adj = reverse_and_invert(&body).expect("adjoint");
        let ps = collect_gate_placements(&adj).expect("placements");
        assert_eq!(ps.len(), 1);
        let Expr::App(f, x) = &ps[0].0.0 else {
            panic!("expected a rotation App");
        };
        let (head, args) = flatten_app(f, x);
        assert!(matches!(&head.0, Expr::Var(n) if n == "Rz"));
        assert!(matches!(&args[0].0, Expr::Float(f) if (*f - (-0.5)).abs() < 1e-12));
    }

    #[test]
    fn adjoint_of_empty_is_empty() {
        let adj = reverse_and_invert(&empty()).expect("adjoint");
        assert!(matches!(&adj.0, Expr::CircuitBlock(s) if s.is_empty()));
    }

    #[test]
    fn double_adjoint_is_identity() {
        // adjoint is involutive: reverse(reverse(g)) = g, invert(invert(g)) = g.
        let bodies = [
            compose(gate1("H", 0), gate1("S", 1)),
            compose(rz(0.3, 0), gate2("CNOT", 0, 1)),
            compose(compose(gate1("T", 2), rz(1.2, 0)), gate1("H", 1)),
        ];
        for body in bodies {
            let once = reverse_and_invert(&body).expect("once");
            let twice = reverse_and_invert(&once).expect("twice");
            // `reverse_and_invert` rebuilds right-leaning, so the double adjoint
            // recovers the gate *sequence* (placements), not the tree shape.
            let pt = collect_gate_placements(&twice).expect("placements");
            let pb = collect_gate_placements(&body).expect("placements");
            assert_eq!(pt.len(), pb.len());
            for (x, y) in pt.iter().zip(pb.iter()) {
                assert_eq!(x.0.0, y.0.0, "gate mismatch");
                assert_eq!(x.1.0, y.1.0, "qubit mismatch");
            }
        }
    }

    // ── SpecializedCircuit::anonymous + adjoint ──

    #[test]
    fn anonymous_sizes_from_max_qubit_and_gate_count() {
        // H@0 |> CNOT@(1,2) → width 3, depth 2.
        let body = compose(gate1("H", 0), gate2("CNOT", 1, 2));
        let circ = SpecializedCircuit::anonymous(body).expect("anonymous");
        assert_eq!(circ.in_qubits, 3);
        assert_eq!(circ.out_qubits, 3);
        assert_eq!(circ.depth, DepthExpr::Nat(2));
        assert!(!circ.clifford);
    }

    #[test]
    fn anonymous_rejects_gateless() {
        // No gate placements → no width to read.
        assert!(SpecializedCircuit::anonymous(var("H")).is_err());
    }

    #[test]
    fn circuit_adjoint_swaps_widths_and_reverses() {
        let body = compose(gate1("H", 0), gate1("S", 1));
        let circ = SpecializedCircuit::anonymous(body).expect("anonymous");
        let adj = circ.adjoint().expect("adjoint");
        assert_eq!(adj.in_qubits, circ.out_qubits);
        assert_eq!(adj.out_qubits, circ.in_qubits);
        assert_eq!(adj.depth, circ.depth);
        let ps = adj.placements().expect("placements");
        assert_eq!(ps.len(), 2);
        assert!(matches!(&ps[0].0.0, Expr::Var(n) if n == "S_dag"));
        assert!(matches!(&ps[1].0.0, Expr::Var(n) if n == "H"));
    }

    // ── max_qubit_index / count_gates ──

    #[test]
    fn max_qubit_index_and_count() {
        let body = compose(compose(gate1("H", 0), gate2("CNOT", 2, 3)), gate1("X", 1));
        assert_eq!(max_qubit_index(&body), Some(3));
        assert_eq!(count_gates(&body), 3);
    }

    #[test]
    fn qubit_targets_single_and_tuple() {
        assert_eq!(qubit_targets(&int(2)), vec![2]);
        assert_eq!(
            qubit_targets(&(Expr::Tuple(vec![int(0), int(3)]), span())),
            vec![0, 3]
        );
    }

    // ── proptest: collect_gate_placements is associative over Compose ──

    fn arb_gate() -> impl Strategy<Value = Sp<Expr>> {
        prop_oneof![
            (0i64..4).prop_map(|q| gate1("H", q)),
            (0i64..4, 0i64..4).prop_map(|(a, b)| gate2("CNOT", a, b)),
            (0f64..6.28, 0i64..4).prop_map(|(a, q)| rz(a, q)),
        ]
    }

    fn arb_tree(depth: u32) -> impl Strategy<Value = Sp<Expr>> {
        if depth == 0 {
            arb_gate().boxed()
        } else {
            prop_oneof![
                arb_gate().boxed(),
                (arb_tree(depth - 1), arb_tree(depth - 1))
                    .prop_map(|(l, r)| compose(l, r)),
            ]
            .boxed()
        }
    }

    proptest! {
        #[test]
        fn placements_associative(a in arb_tree(2), b in arb_tree(2), c in arb_tree(2)) {
            let left = compose(compose(a.clone(), b.clone()), c.clone());
            let right = compose(a, compose(b, c));
            let pl = collect_gate_placements(&left).expect("placements");
            let pr = collect_gate_placements(&right).expect("placements");
            prop_assert_eq!(pl.len(), pr.len());
            // Same gate names and qubit targets in order.
            for (x, y) in pl.iter().zip(pr.iter()) {
                prop_assert_eq!(&x.0.0, &y.0.0);
                prop_assert_eq!(&x.1.0, &y.1.0);
            }
        }

        #[test]
        fn double_adjoint_recovers_original(body in arb_tree(3)) {
            // `reverse_and_invert` rebuilds right-leaning, so the double adjoint
            // recovers the gate *sequence* (placements), not the tree shape.
            if let Ok(once) = reverse_and_invert(&body) {
                if let Ok(twice) = reverse_and_invert(&once) {
                    let pt = collect_gate_placements(&twice).expect("placements");
                    let pb = collect_gate_placements(&body).expect("placements");
                    prop_assert_eq!(pt.len(), pb.len());
                    for (x, y) in pt.iter().zip(pb.iter()) {
                        prop_assert_eq!(&x.0.0, &y.0.0);
                        prop_assert_eq!(&x.1.0, &y.1.0);
                    }
                }
            }
        }
    }
}
