use std::collections::HashSet;
use std::path::Path;

use frontend::TypedProgram;
use frontend::ast::Decl;
use frontend::lexer::{SimpleSpan, Sp};

use crate::config::LintConfig;
use crate::suppressions::SuppressionState;

/// Per-file lint analysis context.
pub struct LintContext<'a> {
    pub source: &'a str,
    pub path: &'a Path,
    pub typed: &'a TypedProgram,
    pub config: &'a LintConfig,
    pub suppressions: &'a SuppressionState,
    in_circuit: bool,
    borrow_depth: u32,
}

impl<'a> LintContext<'a> {
    pub fn new(
        source: &'a str,
        path: &'a Path,
        typed: &'a TypedProgram,
        config: &'a LintConfig,
        suppressions: &'a SuppressionState,
    ) -> Self {
        Self {
            source,
            path,
            typed,
            config,
            suppressions,
            in_circuit: false,
            borrow_depth: 0,
        }
    }

    fn child(&self, in_circuit: bool, borrow_depth: u32) -> Self {
        Self {
            source: self.source,
            path: self.path,
            typed: self.typed,
            config: self.config,
            suppressions: self.suppressions,
            in_circuit,
            borrow_depth,
        }
    }

    pub fn with_circuit<R>(&self, f: impl FnOnce(&Self) -> R) -> R {
        f(&self.child(true, self.borrow_depth))
    }

    pub fn with_borrow<R>(&self, f: impl FnOnce(&Self) -> R) -> R {
        f(&self.child(self.in_circuit, self.borrow_depth + 1))
    }

    pub fn fn_type(&self, name: &str) -> Option<&frontend::types::Ty> {
        self.typed.fn_type(name)
    }

    pub fn expr_type(&self, span: SimpleSpan) -> Option<&frontend::types::Ty> {
        self.typed.expr_type(span)
    }

    pub fn is_suppressed(&self, rule: &str, span: SimpleSpan) -> bool {
        self.suppressions
            .is_suppressed_with_source(rule, span.start, self.source)
    }

    pub fn in_circuit(&self) -> bool {
        self.in_circuit
    }

    pub fn borrow_depth(&self) -> u32 {
        self.borrow_depth
    }
}

pub fn collect_borrow_ancillae(
    bindings: &[(Sp<frontend::ast::Name>, Sp<frontend::ast::Type>)],
) -> HashSet<String> {
    bindings.iter().map(|(n, _)| n.0.clone()).collect()
}

pub fn callee_name(expr: &frontend::ast::Expr) -> Option<&str> {
    use frontend::ast::Expr;
    match expr {
        Expr::Var(n) => Some(n.as_str()),
        Expr::App(f, _) => callee_name(&f.0),
        _ => None,
    }
}

pub fn is_literal_int(expr: &frontend::ast::Expr) -> bool {
    matches!(expr, frontend::ast::Expr::Int(_))
}

pub fn is_universal_gate(name: &str) -> bool {
    matches!(name, "T" | "T_dag" | "Td" | "Rz" | "Rx" | "Ry" | "U" | "U3")
}

pub fn is_swap_gate(name: &str) -> bool {
    matches!(name, "SWAP" | "swap")
}

pub fn is_entangling_gate(name: &str) -> bool {
    matches!(
        name,
        "CNOT" | "CZ" | "CY" | "CRz" | "CRx" | "CRy" | "SWAP" | "swap"
    )
}

pub fn walk_stmts(
    ctx: &LintContext<'_>,
    stmts: &[Sp<frontend::ast::Stmt>],
    visit: &mut dyn FnMut(&LintContext<'_>, &Sp<frontend::ast::Expr>),
) {
    for stmt in stmts {
        walk_stmt(ctx, stmt, visit);
    }
}

pub fn walk_stmt(
    ctx: &LintContext<'_>,
    stmt: &Sp<frontend::ast::Stmt>,
    visit: &mut dyn FnMut(&LintContext<'_>, &Sp<frontend::ast::Expr>),
) {
    use frontend::ast::Stmt;
    match &stmt.0 {
        Stmt::Bind { rhs, .. } | Stmt::Let { rhs, .. } => visit(ctx, rhs),
        Stmt::Expr(e) => walk_expr(ctx, e, visit),
    }
}

pub fn walk_expr(
    ctx: &LintContext<'_>,
    expr: &Sp<frontend::ast::Expr>,
    visit: &mut dyn FnMut(&LintContext<'_>, &Sp<frontend::ast::Expr>),
) {
    use frontend::ast::Expr;
    visit(ctx, expr);
    match &expr.0 {
        Expr::Lam { body, .. } => walk_expr(ctx, body, visit),
        Expr::App(a, b) => {
            walk_expr(ctx, a, visit);
            walk_expr(ctx, b, visit);
        }
        Expr::BinOp { lhs, rhs, .. } => {
            walk_expr(ctx, lhs, visit);
            walk_expr(ctx, rhs, visit);
        }
        Expr::Neg(e) => walk_expr(ctx, e, visit),
        Expr::Let { rhs, body, .. } => {
            walk_expr(ctx, rhs, visit);
            walk_expr(ctx, body, visit);
        }
        Expr::If { cond, then, else_ } => {
            walk_expr(ctx, cond, visit);
            walk_expr(ctx, then, visit);
            walk_expr(ctx, else_, visit);
        }
        Expr::Match { scrutinee, arms } => {
            walk_expr(ctx, scrutinee, visit);
            for (_, body) in arms {
                walk_expr(ctx, body, visit);
            }
        }
        Expr::For { iter, body, .. } => {
            walk_expr(ctx, iter, visit);
            walk_expr(ctx, body, visit);
        }
        Expr::Tuple(es) | Expr::List(es) => {
            for e in es {
                walk_expr(ctx, e, visit);
            }
        }
        Expr::CircuitBlock(stmts) => {
            ctx.with_circuit(|nested| walk_stmts(nested, stmts, visit));
        }
        Expr::Compose(a, b) | Expr::Par(a, b) => {
            walk_expr(ctx, a, visit);
            walk_expr(ctx, b, visit);
        }
        Expr::Adjoint(e) | Expr::Controlled(e) | Expr::Ascribe(e, _) => walk_expr(ctx, e, visit),
        // Type-level args are `NatExpr`s, not expressions — only the callee is walkable.
        Expr::TypeApp { callee, .. } => walk_expr(ctx, callee, visit),
        Expr::GateApp { gate, qubits } => {
            walk_expr(ctx, gate, visit);
            walk_expr(ctx, qubits, visit);
        }
        Expr::RunBlock(stmts) => walk_stmts(ctx, stmts, visit),
        Expr::Bind { rhs, body, .. } => {
            walk_expr(ctx, rhs, visit);
            walk_expr(ctx, body, visit);
        }
        Expr::Return(e) => walk_expr(ctx, e, visit),
        Expr::Borrow { body, .. } => {
            ctx.with_borrow(|nested| walk_stmts(nested, body, visit));
        }
        Expr::Int(_) | Expr::Float(_) | Expr::Bool(_) | Expr::Unit | Expr::Var(_) => {}
    }
}

pub fn walk_fn_bodies(
    ctx: &LintContext<'_>,
    visit: &mut dyn FnMut(&LintContext<'_>, &Sp<frontend::ast::Expr>),
) {
    for decl in &ctx.typed.decls {
        if let Decl::Fn { body, .. } = &decl.0 {
            walk_expr(ctx, body, visit);
        }
    }
}
