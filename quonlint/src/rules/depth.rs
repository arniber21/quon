use frontend::ast::{Decl, Expr};
use frontend::lexer::Sp;
use frontend::types::Ty;

use crate::context::{LintContext, is_literal_int, walk_expr};
use crate::diagnostic::{LintDiagnostic, Severity};
use crate::rules::{LintRule, emit_if_allowed};

pub struct SequentialForBlowup;

impl LintRule for SequentialForBlowup {
    fn id(&self) -> String {
        "depth/sequential-for-blowup".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Info
    }

    fn description(&self) -> &'static str {
        "Sequential `for` inside a circuit block multiplies depth by iteration count"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        let mut visit = |ctx: &LintContext<'_>, expr: &Sp<Expr>| {
            if !ctx.in_circuit() {
                return;
            }
            let Expr::For { iter, body, .. } = &expr.0 else {
                return;
            };
            let mut severity = default;
            if !is_literal_int(&iter.0) && body_depth_hint(&body.0) > 1 {
                severity = Severity::Warning;
            }
            let span = expr.1;
            let diag = LintDiagnostic::new(
                &rule,
                severity,
                "sequential `for` multiplies circuit depth by iteration count",
                span,
            )
            .with_help(
                "consider `par` if layers commute, or annotate explicit depth `n_steps * d_layer`",
            );
            emit_if_allowed(ctx, &rule, severity, diag, emit);
        };
        for decl in &ctx.typed.decls {
            walk_fn_body(ctx, decl, &mut visit);
        }
    }
}

pub struct UnsoundDepthAnnotation;

impl LintRule for UnsoundDepthAnnotation {
    fn id(&self) -> String {
        "depth/unsound-depth-annotation".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Warning
    }

    fn description(&self) -> &'static str {
        "Recursive circuit function's synthesized step depth exceeds declared depth bound"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        for (name, ty) in &ctx.typed.fn_types {
            let Ty::Circuit { d, .. } = ty else {
                continue;
            };
            if let Some(body_ty) = synthesized_return_type(ctx, name)
                && let Ty::Circuit { d: synth_d, .. } = body_ty
                && depth_exceeds_declared(d, &synth_d)
                && let Some(span) = fn_name_span(ctx, name)
            {
                let diag = LintDiagnostic::new(
                    &rule,
                    default,
                    format!(
                        "function `{name}` synthesized depth `{synth_d}` exceeds declared bound `{d}`"
                    ),
                    span,
                )
                .with_help("tighten the depth annotation or refactor the recursive step");
                emit_if_allowed(ctx, &rule, default, diag, emit);
            }
        }
    }
}

pub struct RepeatNonLiteralCount;

impl LintRule for RepeatNonLiteralCount {
    fn id(&self) -> String {
        "depth/repeat-non-literal-count".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Info
    }

    fn description(&self) -> &'static str {
        "`par { c } * count` or `repeat(count, c)` with non-literal count makes depth symbolic"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        let rule = self.id();
        let default = self.default_severity();
        let mut visit = |ctx: &LintContext<'_>, expr: &Sp<Expr>| {
            let Expr::Par(_, count) = &expr.0 else {
                return;
            };
            if is_literal_int(&count.0) {
                return;
            }
            let diag = LintDiagnostic::new(
                &rule,
                default,
                "`par { } * count` with non-literal count yields symbolic depth",
                expr.1,
            )
            .with_help("use a literal repeat count when possible for predictable depth bounds");
            emit_if_allowed(ctx, &rule, default, diag, emit);
        };
        for decl in &ctx.typed.decls {
            walk_fn_body(ctx, decl, &mut visit);
        }
    }
}

pub struct ControlledChain;

impl LintRule for ControlledChain {
    fn id(&self) -> String {
        "depth/controlled-chain".into()
    }

    fn default_severity(&self) -> Severity {
        Severity::Info
    }

    fn description(&self) -> &'static str {
        "Long chain of `controlled(...)` compositions may indicate missed `par` opportunities"
    }

    fn run(&self, ctx: &LintContext<'_>, emit: &mut dyn FnMut(LintDiagnostic)) {
        const THRESHOLD: u32 = 4;
        let rule = self.id();
        let default = self.default_severity();
        let mut visit = |ctx: &LintContext<'_>, expr: &Sp<Expr>| {
            if !ctx.in_circuit() {
                return;
            }
            let depth = controlled_depth(&expr.0);
            if depth >= THRESHOLD {
                let diag = LintDiagnostic::new(
                    &rule,
                    default,
                    format!(
                        "chain of {depth} nested `controlled(...)` wrappers adds +{depth} depth"
                    ),
                    expr.1,
                )
                .with_help("consider parallelizing independent controlled rotations");
                emit_if_allowed(ctx, &rule, default, diag, emit);
            }
        };
        for decl in &ctx.typed.decls {
            walk_fn_body(ctx, decl, &mut visit);
        }
    }
}

fn walk_fn_body(
    ctx: &LintContext<'_>,
    decl: &Sp<Decl>,
    visit: &mut dyn FnMut(&LintContext<'_>, &Sp<Expr>),
) {
    let Decl::Fn { body, .. } = &decl.0 else {
        return;
    };
    walk_expr(ctx, body, visit);
}

fn body_depth_hint(expr: &Expr) -> u32 {
    match expr {
        Expr::CircuitBlock(stmts) => stmts
            .iter()
            .map(|s| match &s.0 {
                frontend::ast::Stmt::Expr(e) => body_depth_hint(&e.0),
                _ => 1,
            })
            .sum(),
        Expr::Compose(a, b) => body_depth_hint(&a.0).saturating_add(body_depth_hint(&b.0)),
        Expr::For { body, .. } => body_depth_hint(&body.0),
        _ => 1,
    }
}

fn controlled_depth(expr: &Expr) -> u32 {
    match expr {
        Expr::Controlled(inner) => 1 + controlled_depth(&inner.0),
        _ => 0,
    }
}

fn fn_name_span(ctx: &LintContext<'_>, name: &str) -> Option<frontend::lexer::SimpleSpan> {
    for decl in &ctx.typed.decls {
        if let Decl::Fn { name: n, .. } = &decl.0
            && n.0 == name
        {
            return Some(n.1);
        }
    }
    None
}

fn synthesized_return_type(ctx: &LintContext<'_>, name: &str) -> Option<Ty> {
    for decl in &ctx.typed.decls {
        if let Decl::Fn { name: n, body, .. } = &decl.0
            && n.0 == name
        {
            return ctx.expr_type(body.1).cloned();
        }
    }
    None
}

/// Conservative check: only flag when both are constant and synth > declared.
fn depth_exceeds_declared(
    declared: &quon_core::DepthExpr,
    synthesized: &quon_core::DepthExpr,
) -> bool {
    use quon_core::DepthExpr;
    match (declared, synthesized) {
        (DepthExpr::Nat(d), DepthExpr::Nat(s)) => s > d,
        _ => false,
    }
}
