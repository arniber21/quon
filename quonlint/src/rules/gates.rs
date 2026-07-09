use frontend::ast::{Decl, Expr};
use frontend::lexer::Sp;
use frontend::types::Ty;

use crate::context::{LintContext, is_swap_gate, is_universal_gate, walk_expr};
use crate::diagnostic::{LintDiagnostic, Severity};
use crate::rules::{LintRule, emit_if_allowed};

pub struct UniversalInCliffordBlock;

impl LintRule for UniversalInCliffordBlock {
    fn id(&self) -> String {
        "gates/universal-in-clifford-block".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn description(&self) -> &'static str {
        "Clifford circuit block contains universal gates (T or parametric rotations)"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        let mut visit = |ctx: &LintContext<'_>, expr: &Sp<Expr>| {
            if !ctx.in_circuit() {
                return;
            }
            let circuit_class = current_circuit_class(ctx, expr);
            if !is_clifford_class(circuit_class) {
                return;
            }
            let Expr::GateApp { gate, .. } = &expr.0 else {
                return;
            };
            let gate_name = gate_name_from_expr(&gate.0);
            if let Some(name) = gate_name
                && is_universal_gate(name)
            {
                let diag = LintDiagnostic::new(
                    &rule,
                    default,
                    format!("universal gate `{name}` in Clifford-classified circuit block"),
                    gate.1,
                )
                .with_help("remove T/rotation gates or widen the Clifford class annotation");
                emit_if_allowed(ctx, &rule, default, diag, emit);
            }
        };
        for decl in &ctx.typed.decls {
            if let Decl::Fn { body, .. } = &decl.0 {
                walk_expr(ctx, body, &mut visit);
            }
        }
    }
}

pub struct ConsecutiveRotations;

impl LintRule for ConsecutiveRotations {
    fn id(&self) -> String {
        "gates/consecutive-rotations".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Info
    }

    fn description(&self) -> &'static str {
        "Consecutive parametric rotations on the same qubit may be mergeable"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        for decl in &ctx.typed.decls {
            let Decl::Fn { body, .. } = &decl.0 else {
                continue;
            };
            if let Expr::CircuitBlock(stmts) = &body.0 {
                check_rotation_runs(ctx, stmts, &rule, default, emit);
            }
        }
    }
}

pub struct SwapInSource;

impl LintRule for SwapInSource {
    fn id(&self) -> String {
        "gates/swap-in-source".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Info
    }

    fn description(&self) -> &'static str {
        "Explicit SWAP in source may be avoidable via routing on connectivity-aware targets"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        let mut visit = |ctx: &LintContext<'_>, expr: &Sp<Expr>| {
            let Expr::GateApp { gate, .. } = &expr.0 else {
                return;
            };
            if let Some(name) = gate_name_from_expr(&gate.0)
                && is_swap_gate(name)
            {
                let diag = LintDiagnostic::new(
                    &rule,
                    default,
                    format!("explicit `{name}` gate in source"),
                    gate.1,
                )
                .with_help("prefer native routing or `swap_reverse` when topology is constrained");
                emit_if_allowed(ctx, &rule, default, diag, emit);
            }
        };
        for decl in &ctx.typed.decls {
            if let Decl::Fn { body, .. } = &decl.0 {
                walk_expr(ctx, body, &mut visit);
            }
        }
    }
}

fn gate_name_from_expr(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Var(n) => Some(n.as_str()),
        Expr::App(f, _) => gate_name_from_expr(&f.0),
        _ => None,
    }
}

fn is_clifford_class(class: Option<frontend::ast::CliffordClass>) -> bool {
    matches!(class, Some(frontend::ast::CliffordClass::Clifford))
}

fn current_circuit_class(
    ctx: &LintContext<'_>,
    expr: &Sp<Expr>,
) -> Option<frontend::ast::CliffordClass> {
    ctx.expr_type(expr.1).and_then(|ty| match ty {
        Ty::Circuit { c, .. } => Some(c.clone()),
        _ => None,
    })
}

