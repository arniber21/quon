//! Parametric circuit elaboration — partial evaluation of `Nat`/`Int`/`Float`
//! classical parameters into a fully monomorphic, first-order circuit body
//! (issue #1, MVP milestone M2).
//!
//! `lower.rs` already knows how to lower a *zero-parameter* circuit function
//! body — a tree of `Compose`/`GateApp`/`Adjoint`/zero-arg `App`/`Var` nodes —
//! directly to `quantum.circ` MLIR. This module bridges the gap for a
//! *parametric* circuit function called with concrete arguments (e.g.
//! `hadamard_all(3)`): it evaluates the classical fragment of the language
//! (arithmetic, `let`, `if`, `match` on `Nat`, the builtins `range`/`qubits`/
//! `diag`/`sqrt`/`round`/`float`/`PI`/`TAU`/`E`) and *unrolls* the circuit
//! fragment's parametric forms (`for pat in iter { .. }`, `repeat(k, c)`, and
//! calls to other parametric circuit functions) into that same zero-parameter
//! shape, which the existing lowering path then handles unchanged.
//!
//! Scope (see `docs/plans/mvp-landing-plan.md` §5): `for` over `qubits`/
//! `range`/`diag`/`pairs`, `repeat` with a computed count, recursive circuit
//! functions via `match n { 0 => .., _ => .. }` (including as a *bare* body,
//! not just inside `circuit { }` — `qft`'s own definition), `identity`,
//! `on_high`/`on_low`, `swap_reverse`, and nested parametric calls are
//! supported. `Rzz(theta) @ (i, j)` and `controlled(c) @ (control, target)`
//! have no native gate of their own, so they are rewritten here into existing
//! primitives (`CNOT`/`CZ`/`CY`/`Rz`/…) — see `decompose_rzz`/
//! `decompose_controlled`. Control distributes over `|>` / `par` / circuit
//! blocks; Clifford+T single-qubit generators (and `Rx`/`Ry`/`Rz`) lower via
//! known decompositions. `tensored`/`split`, `fold` over a circuit accumulator,
//! and residual unsupported controlled bodies are rejected with
//! [`ElabError::Unsupported`] (span-accurate), not silently miscompiled.

use std::collections::HashMap;

use chumsky::span::SimpleSpan;
use quon_core::DepthExpr;
use thiserror::Error;

use crate::ast::{BinOp, Expr, LitPat, Pat, Stmt};
use crate::lexer::Sp;
use crate::specialized_circuit::{flatten_app, reverse_and_invert};
use crate::typecheck::circuit;

#[derive(Debug, Error)]
pub enum ElabError {
    #[error("elaboration is not implemented for `{construct}`")]
    Unsupported {
        construct: &'static str,
        span: SimpleSpan,
    },
    #[error("`{name}` is not a classically evaluable expression")]
    NotClassical { name: &'static str },
    #[error("unbound variable `{name}` during elaboration")]
    UnboundVar { name: String },
    #[error("elaboration exceeded its evaluation budget (possible non-terminating recursion)")]
    FuelExhausted,
}

impl ElabError {
    pub(crate) fn unsupported(construct: &'static str, span: SimpleSpan) -> Self {
        Self::Unsupported { construct, span }
    }

    /// Source span for diagnostics (controlled / elaboration failures).
    pub fn span(&self) -> SimpleSpan {
        match self {
            Self::Unsupported { span, .. } => *span,
            _ => SimpleSpan::from(0..0),
        }
    }
}

/// A classically-evaluated value: the elaborator's evaluation domain for the
/// non-quantum fragment of the language.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Int(i64),
    Float(f64),
    Bool(bool),
    List(Vec<Value>),
    Tuple(Vec<Value>),
    Unit,
}

impl Value {
    fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(n) => Some(*n),
            _ => None,
        }
    }

    fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Int(n) => Some(*n as f64),
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }

    fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
}

/// A recorded parametric circuit function definition, resolved once from the
/// declaration list before any call site is elaborated. `ret_ty` is the
/// checker-resolved `Ty::Circuit { n, m, d, c }` return type — carried here
/// rather than re-derived from the AST at each specialization, since the type
/// checker has already validated it.
#[derive(Clone)]
pub struct ParametricDef {
    pub params: Vec<String>,
    pub body: Sp<Expr>,
    pub ret_ty: crate::types::Ty,
}

/// Read-only context threaded through elaboration: every parametric circuit
/// function's definition, keyed by name, so a call site can look up and
/// recursively specialize its callee.
pub struct ElabCtx {
    pub parametric: HashMap<String, ParametricDef>,
}

type ClassicalEnv = HashMap<String, Value>;

const FUEL_START: u32 = 1_000_000;

fn no_span() -> SimpleSpan {
    SimpleSpan::from(0..0)
}

/// Evaluates a classical (non-quantum) expression to a [`Value`] under `env`.
///
/// Total for the supported fragment: recursion in the source language is only
/// ever over a structurally decreasing `Nat` (already proved terminating by
/// the type checker's `check_termination`, `frontend/src/typecheck/obligation.rs`),
/// `fold`/`for` ranges are finite lists, and `fuel` is a defensive backstop
/// against a bug in that reasoning surfacing here as an infinite recursion
/// instead of a clean diagnostic.
pub fn eval_classical(
    expr: &Sp<Expr>,
    env: &ClassicalEnv,
    fuel: &mut u32,
) -> Result<Value, ElabError> {
    *fuel = fuel.checked_sub(1).ok_or(ElabError::FuelExhausted)?;
    match &expr.0 {
        Expr::Int(n) => Ok(Value::Int(*n)),
        Expr::Float(f) => Ok(Value::Float(*f)),
        Expr::Bool(b) => Ok(Value::Bool(*b)),
        Expr::Unit => Ok(Value::Unit),
        Expr::Var(name) => {
            if let Some(value) = env.get(name) {
                return Ok(value.clone());
            }
            match name.as_str() {
                "PI" => Ok(Value::Float(std::f64::consts::PI)),
                "TAU" => Ok(Value::Float(std::f64::consts::TAU)),
                "E" => Ok(Value::Float(std::f64::consts::E)),
                _ => Err(ElabError::UnboundVar { name: name.clone() }),
            }
        }
        Expr::Neg(inner) => match eval_classical(inner, env, fuel)? {
            Value::Int(n) => Ok(Value::Int(-n)),
            Value::Float(f) => Ok(Value::Float(-f)),
            _ => Err(ElabError::NotClassical {
                name: "negation of a non-numeric value",
            }),
        },
        Expr::BinOp { op, lhs, rhs } => {
            let a = eval_classical(lhs, env, fuel)?;
            let b = eval_classical(rhs, env, fuel)?;
            eval_binop(*op, &a, &b)
        }
        Expr::Tuple(items) => Ok(Value::Tuple(
            items
                .iter()
                .map(|item| eval_classical(item, env, fuel))
                .collect::<Result<_, _>>()?,
        )),
        Expr::List(items) => Ok(Value::List(
            items
                .iter()
                .map(|item| eval_classical(item, env, fuel))
                .collect::<Result<_, _>>()?,
        )),
        Expr::Let { pat, rhs, body } => {
            let value = eval_classical(rhs, env, fuel)?;
            let mut inner = env.clone();
            bind_classical_pat(pat, value, &mut inner)?;
            eval_classical(body, &inner, fuel)
        }
        Expr::If { cond, then, else_ } => {
            let value =
                eval_classical(cond, env, fuel)?
                    .as_bool()
                    .ok_or(ElabError::NotClassical {
                        name: "if condition",
                    })?;
            eval_classical(if value { then } else { else_ }, env, fuel)
        }
        Expr::Match { scrutinee, arms } => {
            let value = eval_classical(scrutinee, env, fuel)?;
            let scrutinee_int = value.as_i64();
            for (pat, body) in arms {
                match &pat.0 {
                    Pat::Lit(LitPat::Int(k)) if scrutinee_int == Some(*k) => {
                        return eval_classical(body, env, fuel);
                    }
                    Pat::Lit(LitPat::Bool(b)) if value.as_bool() == Some(*b) => {
                        return eval_classical(body, env, fuel);
                    }
                    Pat::Lit(_) => continue,
                    Pat::Wildcard => return eval_classical(body, env, fuel),
                    Pat::Var(name) => {
                        let mut inner = env.clone();
                        inner.insert(name.clone(), value);
                        return eval_classical(body, &inner, fuel);
                    }
                    Pat::Tuple(_) => {
                        return Err(ElabError::unsupported(
                            "tuple pattern in classical match",
                            no_span(),
                        ));
                    }
                }
            }
            Err(ElabError::unsupported(
                "non-exhaustive classical match",
                no_span(),
            ))
        }
        Expr::App(f, x) => {
            let (head, args) = flatten_app(f, x);
            if let Expr::Var(name) = &head.0 {
                let values = args
                    .iter()
                    .map(|arg| eval_classical(arg, env, fuel))
                    .collect::<Result<Vec<_>, _>>()?;
                if let Some(result) = eval_builtin(name, &values)? {
                    return Ok(result);
                }
            }
            Err(ElabError::unsupported("classical function call", no_span()))
        }
        _ => Err(ElabError::unsupported("classical expression", no_span())),
    }
}

