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
//! supported. `Rzz(theta) @ (i, j)` and `controlled(X | Z | Rz(theta)) @
//! (control, target)` have no native gate of their own, so they are rewritten
//! here into the existing `CNOT`/`Rz` primitives every other gate already
//! lowers through — see `decompose_rzz`/`decompose_controlled`. `tensored`/
//! `split`, `fold` over a circuit accumulator, and `controlled` of anything
//! other than `X`/`Z`/`Rz` are not yet elaborated — a program using them is
//! rejected with [`ElabError::Unsupported`], not silently miscompiled.

use std::collections::HashMap;

use quon_core::DepthExpr;
use thiserror::Error;

use crate::ast::{BinOp, Expr, LitPat, Pat};
use crate::lexer::Sp;
use crate::typecheck::circuit;

#[derive(Debug, Error)]
pub enum ElabError {
    #[error("elaboration is not implemented for `{construct}`")]
    Unsupported { construct: &'static str },
    #[error("`{name}` is not a classically evaluable expression")]
    NotClassical { name: &'static str },
    #[error("unbound variable `{name}` during elaboration")]
    UnboundVar { name: String },
    #[error("elaboration exceeded its evaluation budget (possible non-terminating recursion)")]
    FuelExhausted,
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

/// Evaluates a classical (non-quantum) expression to a [`Value`] under `env`.
///
/// Total for the supported fragment: recursion in the source language is only
/// ever over a structurally decreasing `Nat` (already proved terminating by
/// the type checker's `check_termination`, `frontend/src/typecheck/mod.rs`),
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
                        return Err(ElabError::Unsupported {
                            construct: "tuple pattern in classical match",
                        });
                    }
                }
            }
            Err(ElabError::Unsupported {
                construct: "non-exhaustive classical match",
            })
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
            Err(ElabError::Unsupported {
                construct: "classical function call",
            })
        }
        _ => Err(ElabError::Unsupported {
            construct: "classical expression",
        }),
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
                return Err(ElabError::Unsupported {
                    construct: "tuple pattern bound to a non-tuple value",
                });
            };
            if pats.len() != values.len() {
                return Err(ElabError::Unsupported {
                    construct: "tuple pattern arity mismatch",
                });
            }
            for (p, v) in pats.iter().zip(values) {
                bind_classical_pat(p, v, env)?;
            }
            Ok(())
        }
        Pat::Lit(_) => Err(ElabError::Unsupported {
            construct: "literal pattern in classical let",
        }),
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
                return Err(ElabError::Unsupported {
                    construct: "empty circuit block",
                });
            };
            let mut inner_env = classical_env.clone();
            for stmt in leading {
                let crate::ast::Stmt::Let { pat, rhs } = &stmt.0 else {
                    return Err(ElabError::Unsupported {
                        construct: "non-let statement inside a circuit block",
                    });
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
                return Err(ElabError::Unsupported {
                    construct: "circuit block not ending in an expression",
                });
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
                    // (`frontend/src/typecheck/mod.rs`), only a proof-context
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
                        return Err(ElabError::Unsupported {
                            construct: "tuple pattern in circuit-body match",
                        });
                    }
                }
            }
            Err(ElabError::Unsupported {
                construct: "non-exhaustive circuit-body match",
            })
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
                return Err(ElabError::Unsupported {
                    construct: "for-loop iterator (expected qubits/range/diag)",
                });
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
        _ => Err(ElabError::Unsupported {
            construct: "circuit body expression during elaboration",
        }),
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
                        return Err(ElabError::Unsupported {
                            construct: "parametric circuit call arity mismatch",
                        });
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
                    return Err(ElabError::Unsupported {
                        construct: "on_high/on_low of a non-Circuit-returning call",
                    });
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
                    ElabError::Unsupported {
                        construct: "on_high/on_low of a call with an unresolved width",
                    },
                );
            }
        }
    }
    Err(ElabError::Unsupported {
        construct: "on_high/on_low of an expression with no statically known width",
    })
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
        _ => Err(ElabError::Unsupported {
            construct: "expected a 2-qubit target tuple `(a, b)`",
        }),
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
fn decompose_rzz(
    angle: &Sp<Expr>,
    q0: &Sp<Expr>,
    q1: &Sp<Expr>,
    span: chumsky::span::SimpleSpan,
) -> Sp<Expr> {
    let cnot = gate_app(
        "CNOT",
        &(Expr::Tuple(vec![q0.clone(), q1.clone()]), span),
        span,
    );
    let rz = rz_gate_app(angle, q1, span);
    compose(vec![cnot.clone(), rz, cnot], span)
}

