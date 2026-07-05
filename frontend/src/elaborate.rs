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
//! `range`/`diag`, `repeat` with a computed count, recursive circuit functions
//! via `match n { 0 => .., _ => .. }`, and nested parametric calls are
//! supported. `Rzz(theta) @ (i, j)` and `controlled(X | Z | Rz(theta)) @
//! (control, target)` have no native gate of their own, so they are rewritten
//! here into the existing `CNOT`/`Rz` primitives every other gate already
//! lowers through — see `decompose_rzz`/`decompose_controlled`. `pairs`,
//! `tensored`/`split`/`on_high`, `fold` over a circuit accumulator, and
//! `controlled` of anything other than `X`/`Z`/`Rz` are not yet elaborated —
//! a program using them is rejected with [`ElabError::Unsupported`], not
//! silently miscompiled.

use std::collections::HashMap;

use crate::ast::{BinOp, Expr, LitPat, Pat};
use crate::lexer::Sp;
use crate::typecheck::circuit;
use thiserror::Error;

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
            Ok((Expr::Compose(Box::new(l), Box::new(r)), span))
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
            let mut composed: Option<Sp<Expr>> = None;
            for item in items {
                let mut inner_env = classical_env.clone();
                bind_classical_pat(pat, item, &mut inner_env)?;
                let step = elaborate_circuit_body(body, &inner_env, ctx, fuel)?;
                composed = Some(match composed {
                    None => step,
                    Some(acc) => (Expr::Compose(Box::new(acc), Box::new(step)), span),
                });
            }
            composed.ok_or(ElabError::Unsupported {
                construct: "empty for-loop (zero iterations)",
            })
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
                let mut composed: Option<Sp<Expr>> = None;
                for _ in 0..count {
                    composed = Some(match composed {
                        None => body.clone(),
                        Some(acc) => (Expr::Compose(Box::new(acc), Box::new(body.clone())), span),
                    });
                }
                return composed.ok_or(ElabError::Unsupported {
                    construct: "repeat with a zero count",
                });
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
    let placements = collect_gate_placements(expr)?;
    let span = expr.1;
    let mut result: Option<Sp<Expr>> = None;
    for (gate, qubits) in placements.into_iter().rev() {
        let inv_name = match &gate.0 {
            Expr::Var(name) => inverse_gate_name(name),
            _ => {
                return Err(ElabError::Unsupported {
                    construct: "adjoint of a non-primitive gate",
                });
            }
        };
        let inv_gate = (Expr::Var(inv_name), gate.1);
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