fn eval_binop(op: BinOp, a: &Value, b: &Value) -> Result<Value, ElabError> {
    if let (Value::Int(x), Value::Int(y)) = (a, b) {
        return Ok(match op {
            BinOp::Add => Value::Int(x + y),
            BinOp::Sub => Value::Int(x - y),
            BinOp::Mul => Value::Int(x * y),
            BinOp::Div => Value::Int(x / y),
            BinOp::Pow => Value::Int(x.pow(u32::try_from(*y).unwrap_or(0))),
        });
    }
    let (x, y) = (
        a.as_f64().ok_or(ElabError::NotClassical {
            name: "binary operand",
        })?,
        b.as_f64().ok_or(ElabError::NotClassical {
            name: "binary operand",
        })?,
    );
    Ok(Value::Float(match op {
        BinOp::Add => x + y,
        BinOp::Sub => x - y,
        BinOp::Mul => x * y,
        BinOp::Div => x / y,
        BinOp::Pow => x.powf(y),
    }))
}

fn eval_builtin(name: &str, args: &[Value]) -> Result<Option<Value>, ElabError> {
    Ok(match (name, args) {
        ("range" | "qubits" | "diag", [n]) => {
            let count = n.as_i64().ok_or(ElabError::NotClassical {
                name: "range/qubits/diag count",
            })?;
            Some(Value::List((0..count).map(Value::Int).collect()))
        }
        // All unique unordered index pairs `(i, j)`, `0 <= i < j < n` — the
        // canonical choice for a symmetric cost matrix (QAOA's `cost_layer`:
        // applying both `(i,j)` and `(j,i)` would double the interaction).
        ("pairs", [n]) => {
            let count = n.as_i64().ok_or(ElabError::NotClassical {
                name: "pairs count",
            })?;
            let mut out = Vec::new();
            for i in 0..count {
                for j in (i + 1)..count {
                    out.push(Value::Tuple(vec![Value::Int(i), Value::Int(j)]));
                }
            }
            Some(Value::List(out))
        }
        ("sqrt", [x]) => Some(Value::Float(
            x.as_f64()
                .ok_or(ElabError::NotClassical {
                    name: "sqrt argument",
                })?
                .sqrt(),
        )),
        ("round", [x]) => Some(Value::Int(
            x.as_f64()
                .ok_or(ElabError::NotClassical {
                    name: "round argument",
                })?
                .round() as i64,
        )),
        ("float", [x]) => Some(Value::Float(x.as_f64().ok_or(ElabError::NotClassical {
            name: "float argument",
        })?)),
        _ => None,
    })
}

fn bind_classical_pat(
    pat: &Sp<Pat>,
    value: Value,
    env: &mut ClassicalEnv,
) -> Result<(), ElabError> {
    match &pat.0 {
        Pat::Var(name) => {
            env.insert(name.clone(), value);
            Ok(())
        }
        Pat::Wildcard => Ok(()),
        Pat::Tuple(pats) => {
            let Value::Tuple(values) = value else {
                return Err(ElabError::unsupported(
                    "tuple pattern bound to a non-tuple value",
                    no_span(),
                ));
            };
            if pats.len() != values.len() {
                return Err(ElabError::unsupported(
                    "tuple pattern arity mismatch",
                    no_span(),
                ));
            }
            for (p, v) in pats.iter().zip(values) {
                bind_classical_pat(p, v, env)?;
            }
            Ok(())
        }
        Pat::Lit(_) => Err(ElabError::unsupported(
            "literal pattern in classical let",
            no_span(),
        )),
    }
}