/// `controlled(g) @ (control, target)` for the two forms this codebase's
/// reference algorithms use: `controlled(X)`/`controlled(Z)` are exactly
/// `CNOT`/`CZ` (no decomposition needed), and `controlled(Rz(theta))` (Shor's
/// `controlled_rotations`/`modmul`) decomposes as `Rz(theta/2) @ target |>
/// CNOT(control,target) |> Rz(-theta/2) @ target |> CNOT(control,target)` —
/// verified by cases: `control = 0` cancels to `Rz(θ/2)·Rz(-θ/2) = I`
/// (`CRz`'s `|0⟩⟨0|⊗I` block); `control = 1` conjugates the second `Rz` by `X`
/// (`X·Rz(-θ/2)·X = Rz(θ/2)`), giving `Rz(θ/2)·Rz(θ/2) = Rz(θ)` (`CRz`'s
/// `|1⟩⟨1|⊗Rz(θ)` block).
fn decompose_controlled(
    inner: &Sp<Expr>,
    control: &Sp<Expr>,
    target: &Sp<Expr>,
    classical_env: &ClassicalEnv,
    span: chumsky::span::SimpleSpan,
) -> Result<Sp<Expr>, ElabError> {
    match &inner.0 {
        Expr::Var(name) if name == "X" => Ok(gate_app(
            "CNOT",
            &(Expr::Tuple(vec![control.clone(), target.clone()]), span),
            span,
        )),
        Expr::Var(name) if name == "Z" => Ok(gate_app(
            "CZ",
            &(Expr::Tuple(vec![control.clone(), target.clone()]), span),
            span,
        )),
        Expr::App(f, x) => {
            let (head, args) = flatten_app(f, x);
            let Expr::Var(name) = &head.0 else {
                return Err(ElabError::Unsupported {
                    construct: "controlled() of an unrecognized gate expression",
                });
            };
            if name != "Rz" || args.len() != 1 {
                return Err(ElabError::Unsupported {
                    construct: "controlled() is only implemented for X, Z, and Rz",
                });
            }
            let angle = subst_classical_vars(args[0], classical_env)?;
            let cnot = gate_app(
                "CNOT",
                &(Expr::Tuple(vec![control.clone(), target.clone()]), span),
                span,
            );
            let rz_half = rz_gate_app(&halve_angle(&angle), target, span);
            let rz_neg_half = rz_gate_app(&negate_angle(&halve_angle(&angle)), target, span);
            Ok(compose(
                vec![rz_half, cnot.clone(), rz_neg_half, cnot],
                span,
            ))
        }
        _ => Err(ElabError::Unsupported {
            construct: "controlled() is only implemented for X, Z, and Rz",
        }),
    }
}

/// Builds `Rz(angle) @ target`.
fn rz_gate_app(angle: &Sp<Expr>, target: &Sp<Expr>, span: chumsky::span::SimpleSpan) -> Sp<Expr> {
    (
        Expr::GateApp {
            gate: Box::new((
                Expr::App(
                    Box::new((Expr::Var("Rz".to_string()), span)),
                    Box::new(angle.clone()),
                ),
                span,
            )),
            qubits: Box::new(target.clone()),
        },
        span,
    )
}

/// Builds the adjoint of an already-elaborated, fully concrete circuit body:
/// reverse gate order and invert each gate (mirrors `lower.rs`'s
/// `inline_inverted_body`, at the AST level so it composes with elaboration).
fn reverse_and_invert(expr: &Sp<Expr>) -> Result<Sp<Expr>, ElabError> {
    if is_empty_circuit(expr) {
        return Ok(expr.clone());
    }
    let placements = collect_gate_placements(expr)?;
    let span = expr.1;
    let mut result: Option<Sp<Expr>> = None;
    for (gate, qubits) in placements.into_iter().rev() {
        let inv_gate = match &gate.0 {
            Expr::Var(name) => (Expr::Var(inverse_gate_name(name)), gate.1),
            // A rotation (`Rz(theta)`, from a source rotation gate or a
            // decomposed `Rzz`/`controlled(Rz(..))`): its adjoint is the same
            // gate at the negated angle (`Rz(θ)† = Rz(-θ)`), not a different
            // gate name.
            Expr::App(f, x) => {
                let (head, args) = flatten_app(f, x);
                let (Expr::Var(name), [angle]) = (&head.0, args.as_slice()) else {
                    return Err(ElabError::Unsupported {
                        construct: "adjoint of an unrecognized rotation gate",
                    });
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
                return Err(ElabError::Unsupported {
                    construct: "adjoint of a non-primitive gate",
                });
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
    result.ok_or(ElabError::Unsupported {
        construct: "adjoint of an empty circuit",
    })
}

type GatePlacement = (Sp<Expr>, Sp<Expr>);

fn collect_gate_placements(expr: &Sp<Expr>) -> Result<Vec<GatePlacement>, ElabError> {
    match &expr.0 {
        Expr::Compose(lhs, rhs) => {
            let mut out = collect_gate_placements(lhs)?;
            out.extend(collect_gate_placements(rhs)?);
            Ok(out)
        }
        Expr::GateApp { gate, qubits } => Ok(vec![(*gate.clone(), *qubits.clone())]),
        _ => Err(ElabError::Unsupported {
            construct: "adjoint of a non-gate-sequence circuit",
        }),
    }
}

fn inverse_gate_name(name: &str) -> String {
    match name {
        "S" => "S_dag".to_string(),
        "S_dag" => "S".to_string(),
        "T" => "T_dag".to_string(),
        "T_dag" => "T".to_string(),
        "SX" => "SX_dag".to_string(),
        "SX_dag" => "SX".to_string(),
        other => other.to_string(),
    }
}

fn flatten_app<'a>(f: &'a Sp<Expr>, x: &'a Sp<Expr>) -> (&'a Sp<Expr>, Vec<&'a Sp<Expr>>) {
    let mut args = vec![x];
    let mut head = f;
    while let Expr::App(inner_f, inner_x) = &head.0 {
        args.push(inner_x);
        head = inner_f;
    }
    args.reverse();
    (head, args)
}

/// A fresh, deterministic fuel budget for one top-level elaboration.
pub fn fresh_fuel() -> u32 {
    FUEL_START
}
