use std::collections::HashSet;

use frontend::ast::{Expr, Pat, Stmt};
use frontend::lexer::Sp;

use crate::context::{
    LintContext, callee_name, collect_borrow_ancillae, is_entangling_gate, walk_fn_bodies,
    walk_stmts,
};
use crate::diagnostic::{LintDiagnostic, Severity};
use crate::rules::{LintRule, emit_if_allowed};

pub struct DiscardInBorrow;

impl LintRule for DiscardInBorrow {
    fn id(&self) -> String {
        "ancilla/discard-in-borrow".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn description(&self) -> &'static str {
        "`borrow` block ends with `discard` on an ancilla used in entangling gates"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        let mut visit = |ctx: &LintContext<'_>, expr: &Sp<Expr>| {
            let Expr::Borrow { bindings, body } = &expr.0 else {
                return;
            };
            let ancillae = collect_borrow_ancillae(bindings);
            let mut entangled: HashSet<String> = HashSet::new();
            let mut discarded: Vec<(String, frontend::lexer::SimpleSpan)> = Vec::new();

            ctx.with_borrow(|nested| {
                let mut scan = |_ctx: &LintContext<'_>, e: &Sp<Expr>| {
                    if let Expr::GateApp { gate, .. } = &e.0
                        && let Some(name) = callee_name(&gate.0)
                        && is_entangling_gate(name)
                    {
                        entangled.extend(ancillae.iter().cloned());
                    }
                    if is_discard_call(&e.0)
                        && let Some(var) = discard_target(&e.0)
                        && ancillae.contains(&var)
                    {
                        discarded.push((var, e.1));
                    }
                };
                walk_stmts(nested, body, &mut scan);
            });

            for (var, span) in discarded {
                if entangled.contains(&var) {
                    let diag = LintDiagnostic::new(
                        &rule,
                        default,
                        format!("`discard({var})` on ancilla used in entangling gates"),
                        span,
                    )
                    .with_help("prefer `reset` when reuse is intended (SPEC §5.9)");
                    emit_if_allowed(ctx, &rule, default, diag, emit);
                }
            }
        };
        walk_fn_bodies(ctx, &mut visit);
    }
}

pub struct NestedBorrow;

impl LintRule for NestedBorrow {
    fn id(&self) -> String {
        "ancilla/nested-borrow".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Info
    }

    fn description(&self) -> &'static str {
        "Nested `borrow` blocks are harder to reason about"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        let mut visit = |ctx: &LintContext<'_>, expr: &Sp<Expr>| {
            if ctx.borrow_depth() >= 1 && matches!(expr.0, Expr::Borrow { .. }) {
                let diag = LintDiagnostic::new(&rule, default, "nested `borrow` block", expr.1)
                    .with_help("flatten ancilla scopes or extract a helper function");
                emit_if_allowed(ctx, &rule, default, diag, emit);
            }
        };
        walk_fn_bodies(ctx, &mut visit);
    }
}

pub struct UnmeasuredAncillaOutput;

impl LintRule for UnmeasuredAncillaOutput {
    fn id(&self) -> String {
        "ancilla/unmeasured-ancilla-output".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn description(&self) -> &'static str {
        "Ancilla from `borrow` appears in return without measure/reset/discard"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        let mut visit = |ctx: &LintContext<'_>, expr: &Sp<Expr>| {
            let Expr::Borrow { bindings, body } = &expr.0 else {
                return;
            };
            let ancillae = collect_borrow_ancillae(bindings);
            let mut finalized: HashSet<String> = HashSet::new();
            let mut returned_ancillae: Vec<(String, frontend::lexer::SimpleSpan)> = Vec::new();

            for stmt in body {
                match &stmt.0 {
                    Stmt::Bind { pat, rhs } | Stmt::Let { pat, rhs } => {
                        if is_measure_call(&rhs.0)
                            && let Some(name) = pat_var_name(pat)
                        {
                            finalized.insert(name);
                        }
                        if is_reset_or_discard_call(&rhs.0)
                            && let Some(name) = call_target(&rhs.0)
                        {
                            finalized.insert(name);
                        }
                    }
                    Stmt::Expr(e) => {
                        if let Expr::Return(ret) = &e.0 {
                            collect_ancilla_in_expr(
                                &ret.0,
                                &ancillae,
                                &mut returned_ancillae,
                                ret.1,
                            );
                        }
                    }
                }
            }

            for (name, span) in returned_ancillae {
                if !finalized.contains(&name) {
                    let diag = LintDiagnostic::new(
                        &rule,
                        default,
                        format!("ancilla `{name}` returned without measure/reset/discard"),
                        span,
                    )
                    .with_help("measure, reset, or discard ancilla before returning it");
                    emit_if_allowed(ctx, &rule, default, diag, emit);
                }
            }
        };
        walk_fn_bodies(ctx, &mut visit);
    }
}

fn is_discard_call(expr: &Expr) -> bool {
    callee_name(expr) == Some("discard")
}

fn is_reset_or_discard_call(expr: &Expr) -> bool {
    matches!(callee_name(expr), Some("discard") | Some("reset"))
}

fn is_measure_call(expr: &Expr) -> bool {
    matches!(callee_name(expr), Some("measure") | Some("measure_all"))
}

fn discard_target(expr: &Expr) -> Option<String> {
    match expr {
        Expr::App(f, arg) if callee_name(&f.0) == Some("discard") => match &arg.0 {
            Expr::Var(n) => Some(n.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn call_target(expr: &Expr) -> Option<String> {
    match expr {
        Expr::App(_, arg) => match &arg.0 {
            Expr::Var(n) => Some(n.clone()),
            _ => None,
        },
        _ => None,
    }
}

fn pat_var_name(pat: &Sp<Pat>) -> Option<String> {
    match &pat.0 {
        Pat::Var(n) => Some(n.clone()),
        _ => None,
    }
}

fn collect_ancilla_in_expr(
    expr: &Expr,
    ancillae: &HashSet<String>,
    out: &mut Vec<(String, frontend::lexer::SimpleSpan)>,
    span: frontend::lexer::SimpleSpan,
) {
    match expr {
        Expr::Var(n) if ancillae.contains(n) => out.push((n.clone(), span)),
        Expr::Tuple(es) | Expr::List(es) => {
            for e in es {
                collect_ancilla_in_expr(&e.0, ancillae, out, e.1);
            }
        }
        _ => {}
    }
}