/// Elaborates a circuit-body expression under `classical_env` into a fully
/// concrete tree of `Compose`/`GateApp`/`Adjoint`/zero-arg `App`/`Var` nodes —
/// the shape `lower.rs`'s existing zero-parameter walker already knows how to
/// emit as MLIR. Every span in the returned tree is copied from the innermost
/// source node it was built from, so lowering errors still point somewhere
/// meaningful.
pub fn elaborate_circuit_body(
    expr: &Sp<Expr>,
    classical_env: &ClassicalEnv,
    ctx: &ElabCtx,
    fuel: &mut u32,
) -> Result<Sp<Expr>, ElabError> {
    *fuel = fuel.checked_sub(1).ok_or(ElabError::FuelExhausted)?;
    let span = expr.1;
    match &expr.0 {
        Expr::CircuitBlock(stmts) => {
            let Some((last, leading)) = stmts.split_last() else {
                return Err(ElabError::unsupported("empty circuit block", no_span()));
            };
            let mut inner_env = classical_env.clone();
            for stmt in leading {
                let crate::ast::Stmt::Let { pat, rhs } = &stmt.0 else {
                    return Err(ElabError::unsupported(
                        "non-let statement inside a circuit block",
                        no_span(),
                    ));
                };
                // A `let` inside a `circuit { }` block binds a classical
                // value (e.g. a per-iteration angle); a local bound to
                // another *circuit* expression is out of scope for this
                // elaborator (`lower.rs`'s zero-parameter walker supports it
                // directly — see `lower_circuit_block`'s `locals` map — but
                // no fixture elaborated so far needs it here).
                let value = eval_classical(rhs, &inner_env, fuel)?;
                bind_classical_pat(pat, value, &mut inner_env)?;
            }
            let crate::ast::Stmt::Expr(last_expr) = &last.0 else {
                return Err(ElabError::unsupported(
                    "circuit block not ending in an expression",
                    no_span(),
                ));
            };
            elaborate_circuit_body(last_expr, &inner_env, ctx, fuel)
        }
        Expr::Compose(lhs, rhs) => {
            let l = elaborate_circuit_body(lhs, classical_env, ctx, fuel)?;
            let r = elaborate_circuit_body(rhs, classical_env, ctx, fuel)?;
            // `identity(0)` (recursion's base case, e.g. `qft`'s `match n { 0
            // => identity(0), .. }`) elaborates to the empty-circuit sentinel
            // (see `empty_circuit`) — composing with it is a no-op, and
            // `Expr::Compose` has no way to represent "the other side, with
            // nothing on this side" directly.
            if is_empty_circuit(&l) {
                return Ok(r);
            }
            if is_empty_circuit(&r) {
                return Ok(l);
            }
            Ok((Expr::Compose(Box::new(l), Box::new(r)), span))
        }
        Expr::Match { scrutinee, arms } => {
            let value = eval_classical(scrutinee, classical_env, fuel)?;
            let scrutinee_int = value.as_i64();
            for (pat, body) in arms {
                match &pat.0 {
                    Pat::Lit(LitPat::Int(k)) if scrutinee_int == Some(*k) => {
                        return elaborate_circuit_body(body, classical_env, ctx, fuel);
                    }
                    Pat::Lit(_) => continue,
                    // A wildcard/var arm binds nothing new for the *value* —
                    // per the type checker's `push_arm_refinement`
                    // (`frontend/src/typecheck/obligation.rs`), only a proof-context
                    // refinement (e.g. `n != 0`) is added there; `n` itself
                    // stays the same outer binding, so `Var` re-binds it to
                    // its own already-known value (a no-op) rather than
                    // shadowing it with something new.
                    Pat::Wildcard => return elaborate_circuit_body(body, classical_env, ctx, fuel),
                    Pat::Var(name) => {
                        let mut inner = classical_env.clone();
                        inner.insert(name.clone(), value);
                        return elaborate_circuit_body(body, &inner, ctx, fuel);
                    }
                    Pat::Tuple(_) => {
                        return Err(ElabError::unsupported(
                            "tuple pattern in circuit-body match",
                            no_span(),
                        ));
                    }
                }
            }
            Err(ElabError::unsupported(
                "non-exhaustive circuit-body match",
                no_span(),
            ))
        }
        // A bare `let x = e in body` as a circuit function's whole
        // definition (e.g. `ising_evolve`'s `let tau = t / float(n_steps) in
        // repeat(..)`) — as opposed to a `let` *statement* inside a
        // `circuit { }` block, handled by the `CircuitBlock` arm above. `e`
        // is always classical (a circuit can't be `let`-bound this way,
        // since `body` is the *only* circuit-valued expression here); `body`
        // is circuit-valued.
        Expr::Let { pat, rhs, body } => {
            let value = eval_classical(rhs, classical_env, fuel)?;
            let mut inner_env = classical_env.clone();
            bind_classical_pat(pat, value, &mut inner_env)?;
            elaborate_circuit_body(body, &inner_env, ctx, fuel)
        }
        Expr::GateApp { gate, qubits } => {
            let qubits = subst_classical_vars(qubits, classical_env)?;
            // `Rzz`/`controlled(..)` have no native `quantum.circ`/QASM
            // representation of their own — rather than thread a new gate
            // primitive through the dialect, decompose, and emitter, they are
            // rewritten here into the existing CNOT/Rz primitives every other
            // gate already lowers through (see the doc comments on
            // `decompose_rzz`/`decompose_controlled_rz` for the identities).
            if let Expr::App(f, x) = &gate.0 {
                let (head, args) = flatten_app(f, x);
                if let Expr::Var(name) = &head.0
                    && name == "Rzz"
                    && args.len() == 1
                {
                    let angle = subst_classical_vars(args[0], classical_env)?;
                    let (q0, q1) = tuple2(&qubits)?;
                    return Ok(decompose_rzz(&angle, &q0, &q1, span));
                }
            }
            if let Expr::Controlled(inner) = &gate.0 {
                let (control, target) = tuple2(&qubits)?;
                return decompose_controlled(inner, &control, &target, classical_env, span);
            }
            let gate = subst_classical_vars(gate, classical_env)?;
            Ok((
                Expr::GateApp {
                    gate: Box::new(gate),
                    qubits: Box::new(qubits),
                },
                span,
            ))
        }
        Expr::Adjoint(inner) => {
            let elaborated = elaborate_circuit_body(inner, classical_env, ctx, fuel)?;
            reverse_and_invert(&elaborated)
        }
        Expr::For { pat, iter, body } => {
            let items = eval_classical(iter, classical_env, fuel)?;
            let Value::List(items) = items else {
                return Err(ElabError::unsupported(
                    "for-loop iterator (expected qubits/range/diag)",
                    no_span(),
                ));
            };
            // A zero-iteration loop (e.g. `controlled_rotations(1)`'s `for i
            // in range(n - 1)` at `n = 1`, a real case in a recursive
            // circuit function's base case) elaborates to the empty-circuit
            // sentinel, not an error — see `empty_circuit`.
            let mut composed = empty_circuit(span);
            for item in items {
                let mut inner_env = classical_env.clone();
                bind_classical_pat(pat, item, &mut inner_env)?;
                let step = elaborate_circuit_body(body, &inner_env, ctx, fuel)?;
                composed = if is_empty_circuit(&composed) {
                    step
                } else {
                    (Expr::Compose(Box::new(composed), Box::new(step)), span)
                };
            }
            Ok(composed)
        }
        Expr::App(f, x) => elaborate_app(&expr.0, span, f, x, classical_env, ctx, fuel),
        Expr::Var(name) => {
            // A reference to a zero-arg circuit function or a `let`-bound
            // local circuit value: not classically substitutable (it names a
            // circuit, not a number), so it passes through unchanged for
            // `lower.rs`'s existing `Var` handling (locals map / `self.bodies`).
            let _ = name;
            Ok(expr.clone())
        }
        _ => Err(ElabError::unsupported(
            "circuit body expression during elaboration",
            no_span(),
        )),
    }
}

