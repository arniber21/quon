use frontend::ast::{Expr, Pat, Stmt};
use frontend::lexer::Sp;
use frontend::types::Ty;

use crate::context::{LintContext, callee_name, walk_fn_bodies};
use crate::diagnostic::{LintDiagnostic, Severity};
use crate::rules::{LintRule, emit_if_allowed};

pub struct CircuitBindWithoutApply;

impl LintRule for CircuitBindWithoutApply {
    fn id(&self) -> String {
        "monad/circuit-bind-without-apply".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Error
    }

    fn description(&self) -> &'static str {
        "In `run { }`, binding a `Circuit` value without applying it to qubits"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        let mut visit = |ctx: &LintContext<'_>, expr: &Sp<Expr>| {
            if let Expr::RunBlock(stmts) = &expr.0 {
                check_run_stmts(ctx, stmts, &rule, default, emit);
            }
        };
        walk_fn_bodies(ctx, &mut visit);
    }
}

pub struct NestedRunBlock;

impl LintRule for NestedRunBlock {
    fn id(&self) -> String {
        "monad/nested-run-block".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Info
    }

    fn description(&self) -> &'static str {
        "Nested `run { }` blocks are often accidental"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        let mut visit = |ctx: &LintContext<'_>, expr: &Sp<Expr>| {
            if let Expr::RunBlock(stmts) = &expr.0 {
                for stmt in stmts {
                    if contains_run_block(stmt) {
                        let diag =
                            LintDiagnostic::new(&rule, default, "nested `run { }` block", stmt.1)
                                .with_help("flatten or extract a helper function");
                        emit_if_allowed(ctx, &rule, default, diag, emit);
                    }
                }
            }
        };
        walk_fn_bodies(ctx, &mut visit);
    }
}

pub struct UnusedMeasurement;

impl LintRule for UnusedMeasurement {
    fn id(&self) -> String {
        "monad/unused-measurement".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn description(&self) -> &'static str {
        "Measurement result bound to `_` or dropped"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        let mut visit = |ctx: &LintContext<'_>, expr: &Sp<Expr>| {
            if let Expr::RunBlock(stmts) = &expr.0 {
                check_unused_measurements(ctx, stmts, &rule, default, emit);
            }
        };
        walk_fn_bodies(ctx, &mut visit);
    }
}

fn check_run_stmts(
    ctx: &LintContext<'_>,
    stmts: &[Sp<Stmt>],
    rule: &str,
    default: Severity,
    emit: &mut dyn FnMut(LintDiagnostic),
) {
    for stmt in stmts {
        if let Stmt::Bind { rhs, .. } = &stmt.0
            && bind_rhs_is_unapplied_circuit(ctx, rhs)
        {
            let diag = LintDiagnostic::new(
                rule,
                default,
                "bound a `Circuit` value without applying it to qubits",
                rhs.1,
            )
            .with_help("apply with `@ q` or `apply_dyn` before binding");
            emit_if_allowed(ctx, rule, default, diag, emit);
        }
    }
}

fn check_unused_measurements(
    ctx: &LintContext<'_>,
    stmts: &[Sp<Stmt>],
    rule: &str,
    default: Severity,
    emit: &mut dyn FnMut(LintDiagnostic),
) {
    for stmt in stmts {
        match &stmt.0 {
            Stmt::Bind { pat, rhs, .. } => {
                if is_wildcard_pat(pat) && is_measure_call(&rhs.0) {
                    let diag = LintDiagnostic::new(
                        rule,
                        default,
                        "measurement result bound to `_`",
                        pat.1,
                    )
                    .with_help("bind the measurement to a name if the outcome matters");
                    emit_if_allowed(ctx, rule, default, diag, emit);
                }
            }
            Stmt::Expr(e) if matches!(&e.0, Expr::RunBlock(_)) => {
                if let Expr::RunBlock(inner) = &e.0 {
                    check_unused_measurements(ctx, inner, rule, default, emit);
                }
            }
            _ => {}
        }
    }
}

fn is_measure_call(expr: &Expr) -> bool {
    matches!(callee_name(expr), Some("measure") | Some("measure_all"))
}

fn bind_rhs_is_unapplied_circuit(ctx: &LintContext<'_>, rhs: &Sp<Expr>) -> bool {
    if matches!(rhs.0, Expr::GateApp { .. }) {
        return false;
    }
    if let Some(name) = circuit_callee_name(&rhs.0)
        && matches!(ctx.fn_type(name), Some(Ty::Circuit { .. }))
    {
        return true;
    }
    matches!(ctx.expr_type(rhs.1), Some(Ty::Circuit { .. }))
}

fn circuit_callee_name(expr: &Expr) -> Option<&str> {
    match expr {
        Expr::Var(n) => Some(n.as_str()),
        Expr::App(f, _) => circuit_callee_name(&f.0),
        _ => None,
    }
}

fn is_wildcard_pat(pat: &Sp<Pat>) -> bool {
    matches!(pat.0, Pat::Wildcard)
}

fn contains_run_block(stmt: &Sp<Stmt>) -> bool {
    match &stmt.0 {
        Stmt::Bind { rhs, .. } | Stmt::Let { rhs, .. } => matches!(rhs.0, Expr::RunBlock(_)),
        Stmt::Expr(e) => matches!(e.0, Expr::RunBlock(_)),
    }
}
