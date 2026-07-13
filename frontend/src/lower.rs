// AST → quantum.circ MLIR lowering — see issue #16, SPEC.md §6
// Translates a type-checked AST to in-memory quantum.circ IR using Melior.
// Circuit<n,m,d,C> indices are encoded as op attributes (ADR-0002).

use std::collections::HashMap;

use melior::Context;
use melior::ir::attribute::{BoolAttribute, StringAttribute};
use melior::ir::operation::OperationBuilder;
use melior::ir::{
    Block, BlockLike, Identifier, Location, Module, Operation, OperationRef, Region, RegionLike,
    Value,
};
use mlir_bridge::dialect::monadic_staging as staging;
use mlir_bridge::dialect::quantum_circ as qc;
use quon_core::DepthExpr;
use thiserror::Error;

use crate::ast::{CliffordClass, Decl, Expr, Name, Pat, Stmt, Type as AstType};
use crate::diagnostics::Diagnostic;
use crate::elaborate;
use crate::lexer::Sp;
use crate::typecheck::circuit;
use crate::typecheck::{TypeChecker, TypeError};
use crate::types::Ty;

type GatePlacement = (Sp<Expr>, Sp<Expr>);

/// Errors raised while lowering a well-typed program to `quantum.circ`.
#[derive(Debug, Error)]
pub enum LowerError {
    #[error("lowering is not implemented for `{construct}`")]
    Unsupported { construct: &'static str },
    #[error(
        "parameterized run function `{name}` is not supported by the QASM lowering pass yet (deferred under #27)"
    )]
    ParametricRunFn { name: String },
    #[error("could not read a constant qubit count from `{field}`")]
    NonConstWidth { field: &'static str },
    #[error("unknown gate `{name}`")]
    UnknownGate { name: String },
    #[error("rotation gate `{name}` needs a static angle literal")]
    NonStaticRotation { name: String },
    #[error("call to unknown circuit function `{name}`")]
    UnknownCallee { name: String },
    #[error("could not statically determine the qubit width of a specialized circuit")]
    UnresolvedSpecializationWidth,
    #[error("MLIR builder failed: {0}")]
    Mlir(#[from] qc::BuildError),
    #[error("internal lowering error: {0}")]
    Internal(&'static str),
    #[error("type checking failed: {0}")]
    Type(#[from] TypeError),
    #[error("elaborating a parametric circuit call: {0}")]
    Elab(#[from] elaborate::ElabError),
}

/// Lowers a desugared, type-checked declaration list into a `quantum.circ` module.
pub struct LoweringCtx<'c> {
    context: &'c Context,
    module: Module<'c>,
    location: Location<'c>,
    checker: TypeChecker,
    /// Bodies of zero-parameter circuit functions, for inlining and adjoint.
    bodies: HashMap<Name, Sp<Expr>>,
    /// Metadata for each circuit function.
    func_meta: HashMap<Name, FuncMeta>,
    /// Parametric (`Nat`/`Int`/`Float`-parameterized) circuit function
    /// definitions, specialized on demand at a concrete call site (issue #1,
    /// MVP milestone M2) — see `elaborate.rs`.
    parametric: HashMap<Name, elaborate::ParametricDef>,
    /// Memoizes specializations already emitted, keyed by a canonical
    /// `"name(arg1,arg2,...)"` string, so the same call site (e.g.
    /// `hadamard_all(n)` reached twice with `n = 3`) is not re-emitted.
    specialized: HashMap<String, Name>,
    next_synth_id: u64,
}

#[derive(Clone)]
struct FuncMeta {
    depth: DepthExpr,
    clifford: bool,
    in_qubits: i64,
    out_qubits: i64,
}

struct GateSpec {
    name: String,
    angle: Option<f64>,
    clifford: bool,
    depth_contribution: i64,
}

impl<'c> LoweringCtx<'c> {
    pub fn new(context: &'c Context) -> Self {
        qc::register_dialect(context);
        let location = Location::unknown(context);
        Self {
            context,
            module: Module::new(location),
            location,
            checker: TypeChecker::new(),
            bodies: HashMap::new(),
            func_meta: HashMap::new(),
            parametric: HashMap::new(),
            specialized: HashMap::new(),
            next_synth_id: 0,
        }
    }

    pub fn lower_decls(&mut self, decls: &[Sp<Decl>]) -> Result<&Module<'c>, LowerError> {
        if let Err(errs) = self.checker.check_decls(decls) {
            return Err(errs
                .into_iter()
                .next()
                .map(LowerError::Type)
                .unwrap_or(LowerError::Internal("empty type error list")));
        }

        for decl in decls {
            if let Decl::Fn {
                name,
                params,
                ret,
                body,
            } = &decl.0
                && let Ok(ret_ty @ Ty::Circuit { .. }) = self.checker.resolve_type(ret)
            {
                if params.is_empty() {
                    let Ty::Circuit { n, m, d, c } = ret_ty else {
                        unreachable!("matched above");
                    };
                    self.func_meta.insert(
                        name.0.clone(),
                        FuncMeta {
                            depth: d,
                            clifford: matches!(c, CliffordClass::Clifford),
                            in_qubits: const_width(&n, "in_qubits")?,
                            out_qubits: const_width(&m, "out_qubits")?,
                        },
                    );
                    self.bodies.insert(name.0.clone(), body.clone());
                } else {
                    // A parametric circuit function (e.g. `hadamard_all(n)`):
                    // not emitted yet — every parameter must be `Nat`/`Int`/
                    // `Float` (issue #1 MVP scope; circuit-valued parameters
                    // are out of scope, see docs/plans/mvp-landing-plan.md
                    // §5), specialized on demand at each concrete call site
                    // (`specialize_named_fn`).
                    self.parametric.insert(
                        name.0.clone(),
                        elaborate::ParametricDef {
                            params: params.iter().map(|(p, _)| p.0.clone()).collect(),
                            body: body.clone(),
                            ret_ty,
                        },
                    );
                }
            }
        }

        for decl in decls {
            if let Decl::Fn {
                name,
                params,
                ret,
                body,
            } = &decl.0
            {
                self.lower_circuit_fn(&name.0, params, ret, body)?;
            }
        }

        // Run-block (`Q<τ>`) functions lower to a `quantum.circ.run` staging op,
        // which the monadic-lowering pass (#17) rewrites to `quantum.dynamic`.
        // Circuit funcs are lowered first so `apply` callees already exist.
        for decl in decls {
            if let Decl::Fn {
                name,
                params,
                ret,
                body,
            } = &decl.0
            {
                self.lower_run_fn(&name.0, params, ret, body)?;
            }
        }
        Ok(&self.module)
    }

    pub fn into_module(self) -> Module<'c> {
        self.module
    }

    fn lower_circuit_fn(
        &mut self,
        name: &Name,
        params: &[(Sp<Name>, Sp<AstType>)],
        ret: &Sp<AstType>,
        body: &Sp<Expr>,
    ) -> Result<(), LowerError> {
        let Ty::Circuit { n, m, d, c } = self.checker.resolve_type(ret)? else {
            return Ok(());
        };
        if !params.is_empty() {
            // Recorded in `self.parametric` during the first pass; emitted on
            // demand by `specialize_named_fn` at each concrete call site, not
            // eagerly here (its width/depth depend on the call-site argument).
            return Ok(());
        }
        let in_qubits = const_width(&n, "in_qubits")?;
        let out_qubits = const_width(&m, "out_qubits")?;
        let clifford = matches!(c, CliffordClass::Clifford);

        if let Expr::Adjoint(inner) = &body.0
            && let Some(callee) = zero_arg_callee_name(inner)
        {
            let callee_meta =
                self.func_meta
                    .get(&callee)
                    .cloned()
                    .ok_or_else(|| LowerError::UnknownCallee {
                        name: callee.clone(),
                    })?;
            let ref_op = circuit_ref_op(
                self.context,
                &callee,
                &callee_meta.depth,
                callee_meta.clifford,
                self.location,
            )?;
            let appended = self.module.body().append_operation(ref_op);
            let callee_ref = match appended.result(0) {
                Ok(value) => Value::from(value),
                Err(_) => return Err(LowerError::Internal("missing circuit ref result")),
            };
            let adjoint = qc::adjoint(self.context, callee_ref, &callee_meta.depth, self.location)?;
            self.module.body().append_operation(adjoint);
        }

        self.emit_circuit_func(name, in_qubits, out_qubits, &d, clifford, body)?;
        Ok(())
    }

    /// Emits `name` as a `quantum.circ.func` with the given, already-concrete
    /// shape, lowering `body` with the existing zero-parameter walker
    /// (`lower_circuit_block`/`lower_circuit_body_expr`). Shared by
    /// `lower_circuit_fn` (a source-level zero-param definition) and
    /// `specialize_named_fn` (a monomorphic instantiation of a parametric
    /// one) so both funnel through one MLIR-emission path.
    fn emit_circuit_func(
        &mut self,
        name: &str,
        in_qubits: i64,
        out_qubits: i64,
        depth: &DepthExpr,
        clifford: bool,
        body: &Sp<Expr>,
    ) -> Result<(), LowerError> {
        self.func_meta.insert(
            name.to_string(),
            FuncMeta {
                depth: depth.clone(),
                clifford,
                in_qubits,
                out_qubits,
            },
        );
        self.bodies.insert(name.to_string(), body.clone());

        let region = Region::new();
        let qubit = qc::qubit_type(self.context);
        let mut block = Block::new(
            &(0..in_qubits)
                .map(|_| (qubit, self.location))
                .collect::<Vec<_>>(),
        );
        let mut wires: Vec<Value<'c, 'c>> = Vec::with_capacity(block.argument_count());
        for i in 0..block.argument_count() {
            let arg = block
                .argument(i)
                .map_err(|_| LowerError::Internal("missing block argument"))?;
            wires.push(Value::from(arg));
        }

        match &body.0 {
            Expr::CircuitBlock(stmts) => {
                self.lower_circuit_block(stmts, &mut block, &mut wires)?;
            }
            other => {
                self.lower_circuit_body_expr(other, &mut block, &mut wires)?;
            }
        }

        if wires.len() != out_qubits as usize {
            return Err(LowerError::Unsupported {
                construct: "circuit output width mismatch",
            });
        }

        block.append_operation(qc::r#return(&wires, self.location)?);
        region.append_block(block);
        let func = qc::func(
            self.context,
            name,
            in_qubits,
            out_qubits,
            depth,
            clifford,
            region,
            self.location,
        )?;
        self.module.body().append_operation(func);
        Ok(())
    }

    /// Specializes the parametric circuit function `name` at concrete
    /// argument values `args` (each a classically-evaluable expression —
    /// typically an integer literal, or an expression closed over an
    /// enclosing parametric scope's own already-bound parameters), emitting a
    /// fresh monomorphic `quantum.circ.func` the first time this exact
    /// `(name, args)` pair is requested, and returning its synthesized name.
    fn specialize_named_fn(
        &mut self,
        name: &str,
        args: &[Sp<Expr>],
        classical_env: &HashMap<Name, elaborate::Value>,
    ) -> Result<Name, LowerError> {
        let def = self
            .parametric
            .get(name)
            .cloned()
            .ok_or_else(|| LowerError::UnknownCallee {
                name: name.to_string(),
            })?;
        if def.params.len() != args.len() {
            return Err(LowerError::Unsupported {
                construct: "parametric circuit call arity mismatch",
            });
        }
        let mut fuel = elaborate::fresh_fuel();
        let mut callee_env: HashMap<Name, elaborate::Value> = HashMap::new();
        let mut cache_key = name.to_string();
        cache_key.push('(');
        for (i, (param, arg)) in def.params.iter().zip(args.iter()).enumerate() {
            let value = elaborate::eval_classical(arg, classical_env, &mut fuel)?;
            if i > 0 {
                cache_key.push(',');
            }
            cache_key.push_str(&format!("{value:?}"));
            callee_env.insert(param.clone(), value);
        }
        cache_key.push(')');

        if let Some(existing) = self.specialized.get(&cache_key) {
            return Ok(existing.clone());
        }

        // Reserve the cache entry before recursing so a (structurally
        // impossible, since recursive circuit fns decrease a `Nat`) self-call
        // at the same arguments cannot recurse into `specialize_named_fn`
        // again and double-emit.
        let synth_name = format!("{name}__elab{}", self.next_synth_id);
        self.next_synth_id += 1;
        self.specialized
            .insert(cache_key.clone(), synth_name.clone());

        let ctx = elaborate::ElabCtx {
            parametric: self
                .parametric
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        };
        let elaborated =
            elaborate::elaborate_circuit_body(&def.body, &callee_env, &ctx, &mut fuel)?;

        let Ty::Circuit { n, m, d, c } = def.ret_ty.clone() else {
            return Err(LowerError::Unsupported {
                construct: "specialized function does not return a Circuit",
            });
        };
        let nat_env: HashMap<String, DepthExpr> = callee_env
            .iter()
            .filter_map(|(k, v)| match v {
                elaborate::Value::Int(i) if *i >= 0 => Some((k.clone(), DepthExpr::Nat(*i as u64))),
                _ => None,
            })
            .collect();
        let in_qubits = n
            .subst(&nat_env)
            .as_const()
            .ok_or(LowerError::UnresolvedSpecializationWidth)? as i64;
        let out_qubits = m
            .subst(&nat_env)
            .as_const()
            .ok_or(LowerError::UnresolvedSpecializationWidth)? as i64;
        let depth = d.subst(&nat_env);
        let clifford = matches!(c, CliffordClass::Clifford);

        self.emit_circuit_func(
            &synth_name,
            in_qubits,
            out_qubits,
            &depth,
            clifford,
            &elaborated,
        )?;
        Ok(synth_name)
    }

    /// Resolves a run-block `@` gate expression that [`circuit_callee`]
    /// couldn't handle directly (a call site `circuit_callee` only recognizes
    /// a bare name or a zero-arg call) — either a direct call to a recorded
    /// parametric circuit function (`hadamard_all(3)`), or an arbitrary
    /// compound circuit expression built from `|>`/`repeat`/parametric calls
    /// (e.g. `hadamard_all(3) |> repeat(iters, oracle() |> diffusion(3))`).
    /// Both are elaborated to a fully concrete gate sequence and emitted as a
    /// fresh, uniquely-named `quantum.circ.func`, whose name is returned as
    /// the callee.
    fn resolve_circuit_callee(&mut self, gate: &Sp<Expr>) -> Result<Name, LowerError> {
        if let Expr::App(f, x) = &gate.0 {
            let (head, args) = flatten_app(f, x);
            if let Expr::Var(name) = &head.0
                && self.parametric.contains_key(name)
            {
                let args: Vec<Sp<Expr>> = args.into_iter().cloned().collect();
                return self.specialize_named_fn(name, &args, &HashMap::new());
            }
        }

        let mut fuel = elaborate::fresh_fuel();
        let ctx = elaborate::ElabCtx {
            parametric: self
                .parametric
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
        };
        let elaborated = elaborate::elaborate_circuit_body(gate, &HashMap::new(), &ctx, &mut fuel)?;

        let width = max_qubit_index(&elaborated)
            .map(|max| max + 1)
            .ok_or(LowerError::UnresolvedSpecializationWidth)?;
        let gate_count = count_gates(&elaborated);
        let synth_name = format!("__anon_circuit{}", self.next_synth_id);
        self.next_synth_id += 1;
        // A conservative depth/Clifford estimate: exact values are optimization
        // bookkeeping (scheduling, Clifford-only rewrites), not semantically
        // load-bearing for the emitted QASM — see mvp-landing-plan.md M1's
        // finding that the emitter never even reads these attributes.
        self.emit_circuit_func(
            &synth_name,
            width as i64,
            width as i64,
            &DepthExpr::Nat(gate_count as u64),
            false,
            &elaborated,
        )?;
        Ok(synth_name)
    }

    /// Lower a quantum-monad (`Q<τ>`) function into a `quantum.circ.run` staging
    /// op. The desugared body is a `Bind`/`Let`/`Return` chain (issue #8); we walk
    /// it, emitting `monadic_staging` ops (`qreg`/`apply`/`measure`/`yield`) into
    /// the run region's entry block. The monadic-lowering pass (#17) then rewrites
    /// those to `quantum.dynamic` IR.
    fn lower_run_fn(
        &mut self,
        name: &Name,
        params: &[(Sp<Name>, Sp<AstType>)],
        ret: &Sp<AstType>,
        body: &Sp<Expr>,
    ) -> Result<(), LowerError> {
        let Ok(Ty::Q(_)) = self.checker.resolve_type(ret) else {
            return Ok(());
        };
        // Qubit-parameterized entry points (e.g. `teleport`) would thread their
        // params in as run operands; the runnable entry points we compile take no
        // qubit parameters (they allocate via `qreg`).
        if !params.is_empty() {
            return Err(LowerError::ParametricRunFn { name: name.clone() });
        }

        let region = Region::new();
        let mut block = Block::new(&[]);
        let mut env: HashMap<Name, Vec<Value<'c, 'c>>> = HashMap::new();
        let outputs = self.lower_monadic(body, &mut block, &mut env)?;
        block.append_operation(staging::r#yield(&outputs, self.location));
        region.append_block(block);
        self.module
            .body()
            .append_operation(staging::run(self.context, &[], region, self.location));
        Ok(())
    }

    /// Walk a monadic expression, emitting staging ops and returning the SSA
    /// values it produces (the monadic result).
    fn lower_monadic(
        &mut self,
        expr: &Sp<Expr>,
        block: &mut Block<'c>,
        env: &mut HashMap<Name, Vec<Value<'c, 'c>>>,
    ) -> Result<Vec<Value<'c, 'c>>, LowerError> {
        match &expr.0 {
            Expr::Bind { rhs, param, body } => {
                let results = self.eval(rhs, block, env)?;
                env.insert(param.0.clone(), results);
                self.lower_monadic(body, block, env)
            }
            // `let (hi, lo) = split(k, q)` (SPEC §5, Shor's modular
            // exponentiation): splits a register's *wire list* into two
            // sublists of k and (len-k) qubits. This can't go through the
            // ordinary `eval` + `bind_pattern` path below: `bind_pattern`'s
            // `Pat::Tuple` case zips one *value* per pattern element (the
            // right model for e.g. `(msg, alice, bob) <- prep() @ qreg(3)`,
            // where each name is one qubit) — but here each half of the
            // pattern must bind to a whole *sublist* of qubits instead.
            Expr::Let {
                pat: (Pat::Tuple(pats), _),
                rhs,
                body,
            } if pats.len() == 2 && is_split_call(rhs) => {
                let (k_expr, q_expr) = split_call_args(rhs)?;
                let Expr::Int(k) = k_expr.0 else {
                    return Err(LowerError::Unsupported {
                        construct: "split() count must be a literal integer",
                    });
                };
                let qubits = self.eval(&q_expr, block, env)?;
                if k < 0 || k as usize > qubits.len() {
                    return Err(LowerError::Unsupported {
                        construct: "split() count out of range",
                    });
                }
                let (hi, lo) = qubits.split_at(k as usize);
                bind_pattern(&pats[0], hi.to_vec(), env)?;
                bind_pattern(&pats[1], lo.to_vec(), env)?;
                self.lower_monadic(body, block, env)
            }
            Expr::Let { pat, rhs, body } => {
                let values = self.eval(rhs, block, env)?;
                bind_pattern(pat, values, env)?;
                self.lower_monadic(body, block, env)
            }
            Expr::Return(inner) => self.eval(inner, block, env),
            // A trailing monadic action used as the block's result expression.
            _ => self.eval(expr, block, env),
        }
    }

    /// Evaluate a run-block expression to the SSA values it produces, emitting any
    /// staging ops it performs. Circuit application (`<-`), pure value forms
    /// (`let` rhs, `@` operands, `return`), and measurement all flow through here;
    /// the monad's purity distinction is erased at the IR level since every form
    /// either threads qubit wires or produces classical bits.
    fn eval(
        &mut self,
        expr: &Sp<Expr>,
        block: &mut Block<'c>,
        env: &mut HashMap<Name, Vec<Value<'c, 'c>>>,
    ) -> Result<Vec<Value<'c, 'c>>, LowerError> {
        match &expr.0 {
            Expr::Var(name) => env.get(name).cloned().ok_or(LowerError::Unsupported {
                construct: "unbound run-block variable",
            }),
            Expr::Tuple(items) => {
                let mut out = Vec::new();
                for item in items {
                    out.extend(self.eval(item, block, env)?);
                }
                Ok(out)
            }
            // `circuit @ qubits` — apply a circuit to qubits. A direct callee
            // lowers to `apply`; an `if cond then C1 else C2` head lowers to the
            // feed-forward `cond_apply`.
            Expr::GateApp { gate, qubits } => {
                let qs = self.eval(qubits, block, env)?;
                if let Expr::If { cond, then, else_ } = &gate.0 {
                    let condition = single_value(self.eval(cond, block, env)?)?;
                    let then_callee = circuit_callee(then).ok_or(LowerError::Unsupported {
                        construct: "conditional `then` branch must be a named circuit",
                    })?;
                    let else_callee = circuit_callee(else_).ok_or(LowerError::Unsupported {
                        construct: "conditional `else` branch must be a named circuit",
                    })?;
                    let op = block.append_operation(staging::cond_apply(
                        self.context,
                        condition,
                        &then_callee,
                        &else_callee,
                        &qs,
                        self.location,
                    ));
                    return collect_results(op, qs.len());
                }
                let callee = match circuit_callee(gate) {
                    Some(name) => name,
                    None => self.resolve_circuit_callee(gate)?,
                };
                let op = block.append_operation(staging::apply(
                    self.context,
                    &callee,
                    &qs,
                    self.location,
                ));
                collect_results(op, qs.len())
            }
            Expr::App(f, x) => {
                let (head, args) = flatten_app(f, x);
                if let Expr::Var(fname) = &head.0 {
                    match fname.as_str() {
                        // `qreg(n)` — allocate `n` fresh qubits.
                        "qreg" if args.len() == 1 => {
                            if let Expr::Int(count) = &args[0].0 {
                                let op = block.append_operation(staging::qreg(
                                    self.context,
                                    *count,
                                    self.location,
                                ));
                                return collect_results(op, *count as usize);
                            }
                        }
                        // `measure(q)` — consume one qubit, produce one bit.
                        "measure" if args.len() == 1 => {
                            let qubit = single_value(self.eval(args[0], block, env)?)?;
                            let op = block.append_operation(staging::measure(
                                self.context,
                                qubit,
                                self.location,
                            ));
                            return collect_results(op, 1);
                        }
                        // `a `tensored` b` (SPEC §5): concatenate two
                        // registers' wire lists into one, `QReg<n+m>`. No
                        // MLIR op of its own — the wires are already live
                        // SSA values, so this is exactly list concatenation.
                        "tensored" if args.len() == 2 => {
                            let mut wires = self.eval(args[0], block, env)?;
                            wires.extend(self.eval(args[1], block, env)?);
                            return Ok(wires);
                        }
                        // `measure_all(qs)` — measure every qubit, producing one
                        // bit each (Bernstein–Vazirani readout).
                        "measure_all" if args.len() == 1 => {
                            let qubits = self.eval(args[0], block, env)?;
                            let mut bits = Vec::with_capacity(qubits.len());
                            for qubit in qubits {
                                let op = block.append_operation(staging::measure(
                                    self.context,
                                    qubit,
                                    self.location,
                                ));
                                bits.extend(collect_results(op, 1)?);
                            }
                            return Ok(bits);
                        }
                        _ => {}
                    }
                }
                Err(LowerError::Unsupported {
                    construct: "run-block expression",
                })
            }
            _ => Err(LowerError::Unsupported {
                construct: "run-block expression",
            }),
        }
    }

    fn lower_circuit_block(
        &mut self,
        stmts: &[Sp<Stmt>],
        block: &mut Block<'c>,
        wires: &mut Vec<Value<'c, 'c>>,
    ) -> Result<(), LowerError> {
        let Some((last, leading)) = stmts.split_last() else {
            return Err(LowerError::Unsupported {
                construct: "empty circuit block",
            });
        };
        let mut locals: HashMap<Name, Sp<Expr>> = HashMap::new();
        for stmt in leading {
            let Stmt::Let { pat, rhs } = &stmt.0 else {
                return Err(LowerError::Unsupported {
                    construct: "non-let statement inside a circuit block",
                });
            };
            if let crate::ast::Pat::Var(var) = &pat.0 {
                locals.insert(var.clone(), rhs.clone());
            }
        }
        let Stmt::Expr(expr) = &last.0 else {
            return Err(LowerError::Unsupported {
                construct: "circuit block not ending in an expression",
            });
        };
        self.lower_circuit_body_expr_with_locals(&expr.0, block, wires, &locals)
    }

    fn lower_circuit_body_expr_with_locals(
        &mut self,
        expr: &Expr,
        block: &mut Block<'c>,
        wires: &mut Vec<Value<'c, 'c>>,
        locals: &HashMap<Name, Sp<Expr>>,
    ) -> Result<(), LowerError> {
        match expr {
            // A circuit function's own body is a `circuit { .. }` block, but
            // only its top-level caller (`emit_circuit_func`) unwraps that —
            // inlining a *call* to it (e.g. `oracle()` composed into another
            // circuit expression) reaches this walker directly on the callee's
            // un-unwrapped `CircuitBlock`, so it must be handled here too.
            Expr::CircuitBlock(stmts) => self.lower_circuit_block(stmts, block, wires),
            Expr::Compose(lhs, rhs) => {
                self.lower_circuit_body_expr_with_locals(&lhs.0, block, wires, locals)?;
                self.lower_circuit_body_expr_with_locals(&rhs.0, block, wires, locals)
            }
            Expr::GateApp { gate, qubits } => {
                // `controlled(c)` / `Rzz(θ)` have no native circ ops — rewrite via
                // the elaborator (issue #182 / existing Rzz path) before apply.
                if needs_gate_elaboration(gate) {
                    let sp = (
                        Expr::GateApp {
                            gate: gate.clone(),
                            qubits: qubits.clone(),
                        },
                        gate.1,
                    );
                    let mut fuel = elaborate::fresh_fuel();
                    let ctx = elaborate::ElabCtx {
                        parametric: self
                            .parametric
                            .iter()
                            .map(|(k, v)| (k.clone(), v.clone()))
                            .collect(),
                    };
                    let elaborated =
                        elaborate::elaborate_circuit_body(&sp, &HashMap::new(), &ctx, &mut fuel)?;
                    return self.lower_circuit_body_expr_with_locals(
                        &elaborated.0,
                        block,
                        wires,
                        locals,
                    );
                }
                self.apply_gate(gate, qubits, block, wires)
            }
            Expr::Adjoint(inner) => {
                let name = zero_arg_callee_name(inner).ok_or(LowerError::Unsupported {
                    construct: "adjoint of non-call",
                })?;
                self.inline_circuit_body(&name, block, wires, true)
            }
            Expr::App(f, x) => {
                let name = zero_arg_callee_name_from_app(f, x).ok_or(LowerError::Unsupported {
                    construct: "indirect circuit call",
                })?;
                self.inline_circuit_body(&name, block, wires, false)
            }
            Expr::Var(name) => {
                let bound = locals
                    .get(name)
                    .cloned()
                    .or_else(|| self.bodies.get(name).cloned());
                if let Some(body) = bound {
                    self.lower_circuit_body_expr_with_locals(&body.0, block, wires, locals)
                } else {
                    Err(LowerError::Unsupported {
                        construct: "circuit variable",
                    })
                }
            }
            _ => Err(LowerError::Unsupported {
                construct: "circuit body expression",
            }),
        }
    }

    fn lower_circuit_body_expr(
        &mut self,
        expr: &Expr,
        block: &mut Block<'c>,
        wires: &mut Vec<Value<'c, 'c>>,
    ) -> Result<(), LowerError> {
        self.lower_circuit_body_expr_with_locals(expr, block, wires, &HashMap::new())
    }

    fn inline_circuit_body(
        &mut self,
        name: &str,
        block: &mut Block<'c>,
        wires: &mut Vec<Value<'c, 'c>>,
        invert: bool,
    ) -> Result<(), LowerError> {
        let body = self
            .bodies
            .get(name)
            .cloned()
            .ok_or_else(|| LowerError::UnknownCallee {
                name: name.to_string(),
            })?;
        if invert {
            self.inline_inverted_body(&body, block, wires)
        } else {
            self.lower_circuit_body_expr(&body.0, block, wires)
        }
    }

    fn inline_inverted_body(
        &mut self,
        body: &Sp<Expr>,
        block: &mut Block<'c>,
        wires: &mut Vec<Value<'c, 'c>>,
    ) -> Result<(), LowerError> {
        let gates = collect_gate_placements(body)?;
        for (gate, qubits) in gates.into_iter().rev() {
            let spec = self.gate_spec(&gate)?;
            let inv_name = inverse_gate_name(&spec.name);
            let inv_gate = (Expr::Var(inv_name), gate.1);
            self.apply_gate(&inv_gate, &qubits, block, wires)?;
        }
        Ok(())
    }

    fn apply_gate(
        &mut self,
        gate: &Sp<Expr>,
        qubits: &Sp<Expr>,
        block: &mut Block<'c>,
        wires: &mut Vec<Value<'c, 'c>>,
    ) -> Result<(), LowerError> {
        let spec = self.gate_spec(gate)?;
        let targets = qubit_targets(qubits);
        for &target in &targets {
            self.ensure_wire(wires, target, block)?;
        }
        let operands: Vec<Value<'c, '_>> = targets.iter().map(|&idx| wires[idx]).collect();
        let op = if let Some(angle) = spec.angle {
            qc::rotation_gate(
                self.context,
                &spec.name,
                angle,
                spec.depth_contribution,
                spec.clifford,
                wires[targets[0]],
                self.location,
            )?
        } else {
            qc::gate(
                self.context,
                &spec.name,
                spec.depth_contribution,
                spec.clifford,
                &operands,
                self.location,
            )?
        };
        let appended = block.append_operation(op);
        for (i, &idx) in targets.iter().enumerate() {
            let out = appended
                .result(i)
                .map_err(|_| LowerError::Internal("missing gate result"))?;
            wires[idx] = Value::from(out);
        }
        Ok(())
    }

    fn ensure_wire(
        &mut self,
        wires: &mut Vec<Value<'c, 'c>>,
        index: usize,
        block: &mut Block<'c>,
    ) -> Result<(), LowerError> {
        while wires.len() <= index {
            let borrow_region = Region::new();
            borrow_region.append_block(Block::new(&[]));
            let borrow = qc::borrow(
                self.context,
                1,
                &DepthExpr::zero(),
                borrow_region,
                self.location,
            )?;
            let appended = block.append_operation(borrow);
            let borrowed = appended
                .result(0)
                .map_err(|_| LowerError::Internal("missing borrowed qubit"))?;
            wires.push(Value::from(borrowed));
        }
        Ok(())
    }

    fn gate_spec(&self, gate: &Sp<Expr>) -> Result<GateSpec, LowerError> {
        match &gate.0 {
            Expr::Var(name) => {
                let ty = circuit::gate_type(name)
                    .ok_or_else(|| LowerError::UnknownGate { name: name.clone() })?;
                let (clifford, depth) = circuit_meta(&ty);
                Ok(GateSpec {
                    name: name.clone(),
                    angle: None,
                    clifford,
                    depth_contribution: depth,
                })
            }
            Expr::App(f, x) => {
                let (head, args) = flatten_app(f, x);
                let Expr::Var(name) = &head.0 else {
                    return Err(LowerError::Unsupported {
                        construct: "indirect rotation gate",
                    });
                };
                if circuit::rotation_arity(name).is_none() {
                    return Err(LowerError::UnknownGate { name: name.clone() });
                }
                let Expr::Float(angle) = &args[0].0 else {
                    return Err(LowerError::NonStaticRotation { name: name.clone() });
                };
                let ty = circuit::gate_type(name)
                    .ok_or_else(|| LowerError::UnknownGate { name: name.clone() })?;
                let (clifford, depth) = circuit_meta(&ty);
                Ok(GateSpec {
                    name: name.clone(),
                    angle: Some(*angle),
                    clifford,
                    depth_contribution: depth,
                })
            }
            _ => Err(LowerError::Unsupported {
                construct: "gate expression",
            }),
        }
    }
}

fn circuit_ref_op<'c>(
    context: &'c Context,
    name: &str,
    depth: &DepthExpr,
    clifford: bool,
    location: Location<'c>,
) -> Result<Operation<'c>, qc::BuildError> {
    let operation = OperationBuilder::new("quantum.circ.ref", location)
        .add_results(&[qc::circuit_type(context)])
        .add_attributes(&[
            (
                Identifier::new(context, qc::attr::SYM_NAME),
                StringAttribute::new(context, name).into(),
            ),
            (
                Identifier::new(context, qc::attr::DEPTH),
                StringAttribute::new(context, &depth.to_sexpr()).into(),
            ),
            (
                Identifier::new(context, qc::attr::CLIFFORD),
                BoolAttribute::new(context, clifford).into(),
            ),
        ])
        .build()?;
    qc::verify(&operation)?;
    Ok(operation)
}

fn collect_gate_placements(expr: &Sp<Expr>) -> Result<Vec<GatePlacement>, LowerError> {
    match &expr.0 {
        Expr::CircuitBlock(stmts) => {
            let Some(Stmt::Expr(last)) = stmts.last().map(|s| &s.0) else {
                return Err(LowerError::Unsupported {
                    construct: "empty circuit block",
                });
            };
            collect_gate_placements(last)
        }
        Expr::Compose(lhs, rhs) => {
            let mut out = collect_gate_placements(lhs)?;
            out.extend(collect_gate_placements(rhs)?);
            Ok(out)
        }
        Expr::GateApp { gate, qubits } => Ok(vec![(*gate.clone(), *qubits.clone())]),
        _ => Err(LowerError::Unsupported {
            construct: "gate placement collection",
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

fn const_width(depth: &DepthExpr, field: &'static str) -> Result<i64, LowerError> {
    depth
        .as_const()
        .and_then(|n| i64::try_from(n).ok())
        .ok_or(LowerError::NonConstWidth { field })
}

fn circuit_meta(ty: &Ty) -> (bool, i64) {
    match ty {
        Ty::Fn(_, body) => circuit_meta(body),
        Ty::Circuit { d, c, .. } => (
            matches!(c, CliffordClass::Clifford),
            i64::try_from(d.as_const().unwrap_or(1)).unwrap_or(1),
        ),
        _ => (true, 1),
    }
}

fn qubit_targets(qubits: &Sp<Expr>) -> Vec<usize> {
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

/// The highest qubit index a fully elaborated (`Compose`/`GateApp`/`Adjoint`)
/// circuit expression places a gate on, or `None` if it places none — used to
/// size a synthesized `quantum.circ.func` for an anonymous circuit expression
/// with no declared `Circuit<n,...>` type of its own to read a width from.
fn max_qubit_index(expr: &Sp<Expr>) -> Option<usize> {
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

/// The number of gate placements in a fully elaborated circuit expression —
/// a crude but safe depth over-estimate for a synthesized anonymous function
/// (see `resolve_circuit_callee`).
fn count_gates(expr: &Sp<Expr>) -> usize {
    match &expr.0 {
        Expr::Compose(lhs, rhs) => count_gates(lhs) + count_gates(rhs),
        Expr::GateApp { .. } => 1,
        Expr::Adjoint(inner) => count_gates(inner),
        _ => 0,
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

/// Whether `expr` is a call to the `split` builtin (`split(k, q)`) — used to
/// route a `let (hi, lo) = split(k, q)` binding through its own handling in
/// `lower_monadic` rather than the ordinary `eval` + `bind_pattern` path.
fn is_split_call(expr: &Sp<Expr>) -> bool {
    match &expr.0 {
        Expr::App(f, x) => {
            let (head, args) = flatten_app(f, x);
            matches!(&head.0, Expr::Var(name) if name == "split") && args.len() == 2
        }
        _ => false,
    }
}

/// Gates that `elaborate_circuit_body` rewrites before `quantum.circ` emission
/// (`controlled(c)`, `Rzz(θ)`).
fn needs_gate_elaboration(gate: &Sp<Expr>) -> bool {
    match &gate.0 {
        Expr::Controlled(_) => true,
        Expr::App(f, x) => {
            let (head, args) = flatten_app(f, x);
            matches!(&head.0, Expr::Var(name) if name == "Rzz") && args.len() == 1
        }
        _ => false,
    }
}

/// The `(k, q)` arguments of a `split(k, q)` call already confirmed by
/// [`is_split_call`].
fn split_call_args(expr: &Sp<Expr>) -> Result<(Sp<Expr>, Sp<Expr>), LowerError> {
    let Expr::App(f, x) = &expr.0 else {
        return Err(LowerError::Internal("split_call_args on a non-App"));
    };
    let (_, args) = flatten_app(f, x);
    let [k, q] = args[..] else {
        return Err(LowerError::Internal("split_call_args arity"));
    };
    Ok((k.clone(), q.clone()))
}

fn zero_arg_callee_name(expr: &Sp<Expr>) -> Option<String> {
    let (head, args) = match &expr.0 {
        Expr::App(f, x) => flatten_app(f, x),
        _ => return None,
    };
    if !is_unit_args(&args) {
        return None;
    }
    match &head.0 {
        Expr::Var(name) => Some(name.clone()),
        _ => None,
    }
}

fn zero_arg_callee_name_from_app(f: &Sp<Expr>, x: &Sp<Expr>) -> Option<String> {
    let (head, args) = flatten_app(f, x);
    if !is_unit_args(&args) {
        return None;
    }
    match &head.0 {
        Expr::Var(name) => Some(name.clone()),
        _ => None,
    }
}

fn is_unit_args(args: &[&Sp<Expr>]) -> bool {
    match args.len() {
        0 => true,
        1 => matches!(args[0].0, Expr::Unit),
        _ => false,
    }
}

/// The circuit-function name a `@` left-hand side applies, if it is a direct
/// reference (`circuit` or `circuit()`). Adjoint / conditional forms return
/// `None` so the caller reports them as unsupported.
fn circuit_callee(gate: &Sp<Expr>) -> Option<String> {
    match &gate.0 {
        Expr::Var(name) => Some(name.clone()),
        Expr::App(f, x) => zero_arg_callee_name_from_app(f, x),
        _ => None,
    }
}

/// Collect the first `count` results of a freshly-appended staging op as values.
fn collect_results<'c>(
    op: OperationRef<'c, 'c>,
    count: usize,
) -> Result<Vec<Value<'c, 'c>>, LowerError> {
    (0..count)
        .map(|i| {
            op.result(i)
                .map(Value::from)
                .map_err(|_| LowerError::Internal("missing staging op result"))
        })
        .collect()
}

/// Unwrap a single-value result list (e.g. the qubit operand of `measure`).
fn single_value<'c>(values: Vec<Value<'c, 'c>>) -> Result<Value<'c, 'c>, LowerError> {
    match values.as_slice() {
        [value] => Ok(*value),
        _ => Err(LowerError::Unsupported {
            construct: "expected a single qubit",
        }),
    }
}

/// Bind a `let` pattern to evaluated values, destructuring tuples positionally.
fn bind_pattern<'c>(
    pat: &Sp<Pat>,
    values: Vec<Value<'c, 'c>>,
    env: &mut HashMap<Name, Vec<Value<'c, 'c>>>,
) -> Result<(), LowerError> {
    match &pat.0 {
        Pat::Var(name) => {
            env.insert(name.clone(), values);
            Ok(())
        }
        Pat::Wildcard => Ok(()),
        Pat::Tuple(pats) => {
            if pats.len() != values.len() {
                return Err(LowerError::Unsupported {
                    construct: "tuple pattern arity mismatch in run block",
                });
            }
            for (p, value) in pats.iter().zip(values) {
                bind_pattern(p, vec![value], env)?;
            }
            Ok(())
        }
        Pat::Lit(_) => Err(LowerError::Unsupported {
            construct: "literal pattern in run-block let",
        }),
    }
}

/// Parse, desugar, type-check, and lower `src` to an in-memory `quantum.circ` module.
pub fn lower_program<'c>(context: &'c Context, src: &str) -> Result<Module<'c>, Vec<Diagnostic>> {
    let decls = crate::desugar_program(src)?;
    let mut lowering = LoweringCtx::new(context);
    lowering.lower_decls(&decls).map_err(|err| {
        // Only `LowerError::Type` carries a real source span (from the type
        // checker); the other variants are lowering-stage internal errors
        // with no natural token to anchor on.
        let span = match &err {
            LowerError::Type(type_error) => type_error.span(),
            LowerError::Elab(elab_error) => elab_error.span(),
            _ => chumsky::span::SimpleSpan::from(0..0),
        };
        vec![Diagnostic::new(err.to_string(), span)]
    })?;
    Ok(lowering.into_module())
}

/// Lower `decls` that have already been desugared.
pub fn lower_checked_decls<'c>(
    context: &'c Context,
    decls: &[Sp<Decl>],
) -> Result<Module<'c>, LowerError> {
    let mut lowering = LoweringCtx::new(context);
    lowering.lower_decls(decls)?;
    Ok(lowering.into_module())
}