fn elaborate_app(
    original: &Expr,
    span: chumsky::span::SimpleSpan,
    f: &Sp<Expr>,
    x: &Sp<Expr>,
    classical_env: &ClassicalEnv,
    ctx: &ElabCtx,
    fuel: &mut u32,
) -> Result<Sp<Expr>, ElabError> {
    let (head, args) = flatten_app(f, x);
    if let Expr::Var(name) = &head.0 {
        // Bare gate-name juxtaposition (`H q`, `Rz(theta) q`) is sugar for
        // `GateApp` — `q` is placed on the ambient register the same as
        // `H @ q` would be (the type checker's `place_gate`,
        // `frontend/src/typecheck/mod.rs`, resolves both identically). A
        // rotation's *angle* argument sits between the gate name and the
        // qubit target(s), so its arity is one more than the gate's own.
        if let Some(qubit_arity) = circuit::rotation_arity(name)
            && args.len() == qubit_arity as usize + 1
        {
            let angle = subst_classical_vars(args[0], classical_env)?;
            let qubits = subst_classical_vars(args[args.len() - 1], classical_env)?;
            let gate = (Expr::App(Box::new(head.clone()), Box::new(angle)), span);
            return Ok((
                Expr::GateApp {
                    gate: Box::new(gate),
                    qubits: Box::new(qubits),
                },
                span,
            ));
        }
        if circuit::gate_type(name).is_some()
            && circuit::rotation_arity(name).is_none()
            && args.len() == 1
            && !matches!(args[0].0, Expr::Unit)
        {
            let qubits = subst_classical_vars(args[0], classical_env)?;
            return Ok((
                Expr::GateApp {
                    gate: Box::new(head.clone()),
                    qubits: Box::new(qubits),
                },
                span,
            ));
        }
        match (name.as_str(), args.len()) {
            ("repeat", 2) => {
                let count = eval_classical(args[0], classical_env, fuel)?
                    .as_i64()
                    .ok_or(ElabError::NotClassical {
                        name: "repeat count",
                    })?;
                let body = elaborate_circuit_body(args[1], classical_env, ctx, fuel)?;
                let mut composed = empty_circuit(span);
                for _ in 0..count {
                    composed = if is_empty_circuit(&composed) {
                        body.clone()
                    } else {
                        (
                            Expr::Compose(Box::new(composed), Box::new(body.clone())),
                            span,
                        )
                    };
                }
                return Ok(composed);
            }
            // `identity(n)` (SPEC §5.7): the base case of a recursive circuit
            // function (`qft`'s `match n { 0 => identity(0), .. }`) and
            // `fold`'s starting accumulator. Elaborates to the empty-circuit
            // sentinel regardless of `n` — this elaborator tracks a
            // specialization's width from its declared `Circuit<n,m,d,C>`
            // type (`on_high_width`), not by re-deriving it from the gate
            // sequence, so `n` itself is only needed for type-checking
            // (already done) and is not otherwise consulted here.
            ("identity", 1) => return Ok(empty_circuit(span)),
            // `c `on_high` n` (SPEC §5): embeds `c` (of some width `k`) into
            // the *high* `k` qubits of an `n`-qubit register — i.e. shift
            // every qubit index `c` places by `n - k`, leaving qubits
            // `0..n-k` untouched. `k` comes from `c`'s own declared
            // `Circuit<k,k,..>` type (`on_high_width`), which is exact even
            // when `c` elaborates to the empty sentinel (`identity(0)`, the
            // recursive base case, whose declared width is still `0`).
            ("on_high" | "on_low", 2) => {
                let target_width = eval_classical(args[1], classical_env, fuel)?
                    .as_i64()
                    .ok_or(ElabError::NotClassical {
                        name: "on_high/on_low register width",
                    })?;
                let inner_width = on_high_width(args[0], classical_env, ctx, fuel)?;
                let elaborated = elaborate_circuit_body(args[0], classical_env, ctx, fuel)?;
                let offset = if name == "on_high" {
                    target_width - inner_width
                } else {
                    0
                };
                return Ok(shift_qubits(&elaborated, offset));
            }
            // `swap_reverse(n)` (SPEC §5): the bit-reversal permutation QFT's
            // recursive definition applies to its output register —
            // `SWAP(i, n-1-i)` for each `i < n/2`.
            ("swap_reverse", 1) => {
                let n = eval_classical(args[0], classical_env, fuel)?
                    .as_i64()
                    .ok_or(ElabError::NotClassical {
                        name: "swap_reverse width",
                    })?;
                let mut composed = empty_circuit(span);
                for i in 0..(n / 2) {
                    let step = gate_app(
                        "SWAP",
                        &(
                            Expr::Tuple(vec![(Expr::Int(i), span), (Expr::Int(n - 1 - i), span)]),
                            span,
                        ),
                        span,
                    );
                    composed = if is_empty_circuit(&composed) {
                        step
                    } else {
                        (Expr::Compose(Box::new(composed), Box::new(step)), span)
                    };
                }
                return Ok(composed);
            }
            _ => {
                if let Some(def) = ctx.parametric.get(name) {
                    if def.params.len() != args.len() {
                        return Err(ElabError::unsupported(
                            "parametric circuit call arity mismatch",
                            no_span(),
                        ));
                    }
                    let mut callee_env = ClassicalEnv::new();
                    for (param, arg) in def.params.iter().zip(args.iter()) {
                        let value = eval_classical(arg, classical_env, fuel)?;
                        callee_env.insert(param.clone(), value);
                    }
                    return elaborate_circuit_body(&def.body, &callee_env, ctx, fuel);
                }
            }
        }
    }
    // Not a parametric construct this elaborator understands (e.g. a
    // zero-arg call to a plain circuit function) — substitute any classical
    // variables in the argument list (there may be none) and pass through
    // unchanged for `lower.rs`'s existing zero-arg `App` handling.
    let _ = original;
    subst_classical_vars(&(original.clone(), span), classical_env)
}

/// Renders an evaluated classical [`Value`] back into surface `Expr` syntax,
/// e.g. so a `for`-loop's `q` can be substituted with a literal qubit index.
/// Every node in the result is given `span` — the substitution site's own
/// span — since a `Value` carries no source location of its own.
fn value_to_expr(value: &Value, span: chumsky::span::SimpleSpan) -> Expr {
    match value {
        Value::Int(n) => Expr::Int(*n),
        Value::Float(f) => Expr::Float(*f),
        Value::Bool(b) => Expr::Bool(*b),
        Value::Unit => Expr::Unit,
        Value::Tuple(items) => Expr::Tuple(
            items
                .iter()
                .map(|item| (value_to_expr(item, span), span))
                .collect(),
        ),
        Value::List(items) => Expr::List(
            items
                .iter()
                .map(|item| (value_to_expr(item, span), span))
                .collect(),
        ),
    }
}

