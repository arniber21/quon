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

// The structural recursion (which children each node has, and in what order) is
// owned by `frontend::visitor`'s canonical `walk_*` drivers (issue #399). The
// lint-specific concern threaded on top is circuit/borrow nesting: the same
// rule callback must observe `ctx.in_circuit()` / `ctx.borrow_depth()` that
// reflect the enclosing `circuit` / `borrow` block. `LintWalker` implements
// `frontend::visitor::Visitor`, pushing/popping that nesting in the
// pre/post-expr hooks, so the canonical driver descends while the callback sees
// the correct `LintContext` at every node — preserving the exact pre-order
// callback sequence the rules relied on, including the historical quirk that a
// `Bind`/`Let` statement's right-hand side is visited but not descended into.
pub fn walk_stmts(
    ctx: &LintContext<'_>,
    stmts: &[Sp<frontend::ast::Stmt>],
    visit: &mut dyn FnMut(&LintContext<'_>, &Sp<frontend::ast::Expr>),
) {
    let mut walker = LintWalker::new(ctx, visit);
    for stmt in stmts {
        frontend::visitor::walk_stmt(&mut walker, stmt);
    }
}

pub fn walk_expr(
    ctx: &LintContext<'_>,
    expr: &Sp<frontend::ast::Expr>,
    visit: &mut dyn FnMut(&LintContext<'_>, &Sp<frontend::ast::Expr>),
) {
    let mut walker = LintWalker::new(ctx, visit);
    frontend::visitor::walk_expr(&mut walker, expr);
}

pub fn walk_fn_bodies(
    ctx: &LintContext<'_>,
    visit: &mut dyn FnMut(&LintContext<'_>, &Sp<frontend::ast::Expr>),
) {
    let mut walker = LintWalker::new(ctx, visit);
    for decl in &ctx.typed.decls {
        if let Decl::Fn { body, .. } = &decl.0 {
            frontend::visitor::walk_expr(&mut walker, body);
        }
    }
}

/// Adapter that drives the canonical AST traversal while threading lint
/// circuit/borrow nesting into the rule callback.
///
/// - `visit_expr_pre`: invokes the rule callback with the *outer* nesting
///   state, then pushes circuit/borrow state so descendants see the nested
///   context. `visit_expr_post` pops it.
/// - `visit_stmt_pre`: for `Bind`/`Let`, invokes the callback on the
///   right-hand side and returns [`Traversal::Skip`] (the rhs is observed but
///   not descended into — the historical lint semantics); for `Expr`,
///   recurses normally.
struct LintWalker<'a, 'v> {
    base: &'a LintContext<'a>,
    /// Circuit-block nesting as a stack so push/pop restores the outer value
    /// (a `circuit` inside a `circuit` returns to `true`, not `false`).
    in_circuit_stack: Vec<bool>,
    borrow_depth: u32,
    visit: &'v mut (dyn FnMut(&LintContext<'_>, &Sp<frontend::ast::Expr>) + 'a),
}

impl<'a, 'v> LintWalker<'a, 'v> {
    fn new(
        base: &'a LintContext<'a>,
        visit: &'v mut (dyn FnMut(&LintContext<'_>, &Sp<frontend::ast::Expr>) + 'a),
    ) -> Self {
        Self {
            base,
            in_circuit_stack: vec![base.in_circuit()],
            borrow_depth: base.borrow_depth(),
            visit,
        }
    }

    fn current_in_circuit(&self) -> bool {
        self.in_circuit_stack.last().copied().unwrap_or(false)
    }

    /// A child `LintContext` reflecting the current nesting, handed to the
    /// rule callback. `child` is private to this module; `LintWalker` lives in
    /// the same module so it can reach it.
    fn callback(&mut self, expr: &Sp<frontend::ast::Expr>) {
        let ctx = self.base.child(self.current_in_circuit(), self.borrow_depth);
        (self.visit)(&ctx, expr);
    }
}

impl<'a, 'v> frontend::visitor::Visitor for LintWalker<'a, 'v> {
    fn visit_expr_pre(&mut self, expr: &Sp<frontend::ast::Expr>) -> frontend::visitor::Traversal {
        use frontend::ast::Expr;
        // Pre-order: the callback sees the node with the *outer* nesting state.
        self.callback(expr);
        match &expr.0 {
            Expr::CircuitBlock(_) => self.in_circuit_stack.push(true),
            Expr::Borrow { .. } => self.borrow_depth += 1,
            // `RunBlock` does NOT enter a circuit context (matches prior walker).
            _ => {}
        }
        frontend::visitor::Traversal::Recurse
    }

    fn visit_expr_post(&mut self, expr: &Sp<frontend::ast::Expr>) {
        use frontend::ast::Expr;
        match &expr.0 {
            Expr::CircuitBlock(_) => {
                self.in_circuit_stack.pop();
            }
            Expr::Borrow { .. } => self.borrow_depth -= 1,
            _ => {}
        }
    }

    fn visit_stmt_pre(&mut self, stmt: &Sp<frontend::ast::Stmt>) -> frontend::visitor::Traversal {
        use frontend::ast::Stmt;
        match &stmt.0 {
            // `Bind`/`Let`: observe the rhs, do not descend (preserves prior
            // walker semantics where only `Stmt::Expr` is recursed into).
            Stmt::Bind { rhs, .. } | Stmt::Let { rhs, .. } => {
                self.callback(rhs);
                frontend::visitor::Traversal::Skip
            }
            Stmt::Expr(_) => frontend::visitor::Traversal::Recurse,
        }
    }
}