fn is_rotation_gate(name: &str) -> bool {
    matches!(name, "Rz" | "Rx" | "Ry")
}

struct RotationWalkState {
    prev_qubit: Option<String>,
    run_start: Option<frontend::lexer::SimpleSpan>,
    run_len: u32,
}

fn check_rotation_runs(
    ctx: &LintContext<'_>,
    stmts: &[Sp<frontend::ast::Stmt>],
    rule: &str,
    default: Severity,
    emit: &mut dyn FnMut(LintDiagnostic),
) {
    let mut state = RotationWalkState {
        prev_qubit: None,
        run_start: None,
        run_len: 0,
    };

    for stmt in stmts {
        match &stmt.0 {
            frontend::ast::Stmt::Expr(e) => {
                walk_rotation_expr(ctx, e, &mut state, rule, default, emit)
            }
            _ => {
                if let Some(start) = state.run_start.take() {
                    flush(ctx, start, state.run_len, rule, default, emit);
                }
                state.prev_qubit = None;
                state.run_len = 0;
            }
        }
    }
    if let Some(start) = state.run_start {
        flush(ctx, start, state.run_len, rule, default, emit);
    }
}

fn walk_rotation_expr(
    ctx: &LintContext<'_>,
    expr: &Sp<Expr>,
    state: &mut RotationWalkState,
    rule: &str,
    default: Severity,
    emit: &mut dyn FnMut(LintDiagnostic),
) {
    match &expr.0 {
        Expr::Compose(a, b) => {
            walk_rotation_expr(ctx, a, state, rule, default, emit);
            walk_rotation_expr(ctx, b, state, rule, default, emit);
        }
        Expr::GateApp { gate, qubits } => {
            record_rotation_gate(ctx, gate, qubits, state, rule, default, emit);
        }
        _ => {
            if let Some(start) = state.run_start.take() {
                flush(ctx, start, state.run_len, rule, default, emit);
            }
            state.prev_qubit = None;
            state.run_len = 0;
        }
    }
}

fn record_rotation_gate(
    ctx: &LintContext<'_>,
    gate: &Sp<Expr>,
    qubits: &Sp<Expr>,
    state: &mut RotationWalkState,
    rule: &str,
    default: Severity,
    emit: &mut dyn FnMut(LintDiagnostic),
) {
    let name = gate_name_from_expr(&gate.0);
    let qubit = qubit_index_string(&qubits.0);
    if let Some(name) = name
        && is_rotation_gate(name)
        && let Some(q) = qubit
    {
        if state.prev_qubit.as_deref() == Some(q.as_str()) {
            state.run_len += 1;
        } else {
            if let Some(start) = state.run_start.take() {
                flush(ctx, start, state.run_len, rule, default, emit);
            }
            state.prev_qubit = Some(q);
            state.run_start = Some(gate.1);
            state.run_len = 1;
        }
    } else if let Some(start) = state.run_start.take() {
        flush(ctx, start, state.run_len, rule, default, emit);
        state.prev_qubit = None;
        state.run_len = 0;
    }
}

fn flush(
    ctx: &LintContext<'_>,
    start: frontend::lexer::SimpleSpan,
    len: u32,
    rule: &str,
    default: Severity,
    emit: &mut dyn FnMut(LintDiagnostic),
) {
    if len >= 2 {
        let diag = LintDiagnostic::new(
            rule,
            default,
            format!("{len} consecutive parametric rotations on the same qubit"),
            start,
        )
        .with_help("rotation_merging pass can combine adjacent rotations");
        emit_if_allowed(ctx, rule, default, diag, emit);
    }
}

fn qubit_index_string(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Int(i) => Some(i.to_string()),
        Expr::Var(n) => Some(n.clone()),
        _ => None,
    }
}