/// Substitutes every classically-bound `Var` in `expr` with its literal
/// value. Used for gate/angle/qubit-index expressions that may reference an
/// enclosing `for`-loop variable or a specialized circuit function's own
/// parameter (e.g. `Rz(gamma * tau)` or `@(i, i + 1)`), without attempting to
/// evaluate the *whole* expression (`H` itself is not a classical value).
/// Reduces `expr` to a literal at a qubit-index/rotation-angle position.
///
/// A qubit target or angle is frequently *arithmetic over* a bound variable
/// (`i + 1`, `gamma * q[i][j]`), not the variable itself — bare structural
/// substitution (replacing only `Var` nodes) would leave a `BinOp` node where
/// `lower.rs` requires a literal `Int`/`Float` (`qubit_targets`/`gate_spec`
/// both match on the literal variant exactly). So this tries full classical
/// evaluation first; only an expression `eval_classical` cannot handle (a
/// bare gate/circuit name like `H`, which is not a classical value) falls
/// back to structural substitution, which leaves such names untouched.
fn subst_classical_vars(expr: &Sp<Expr>, env: &ClassicalEnv) -> Result<Sp<Expr>, ElabError> {
    let mut probe_fuel = 10_000;
    if let Ok(value) = eval_classical(expr, env, &mut probe_fuel) {
        return Ok((value_to_expr(&value, expr.1), expr.1));
    }
    let span = expr.1;
    match &expr.0 {
        Expr::Var(_) | Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unit => {
            Ok(expr.clone())
        }
        Expr::Neg(inner) => Ok((Expr::Neg(Box::new(subst_classical_vars(inner, env)?)), span)),
        Expr::BinOp { op, lhs, rhs } => Ok((
            Expr::BinOp {
                op: *op,
                lhs: Box::new(subst_classical_vars(lhs, env)?),
                rhs: Box::new(subst_classical_vars(rhs, env)?),
            },
            span,
        )),
        Expr::Tuple(items) => Ok((
            Expr::Tuple(
                items
                    .iter()
                    .map(|item| subst_classical_vars(item, env))
                    .collect::<Result<_, _>>()?,
            ),
            span,
        )),
        Expr::App(f, x) => Ok((
            Expr::App(
                Box::new(subst_classical_vars(f, env)?),
                Box::new(subst_classical_vars(x, env)?),
            ),
            span,
        )),
        Expr::Compose(lhs, rhs) => Ok((
            Expr::Compose(
                Box::new(subst_classical_vars(lhs, env)?),
                Box::new(subst_classical_vars(rhs, env)?),
            ),
            span,
        )),
        Expr::Par(body, count) => Ok((
            Expr::Par(
                Box::new(subst_classical_vars(body, env)?),
                Box::new(subst_classical_vars(count, env)?),
            ),
            span,
        )),
        Expr::Adjoint(inner) => Ok((
            Expr::Adjoint(Box::new(subst_classical_vars(inner, env)?)),
            span,
        )),
        Expr::Controlled(inner) => Ok((
            Expr::Controlled(Box::new(subst_classical_vars(inner, env)?)),
            span,
        )),
        Expr::GateApp { gate, qubits } => Ok((
            Expr::GateApp {
                gate: Box::new(subst_classical_vars(gate, env)?),
                qubits: Box::new(subst_classical_vars(qubits, env)?),
            },
            span,
        )),
        _ => Ok(expr.clone()),
    }
}

/// The elaborated form of `identity(n)` for any `n` (SPEC §5.7's zero-gate
/// circuit): an empty `CircuitBlock` is never otherwise produced by this
/// elaborator (every other path immediately unwraps a source `CircuitBlock`
/// via its match arm above), so it is a safe, easily-recognized sentinel for
/// "no gates" — needed because `Expr::Compose` has no way to represent one
/// empty side directly.
fn empty_circuit(span: chumsky::span::SimpleSpan) -> Sp<Expr> {
    (Expr::CircuitBlock(Vec::new()), span)
}

fn is_empty_circuit(expr: &Sp<Expr>) -> bool {
    matches!(&expr.0, Expr::CircuitBlock(stmts) if stmts.is_empty())
}

/// The declared qubit width (`Circuit<k,k,..>`'s `k`) of a circuit expression
/// in `on_high`/`on_low` position — read from the type the checker already
/// validated, rather than re-derived by scanning the elaborated gate
/// sequence, so it is exact even for `identity(0)` (which elaborates to zero
/// gates and would otherwise look indistinguishable from "unknown width").
fn on_high_width(
    expr: &Sp<Expr>,
    classical_env: &ClassicalEnv,
    ctx: &ElabCtx,
    fuel: &mut u32,
) -> Result<i64, ElabError> {
    if let Expr::App(f, x) = &expr.0 {
        let (head, args) = flatten_app(f, x);
        if let Expr::Var(name) = &head.0 {
            if name == "identity" && args.len() == 1 {
                return eval_classical(args[0], classical_env, fuel)?
                    .as_i64()
                    .ok_or(ElabError::NotClassical {
                        name: "identity width",
                    });
            }
            if let Some(def) = ctx.parametric.get(name) {
                let crate::types::Ty::Circuit { n, .. } = &def.ret_ty else {
                    return Err(ElabError::unsupported(
                        "on_high/on_low of a non-Circuit-returning call",
                        no_span(),
                    ));
                };
                let mut nat_env: HashMap<String, DepthExpr> = HashMap::new();
                for (param, arg) in def.params.iter().zip(args.iter()) {
                    if let Some(v) = eval_classical(arg, classical_env, fuel)?.as_i64()
                        && v >= 0
                    {
                        nat_env.insert(param.clone(), DepthExpr::Nat(v as u64));
                    }
                }
                return n.subst(&nat_env).as_const().map(|w| w as i64).ok_or(
                    ElabError::unsupported(
                        "on_high/on_low of a call with an unresolved width",
                        no_span(),
                    ),
                );
            }
        }
    }
    Err(ElabError::unsupported(
        "on_high/on_low of an expression with no statically known width",
        no_span(),
    ))
}

/// Adds `offset` to every literal qubit index in an already-elaborated
/// circuit expression — `on_high`'s embedding step.
fn shift_qubits(expr: &Sp<Expr>, offset: i64) -> Sp<Expr> {
    let span = expr.1;
    match &expr.0 {
        Expr::Compose(lhs, rhs) => (
            Expr::Compose(
                Box::new(shift_qubits(lhs, offset)),
                Box::new(shift_qubits(rhs, offset)),
            ),
            span,
        ),
        Expr::GateApp { gate, qubits } => (
            Expr::GateApp {
                gate: gate.clone(),
                qubits: Box::new(shift_qubit_targets(qubits, offset)),
            },
            span,
        ),
        Expr::Adjoint(inner) => (Expr::Adjoint(Box::new(shift_qubits(inner, offset))), span),
        _ => expr.clone(),
    }
}

fn shift_qubit_targets(qubits: &Sp<Expr>, offset: i64) -> Sp<Expr> {
    let span = qubits.1;
    match &qubits.0 {
        Expr::Int(n) => (Expr::Int(n + offset), span),
        Expr::Tuple(items) => (
            Expr::Tuple(
                items
                    .iter()
                    .map(|item| shift_qubit_targets(item, offset))
                    .collect(),
            ),
            span,
        ),
        _ => qubits.clone(),
    }
}

/// Extracts exactly two elements from an (already-elaborated) qubit-target
/// expression — the shape both `Rzz(theta) @ (i, j)` and `controlled(g) @
/// (control, target)` require.
fn tuple2(qubits: &Sp<Expr>) -> Result<(Sp<Expr>, Sp<Expr>), ElabError> {
    match &qubits.0 {
        Expr::Tuple(items) if items.len() == 2 => Ok((items[0].clone(), items[1].clone())),
        _ => Err(ElabError::unsupported(
            "expected a 2-qubit target tuple `(a, b)`",
            no_span(),
        )),
    }
}

fn gate_app(name: &str, qubit: &Sp<Expr>, span: chumsky::span::SimpleSpan) -> Sp<Expr> {
    (
        Expr::GateApp {
            gate: Box::new((Expr::Var(name.to_string()), span)),
            qubits: Box::new(qubit.clone()),
        },
        span,
    )
}

fn compose(steps: Vec<Sp<Expr>>, span: chumsky::span::SimpleSpan) -> Sp<Expr> {
    let mut iter = steps.into_iter();
    let first = iter.next().expect("compose called with no steps");
    iter.fold(first, |acc, step| {
        (Expr::Compose(Box::new(acc), Box::new(step)), span)
    })
}

/// `angle` is already a literal by this point (`decompose_controlled`'s caller
/// ran it through `subst_classical_vars`, which evaluates fully) — `gate_spec`
/// (`lower.rs`) requires a literal `Expr::Float` for a rotation's angle
/// argument, not a symbolic `Neg`/`BinOp` node, so these compute directly on
/// the literal rather than building one.
fn angle_value(angle: &Sp<Expr>) -> f64 {
    match &angle.0 {
        Expr::Float(f) => *f,
        Expr::Int(n) => *n as f64,
        _ => unreachable!("subst_classical_vars always reduces an angle to a literal"),
    }
}

fn negate_angle(angle: &Sp<Expr>) -> Sp<Expr> {
    (Expr::Float(-angle_value(angle)), angle.1)
}

fn halve_angle(angle: &Sp<Expr>) -> Sp<Expr> {
    (Expr::Float(angle_value(angle) / 2.0), angle.1)
}

/// `Rzz(theta) @ (i, j)` (exp(-iθ/2 · Z⊗Z), the ZZ-interaction used by Ising's
/// `zz_layer` and QAOA's `cost_layer`) has no native gate of its own; it
/// decomposes exactly as `CNOT(i,j) |> Rz(theta) @ j |> CNOT(i,j)` — verified
/// by cases: in the `i = 0` branch CNOT no-ops so this applies `Rz(theta)` to
/// `j` directly; in `i = 1` the two CNOTs conjugate `Rz(theta)` by `X`, and `X
/// · Rz(θ) · X = Rz(-θ)`. Comparing all four computational-basis phases
/// against `Rzz(θ) = diag(e^{-iθ/2}, e^{iθ/2}, e^{iθ/2}, e^{-iθ/2})`
/// (the ±1 eigenvalues of `Z⊗Z`) confirms the match exactly.
fn decompose_rzz(angle: &Sp<Expr>, q0: &Sp<Expr>, q1: &Sp<Expr>, span: SimpleSpan) -> Sp<Expr> {
    let cnot = cnot_app(control_target_tuple(q0, q1, span), span);
    let rz = rz_gate_app(angle, q1, span);
    compose(vec![cnot.clone(), rz, cnot], span)
}

fn control_target_tuple(control: &Sp<Expr>, target: &Sp<Expr>, span: SimpleSpan) -> Sp<Expr> {
    (Expr::Tuple(vec![control.clone(), target.clone()]), span)
}

fn cnot_app(qubits: Sp<Expr>, span: SimpleSpan) -> Sp<Expr> {
    gate_app("CNOT", &qubits, span)
}

/// `controlled(c) @ (control, target)` (SPEC §4.4 / issue #182).
///
/// Control distributes over sequential composition and circuit blocks:
/// `controlled(A |> B) = controlled(A) |> controlled(B)` on the same wires.
/// `par { body } * k` expands to `k` controlled copies on contiguous targets
/// `target, target+1, …` when `body` is width-1 (the only shape this path
/// places with a 2-tuple `(control, target)` start). Clifford+T single-qubit
/// generators and `Rx`/`Ry`/`Rz` use known decompositions into `CNOT`/`CZ`/
/// `CY`/`Rz`/local singles. Anything else is a span-accurate
/// [`ElabError::Unsupported`].
fn decompose_controlled(
    inner: &Sp<Expr>,
    control: &Sp<Expr>,
    target: &Sp<Expr>,
    classical_env: &ClassicalEnv,
    span: SimpleSpan,
) -> Result<Sp<Expr>, ElabError> {
    let fail = |construct: &'static str| ElabError::unsupported(construct, inner.1);
    match &inner.0 {
        Expr::Compose(lhs, rhs) => {
            let left = decompose_controlled(lhs, control, target, classical_env, span)?;
            let right = decompose_controlled(rhs, control, target, classical_env, span)?;
            Ok(compose_nonempty(left, right, span))
        }
        Expr::CircuitBlock(stmts) => {
            let body = circuit_block_expr(stmts, classical_env, inner.1)?;
            decompose_controlled(&body, control, target, classical_env, span)
        }
        Expr::Par(body, count) => {
            let mut fuel = 10_000u32;
            let k = eval_classical(count, classical_env, &mut fuel)?
                .as_i64()
                .ok_or_else(|| fail("controlled(par) count (expected Int)"))?;
            if k < 0 {
                return Err(fail("controlled(par) with a negative count"));
            }
            if k == 0 {
                return Ok(empty_circuit(span));
            }
            // Width-1 body copies land on contiguous targets starting at `target`.
            let mut composed = empty_circuit(span);
            for i in 0..k {
                let t = shift_qubit_targets(target, i);
                let step = decompose_controlled(body, control, &t, classical_env, span)?;
                composed = compose_nonempty(composed, step, span);
            }
            Ok(composed)
        }
        Expr::Adjoint(c) => {
            // For unitary `U`, `controlled(U†) = controlled(U)†` (control wire
            // is unchanged by the adjoint).
            let controlled = decompose_controlled(c, control, target, classical_env, span)?;
            reverse_and_invert(&controlled)
        }
        Expr::Var(name) => controlled_named_gate(name, None, control, target, classical_env, span),
        Expr::App(f, x) => {
            let (head, args) = flatten_app(f, x);
            let Expr::Var(name) = &head.0 else {
                return Err(fail("controlled() of an unrecognized gate expression"));
            };
            if args.len() != 1 {
                return Err(fail("controlled() of a multi-argument gate expression"));
            }
            let angle = subst_classical_vars(args[0], classical_env)?;
            controlled_named_gate(name, Some(angle), control, target, classical_env, span)
        }
        Expr::Controlled(_) => Err(fail(
            "nested controlled() (multi-controlled gates are not elaborated yet)",
        )),
        _ => Err(fail("controlled() of an unsupported circuit body")),
    }
}

/// Evaluate `let`s in a circuit block and return the trailing expression.
fn circuit_block_expr(
    stmts: &[Sp<Stmt>],
    classical_env: &ClassicalEnv,
    span: SimpleSpan,
) -> Result<Sp<Expr>, ElabError> {
    let Some((last, leading)) = stmts.split_last() else {
        return Err(ElabError::unsupported(
            "empty circuit block under controlled()",
            span,
        ));
    };
    let mut inner_env = classical_env.clone();
    let mut fuel = 10_000u32;
    for stmt in leading {
        let Stmt::Let { pat, rhs } = &stmt.0 else {
            return Err(ElabError::unsupported(
                "non-let statement inside a circuit block under controlled()",
                stmt.1,
            ));
        };
        let value = eval_classical(rhs, &inner_env, &mut fuel)?;
        bind_classical_pat(pat, value, &mut inner_env)?;
    }
    let Stmt::Expr(last_expr) = &last.0 else {
        return Err(ElabError::unsupported(
            "circuit block under controlled() not ending in an expression",
            last.1,
        ));
    };
    // Substitute classical bindings into the body so angles/indices resolve.
    subst_classical_vars(last_expr, &inner_env)
}

fn compose_nonempty(left: Sp<Expr>, right: Sp<Expr>, span: SimpleSpan) -> Sp<Expr> {
    if is_empty_circuit(&left) {
        return right;
    }
    if is_empty_circuit(&right) {
        return left;
    }
    (Expr::Compose(Box::new(left), Box::new(right)), span)
}

/// Controlled single-qubit Clifford+T / rotation generators (issue #182).
fn controlled_named_gate(
    name: &str,
    angle: Option<Sp<Expr>>,
    control: &Sp<Expr>,
    target: &Sp<Expr>,
    _classical_env: &ClassicalEnv,
    span: SimpleSpan,
) -> Result<Sp<Expr>, ElabError> {
    let ct = control_target_tuple(control, target, span);
    let cnot = || cnot_app(ct.clone(), span);
    match (name, angle) {
        ("I", None) => Ok(empty_circuit(span)),
        ("X", None) => Ok(gate_app("CNOT", &ct, span)),
        ("Y", None) => Ok(gate_app("CY", &ct, span)),
        ("Z", None) => Ok(gate_app("CZ", &ct, span)),
        // CH: S·H·T·CX·T†·H·S† on the target (standard OpenQASM/Qiskit form).
        ("H", None) => Ok(compose(
            vec![
                gate_app("S", target, span),
                gate_app("H", target, span),
                gate_app("T", target, span),
                cnot(),
                gate_app("T_dag", target, span),
                gate_app("H", target, span),
                gate_app("S_dag", target, span),
            ],
            span,
        )),
        // CS = CP(π/2), CT = CP(π/4); S†/T† use the negated phase.
        ("S", None) => Ok(decompose_controlled_phase(
            std::f64::consts::FRAC_PI_2,
            control,
            target,
            span,
        )),
        ("S_dag", None) => Ok(decompose_controlled_phase(
            -std::f64::consts::FRAC_PI_2,
            control,
            target,
            span,
        )),
        ("T", None) => Ok(decompose_controlled_phase(
            std::f64::consts::FRAC_PI_4,
            control,
            target,
            span,
        )),
        ("T_dag", None) => Ok(decompose_controlled_phase(
            -std::f64::consts::FRAC_PI_4,
            control,
            target,
            span,
        )),
        // SX = Rx(π/2); controlled via H·CRz·H.
        ("SX", None) => controlled_rx_angle(
            (Expr::Float(std::f64::consts::FRAC_PI_2), span),
            control,
            target,
            span,
        ),
        ("SX_dag", None) => controlled_rx_angle(
            (Expr::Float(-std::f64::consts::FRAC_PI_2), span),
            control,
            target,
            span,
        ),
        ("Rz", Some(theta)) => Ok(decompose_controlled_rz(&theta, control, target, span)),
        ("Rx", Some(theta)) => controlled_rx_angle(theta, control, target, span),
        ("Ry", Some(theta)) => Ok(decompose_controlled_ry(&theta, control, target, span)),
        ("Rz" | "Rx" | "Ry", None) => Err(ElabError::unsupported(
            "controlled() of a rotation missing its angle",
            span,
        )),
        (other, _) if circuit::gate_type(other).is_some() => Err(ElabError::unsupported(
            "controlled() of a multi-qubit or unrecognized single-qubit gate",
            span,
        )),
        _ => Err(ElabError::unsupported(
            "controlled() of an unsupported gate expression",
            span,
        )),
    }
}

/// `CRz(θ) = Rz(θ/2)@t |> CNOT |> Rz(-θ/2)@t |> CNOT`.
fn decompose_controlled_rz(
    angle: &Sp<Expr>,
    control: &Sp<Expr>,
    target: &Sp<Expr>,
    span: SimpleSpan,
) -> Sp<Expr> {
    let cnot = cnot_app(control_target_tuple(control, target, span), span);
    let rz_half = rz_gate_app(&halve_angle(angle), target, span);
    let rz_neg_half = rz_gate_app(&negate_angle(&halve_angle(angle)), target, span);
    compose(vec![rz_half, cnot.clone(), rz_neg_half, cnot], span)
}

/// `CRy(θ) = Ry(θ/2)@t |> CNOT |> Ry(-θ/2)@t |> CNOT`.
fn decompose_controlled_ry(
    angle: &Sp<Expr>,
    control: &Sp<Expr>,
    target: &Sp<Expr>,
    span: SimpleSpan,
) -> Sp<Expr> {
    let cnot = cnot_app(control_target_tuple(control, target, span), span);
    let ry_half = rotation_gate_app("Ry", &halve_angle(angle), target, span);
    let ry_neg_half = rotation_gate_app("Ry", &negate_angle(&halve_angle(angle)), target, span);
    compose(vec![ry_half, cnot.clone(), ry_neg_half, cnot], span)
}

/// `CRx(θ) = H@t |> CRz(θ) |> H@t` (since `H·Rz·H = Rx`).
fn controlled_rx_angle(
    angle: Sp<Expr>,
    control: &Sp<Expr>,
    target: &Sp<Expr>,
    span: SimpleSpan,
) -> Result<Sp<Expr>, ElabError> {
    let h = gate_app("H", target, span);
    let crz = decompose_controlled_rz(&angle, control, target, span);
    Ok(compose(vec![h.clone(), crz, h], span))
}

/// Controlled phase `CP(φ) = |1⟩⟨1| ⊗ P(φ)` as
/// `Rz(φ/2)@c |> CNOT |> Rz(-φ/2)@t |> CNOT |> Rz(φ/2)@t`.
fn decompose_controlled_phase(
    phi: f64,
    control: &Sp<Expr>,
    target: &Sp<Expr>,
    span: SimpleSpan,
) -> Sp<Expr> {
    let half = (Expr::Float(phi / 2.0), span);
    let neg_half = (Expr::Float(-phi / 2.0), span);
    let cnot = cnot_app(control_target_tuple(control, target, span), span);
    compose(
        vec![
            rz_gate_app(&half, control, span),
            cnot.clone(),
            rz_gate_app(&neg_half, target, span),
            cnot,
            rz_gate_app(&half, target, span),
        ],
        span,
    )
}

/// Builds `Rz(angle) @ target`.
fn rz_gate_app(angle: &Sp<Expr>, target: &Sp<Expr>, span: SimpleSpan) -> Sp<Expr> {
    rotation_gate_app("Rz", angle, target, span)
}

fn rotation_gate_app(
    name: &str,
    angle: &Sp<Expr>,
    target: &Sp<Expr>,
    span: SimpleSpan,
) -> Sp<Expr> {
    (
        Expr::GateApp {
            gate: Box::new((
                Expr::App(
                    Box::new((Expr::Var(name.to_string()), span)),
                    Box::new(angle.clone()),
                ),
                span,
            )),
            qubits: Box::new(target.clone()),
        },
        span,
    )
}

/// A fresh, deterministic fuel budget for one top-level elaboration.
pub fn fresh_fuel() -> u32 {
    FUEL_START
}

#[cfg(test)]
mod controlled_tests {
    use super::*;
    use crate::specialized_circuit::collect_gate_placements;
    use backend::unitary::{
        Complex, M2, M4, gate_unitary, mul4, rotation_unitary, tensor, two_qubit_gate_unitary,
        unitary_distance,
    };
    use std::collections::HashMap;
    use std::f64::consts::PI;

    fn lit_int(n: i64) -> Sp<Expr> {
        (Expr::Int(n), no_span())
    }

    fn var(name: &str) -> Sp<Expr> {
        (Expr::Var(name.to_string()), no_span())
    }

    fn controlled_of(inner: Sp<Expr>) -> Result<Sp<Expr>, ElabError> {
        decompose_controlled(&inner, &lit_int(0), &lit_int(1), &HashMap::new(), no_span())
    }

    fn ideal_controlled(u: M2) -> M4 {
        let zero = Complex::new(0.0, 0.0);
        let one = Complex::new(1.0, 0.0);
        M4([
            [one, zero, zero, zero],
            [zero, one, zero, zero],
            [zero, zero, u.0[0][0], u.0[0][1]],
            [zero, zero, u.0[1][0], u.0[1][1]],
        ])
    }

    fn expand_placement(gate: &Sp<Expr>, qubits: &Sp<Expr>) -> M4 {
        let targets: Vec<i64> = match &qubits.0 {
            Expr::Int(n) => vec![*n],
            Expr::Tuple(items) => items
                .iter()
                .map(|q| match q.0 {
                    Expr::Int(n) => n,
                    _ => panic!("non-literal qubit"),
                })
                .collect(),
            _ => panic!("bad qubit expr"),
        };
        let (name, angle) = match &gate.0 {
            Expr::Var(n) => (n.as_str(), None),
            Expr::App(f, x) => {
                let (head, args) = flatten_app(f, x);
                let Expr::Var(n) = &head.0 else {
                    panic!("bad rotation head");
                };
                let Expr::Float(a) = args[0].0 else {
                    panic!("non-float angle");
                };
                (n.as_str(), Some(a))
            }
            _ => panic!("bad gate"),
        };
        match targets.as_slice() {
            [q] => {
                let u = if let Some(a) = angle {
                    rotation_unitary(name, a).unwrap()
                } else {
                    gate_unitary(name).unwrap_or_else(|| panic!("unknown gate {name}"))
                };
                if *q == 0 {
                    tensor(u, gate_unitary("I").unwrap())
                } else {
                    tensor(gate_unitary("I").unwrap(), u)
                }
            }
            [0, 1] => {
                if let Some(u) = two_qubit_gate_unitary(name) {
                    u
                } else {
                    panic!("unknown two-qubit gate {name}")
                }
            }
            _ => panic!("unexpected qubit targets {targets:?}"),
        }
    }

    fn unitary_of_elaborated(expr: &Sp<Expr>) -> M4 {
        let placements = collect_gate_placements(expr).expect("placements");
        let id = tensor(gate_unitary("I").unwrap(), gate_unitary("I").unwrap());
        placements.iter().fold(id, |acc, (gate, qubits)| {
            // Circuit composition A |> B applies A first, then B: U_B · U_A · |ψ⟩.
            mul4(expand_placement(gate, qubits), acc)
        })
    }

    fn assert_controlled_equiv(inner: Sp<Expr>, u: M2) {
        let elaborated = controlled_of(inner).expect("decompose");
        let got = unitary_of_elaborated(&elaborated);
        let want = ideal_controlled(u);
        let dist = unitary_distance(got, want);
        assert!(
            dist < 1e-9,
            "controlled body not unitary-equivalent (distance {dist})"
        );
    }

    #[test]
    fn controlled_x_z_y_match_natives() {
        assert_controlled_equiv(var("X"), gate_unitary("X").unwrap());
        assert_controlled_equiv(var("Y"), gate_unitary("Y").unwrap());
        assert_controlled_equiv(var("Z"), gate_unitary("Z").unwrap());
    }

    #[test]
    fn controlled_h_s_t_clifford_t() {
        assert_controlled_equiv(var("H"), gate_unitary("H").unwrap());
        assert_controlled_equiv(var("S"), gate_unitary("S").unwrap());
        assert_controlled_equiv(var("T"), gate_unitary("T").unwrap());
        assert_controlled_equiv(var("S_dag"), gate_unitary("S_dag").unwrap());
        assert_controlled_equiv(var("T_dag"), gate_unitary("T_dag").unwrap());
    }

    #[test]
    fn controlled_rotations_rx_ry_rz() {
        let theta = PI / 5.0;
        let rz = (
            Expr::App(
                Box::new(var("Rz")),
                Box::new((Expr::Float(theta), no_span())),
            ),
            no_span(),
        );
        let rx = (
            Expr::App(
                Box::new(var("Rx")),
                Box::new((Expr::Float(theta), no_span())),
            ),
            no_span(),
        );
        let ry = (
            Expr::App(
                Box::new(var("Ry")),
                Box::new((Expr::Float(theta), no_span())),
            ),
            no_span(),
        );
        assert_controlled_equiv(rz, rotation_unitary("Rz", theta).unwrap());
        assert_controlled_equiv(rx, rotation_unitary("Rx", theta).unwrap());
        assert_controlled_equiv(ry, rotation_unitary("Ry", theta).unwrap());
    }

    #[test]
    fn controlled_distributes_over_compose() {
        let body = (
            Expr::Compose(Box::new(var("H")), Box::new(var("T"))),
            no_span(),
        );
        let u = {
            // H then T: U = T · H
            let h = gate_unitary("H").unwrap();
            let t = gate_unitary("T").unwrap();
            backend::unitary::mul2(t, h)
        };
        assert_controlled_equiv(body, u);
    }

    #[test]
    fn controlled_circuit_block_h_then_t() {
        let body = (
            Expr::CircuitBlock(vec![(
                Stmt::Expr((
                    Expr::Compose(Box::new(var("H")), Box::new(var("T"))),
                    no_span(),
                )),
                no_span(),
            )]),
            no_span(),
        );
        let u = backend::unitary::mul2(gate_unitary("T").unwrap(), gate_unitary("H").unwrap());
        assert_controlled_equiv(body, u);
    }

    #[test]
    fn controlled_par_two_hadamards() {
        // par { H } * 2 under control on targets 1 and 2 — use control=0, target=1 start.
        let body = (
            Expr::Par(Box::new(var("H")), Box::new(lit_int(2))),
            no_span(),
        );
        let elaborated = controlled_of(body).expect("decompose par");
        // Ideal: CH on (0,1) then CH on (0,2). Build 8×8? Our helpers are 4×4 only.
        // Instead check the elaborated form expands to two CH decompositions with
        // targets 1 and 2.
        let placements = collect_gate_placements(&elaborated).unwrap();
        let targets: Vec<Vec<i64>> = placements
            .iter()
            .map(|(_, q)| match &q.0 {
                Expr::Int(n) => vec![*n],
                Expr::Tuple(items) => items
                    .iter()
                    .map(|i| match i.0 {
                        Expr::Int(n) => n,
                        _ => panic!(),
                    })
                    .collect(),
                _ => panic!(),
            })
            .collect();
        assert!(
            targets.iter().any(|t| t == &[0, 1]) && targets.iter().any(|t| t == &[0, 2]),
            "expected CNOT/CY-style pairs on (0,1) and (0,2), got {targets:?}"
        );
    }

    #[test]
    fn unsupported_nested_controlled_names_construct_and_span() {
        let nested = (
            Expr::Controlled(Box::new(var("X"))),
            SimpleSpan::from(10..20),
        );
        let err = controlled_of(nested).expect_err("nested controlled");
        match err {
            ElabError::Unsupported { construct, span } => {
                assert!(construct.contains("nested controlled"));
                assert_eq!((span.start, span.end), (10, 20));
            }
            other => panic!("unexpected {other}"),
        }
    }
}
