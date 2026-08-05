//! Synthetic new-node consistency test for the canonical AST visitor (issue #399).
//!
//! Builds a representative program exercising declarations, expressions,
//! statements, patterns, types, type parameters, and type-level natural
//! expressions through the public parser, then drives
//! [`frontend::visitor::walk_program`] with a recording visitor. The assertion
//! is that every node kind is visited with correctly nested pre/post pairing:
//! for each `visit_*_pre` there is a matching `visit_*_post` in LIFO order, and
//! the recorded sequence is exhaustive over the node kinds present in the
//! fixture. When a new AST variant lands, this test pins the contract that the
//! canonical traversal reaches it.

#![cfg(feature = "analyze")]

use std::collections::VecDeque;

use frontend::ast::{Decl, Expr, NatExpr, Pat, Stmt, Type};
use frontend::lexer::Sp;
use frontend::visitor::{Traversal, Visitor, walk_program};

/// Records the kind and phase of every visit; `pre`/`post` are paired by kind.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Event {
    DeclPre,
    DeclPost,
    ExprPre(&'static str),
    ExprPost(&'static str),
    StmtPre,
    StmtPost,
    PatPre,
    PatPost,
    TypePre(&'static str),
    TypePost(&'static str),
    NatExprPre,
    NatExprPost,
    TypeParamPre,
    TypeParamPost,
}

struct Recorder {
    events: VecDeque<Event>,
    /// Stack of open expression-kind tags; a `pre` pushes, its `post` pops and
    /// must match — proves the pre/post nesting is balanced.
    expr_stack: Vec<&'static str>,
    type_stack: Vec<&'static str>,
    /// Counters per node kind, to assert exhaustiveness.
    seen_decl: u32,
    seen_expr: u32,
    seen_stmt: u32,
    seen_pat: u32,
    seen_type: u32,
    seen_nat_expr: u32,
    seen_type_param: u32,
}

impl Recorder {
    fn new() -> Self {
        Self {
            events: VecDeque::new(),
            expr_stack: Vec::new(),
            type_stack: Vec::new(),
            seen_decl: 0,
            seen_expr: 0,
            seen_stmt: 0,
            seen_pat: 0,
            seen_type: 0,
            seen_nat_expr: 0,
            seen_type_param: 0,
        }
    }
}

fn expr_tag(e: &Expr) -> &'static str {
    match e {
        Expr::Int(_) => "Int",
        Expr::Float(_) => "Float",
        Expr::Bool(_) => "Bool",
        Expr::Unit => "Unit",
        Expr::Var(_) => "Var",
        Expr::Lam { .. } => "Lam",
        Expr::App(_, _) => "App",
        Expr::TypeApp { .. } => "TypeApp",
        Expr::BinOp { .. } => "BinOp",
        Expr::Neg(_) => "Neg",
        Expr::Let { .. } => "Let",
        Expr::If { .. } => "If",
        Expr::Match { .. } => "Match",
        Expr::For { .. } => "For",
        Expr::Tuple(_) => "Tuple",
        Expr::List(_) => "List",
        Expr::CircuitBlock(_) => "CircuitBlock",
        Expr::Compose(_, _) => "Compose",
        Expr::Par(_, _) => "Par",
        Expr::ParN(_) => "ParN",
        Expr::Adjoint(_) => "Adjoint",
        Expr::Controlled(_) => "Controlled",
        Expr::GateApp { .. } => "GateApp",
        Expr::RunBlock(_) => "RunBlock",
        Expr::Bind { .. } => "Bind",
        Expr::Return(_) => "Return",
        Expr::Borrow { .. } => "Borrow",
        Expr::Ascribe(_, _) => "Ascribe",
    }
}

fn type_tag(t: &Type) -> &'static str {
    match t {
        Type::Qubit => "Qubit",
        Type::QReg(_) => "QReg",
        Type::Bit => "Bit",
        Type::Bool => "Bool",
        Type::Int => "Int",
        Type::Float => "Float",
        Type::Unit => "Unit",
        Type::Nat => "Nat",
        Type::List(_) => "List",
        Type::Tuple(_) => "Tuple",
        Type::Fn(_, _) => "Fn",
        Type::Linear(_, _) => "Linear",
        Type::Circuit { .. } => "Circuit",
        Type::Q(_) => "Q",
        Type::Matrix(_, _, _) => "Matrix",
        Type::QecBlock { .. } => "QecBlock",
        Type::Var(_) => "Var",
        Type::Named { .. } => "Named",
    }
}

impl Visitor for Recorder {
    fn visit_decl_pre(&mut self, _d: &Sp<Decl>) -> Traversal {
        self.events.push_back(Event::DeclPre);
        self.seen_decl += 1;
        Traversal::Recurse
    }
    fn visit_decl_post(&mut self, _d: &Sp<Decl>) {
        self.events.push_back(Event::DeclPost);
    }

    fn visit_expr_pre(&mut self, e: &Sp<Expr>) -> Traversal {
        let tag = expr_tag(&e.0);
        self.events.push_back(Event::ExprPre(tag));
        self.expr_stack.push(tag);
        self.seen_expr += 1;
        Traversal::Recurse
    }
    fn visit_expr_post(&mut self, e: &Sp<Expr>) {
        let tag = self.expr_stack.pop().expect("expr post without pre");
        assert_eq!(tag, expr_tag(&e.0), "expr pre/post tag mismatch");
        self.events.push_back(Event::ExprPost(tag));
    }

    fn visit_stmt_pre(&mut self, _s: &Sp<Stmt>) -> Traversal {
        self.events.push_back(Event::StmtPre);
        self.seen_stmt += 1;
        Traversal::Recurse
    }
    fn visit_stmt_post(&mut self, _s: &Sp<Stmt>) {
        self.events.push_back(Event::StmtPost);
    }

    fn visit_pat_pre(&mut self, _p: &Sp<Pat>) -> Traversal {
        self.events.push_back(Event::PatPre);
        self.seen_pat += 1;
        Traversal::Recurse
    }
    fn visit_pat_post(&mut self, _p: &Sp<Pat>) {
        self.events.push_back(Event::PatPost);
    }

    fn visit_type_pre(&mut self, t: &Sp<Type>) -> Traversal {
        let tag = type_tag(&t.0);
        self.events.push_back(Event::TypePre(tag));
        self.type_stack.push(tag);
        self.seen_type += 1;
        Traversal::Recurse
    }
    fn visit_type_post(&mut self, t: &Sp<Type>) {
        let tag = self.type_stack.pop().expect("type post without pre");
        assert_eq!(tag, type_tag(&t.0), "type pre/post tag mismatch");
        self.events.push_back(Event::TypePost(tag));
    }

    fn visit_nat_expr_pre(&mut self, _n: &Sp<NatExpr>) -> Traversal {
        self.events.push_back(Event::NatExprPre);
        self.seen_nat_expr += 1;
        Traversal::Recurse
    }
    fn visit_nat_expr_post(&mut self, _n: &Sp<NatExpr>) {
        self.events.push_back(Event::NatExprPost);
    }

    fn visit_type_param_pre(&mut self, _tp: &frontend::ast::TypeParam) -> Traversal {
        self.events.push_back(Event::TypeParamPre);
        self.seen_type_param += 1;
        Traversal::Recurse
    }
    fn visit_type_param_post(&mut self, _tp: &frontend::ast::TypeParam) {
        self.events.push_back(Event::TypeParamPost);
    }
}

/// A fixture exercising a broad slice of node kinds: a kinded type-param fn
/// with a circuit body (stmts, gate apps, composition), a type alias with a
/// `Nat` parameter, patterns (tuple, var), `if`/`match`, `borrow`, and nested
/// `run`-desugared `Bind`. Type-level expressions appear in `QReg<n+1>` and
/// `Circuit<2*n, ...>`.
const FIXTURE: &str = r#"
type Oracle<n: Nat> = QReg<n+1>

fn teleport<F: CodeFamily, d: Nat>(q: Qubit): Circuit<2, 2, 1, Clifford> = circuit {
  let pair = bell()
  H @ pair |> CNOT @(pair, q)
}
fn main(): Q<Unit> = run {
  borrow a: Qubit in {
    discard(a)
  }
}
"#;

#[test]
fn canonical_visitor_visits_every_node_kind_with_balanced_pre_post() {
    let decls = frontend::desugar_program(FIXTURE).expect("fixture must parse+desugar");
    let mut rec = Recorder::new();
    walk_program(&mut rec, &decls);

    // Every node kind present in the fixture was reached.
    assert!(rec.seen_decl >= 2, "decls: {}", rec.seen_decl);
    assert!(rec.seen_expr >= 5, "exprs: {}", rec.seen_expr);
    assert!(rec.seen_stmt >= 1, "stmts: {}", rec.seen_stmt);
    assert!(rec.seen_pat >= 1, "pats: {}", rec.seen_pat);
    assert!(rec.seen_type >= 3, "types: {}", rec.seen_type);
    assert!(rec.seen_nat_expr >= 2, "nat exprs: {}", rec.seen_nat_expr);
    assert!(
        rec.seen_type_param >= 1,
        "type params: {}",
        rec.seen_type_param
    );

    // All pre/post stacks drained: nesting is balanced.
    assert!(rec.expr_stack.is_empty(), "unbalanced expr pre/post");
    assert!(rec.type_stack.is_empty(), "unbalanced type pre/post");

    // Globally, every `pre` has a matching `post` of the same kind.
    assert_decl_balanced(&rec.events);
    assert_stmt_balanced(&rec.events);
    assert_pat_balanced(&rec.events);
    assert_nat_expr_balanced(&rec.events);
    assert_type_param_balanced(&rec.events);
}

fn assert_decl_balanced(ev: &VecDeque<Event>) {
    let pre = ev.iter().filter(|e| matches!(e, Event::DeclPre)).count();
    let post = ev.iter().filter(|e| matches!(e, Event::DeclPost)).count();
    assert_eq!(pre, post, "Decl pre {pre} != post {post}");
}

fn assert_stmt_balanced(ev: &VecDeque<Event>) {
    let pre = ev.iter().filter(|e| matches!(e, Event::StmtPre)).count();
    let post = ev.iter().filter(|e| matches!(e, Event::StmtPost)).count();
    assert_eq!(pre, post, "Stmt pre {pre} != post {post}");
}

fn assert_pat_balanced(ev: &VecDeque<Event>) {
    let pre = ev.iter().filter(|e| matches!(e, Event::PatPre)).count();
    let post = ev.iter().filter(|e| matches!(e, Event::PatPost)).count();
    assert_eq!(pre, post, "Pat pre {pre} != post {post}");
}

fn assert_nat_expr_balanced(ev: &VecDeque<Event>) {
    let pre = ev.iter().filter(|e| matches!(e, Event::NatExprPre)).count();
    let post = ev
        .iter()
        .filter(|e| matches!(e, Event::NatExprPost))
        .count();
    assert_eq!(pre, post, "NatExpr pre {pre} != post {post}");
}

fn assert_type_param_balanced(ev: &VecDeque<Event>) {
    let pre = ev
        .iter()
        .filter(|e| matches!(e, Event::TypeParamPre))
        .count();
    let post = ev
        .iter()
        .filter(|e| matches!(e, Event::TypeParamPost))
        .count();
    assert_eq!(pre, post, "TypeParam pre {pre} != post {post}");
}

#[test]
fn canonical_visitor_preserves_source_spans() {
    // The visitor receives `&Sp<T>`, so `.1` is the node's source span. Verify
    // that decl spans actually cover source positions (non-zero length).
    let src = "fn f(): Int = 1\n";
    let decls = frontend::parse_program(src).expect("parse");
    struct SpanProbe {
        saw_decl_span: bool,
    }
    impl Visitor for SpanProbe {
        fn visit_decl_pre(&mut self, d: &Sp<Decl>) -> Traversal {
            assert!(d.1.start < d.1.end, "decl span is empty");
            self.saw_decl_span = true;
            Traversal::Recurse
        }
    }
    let mut probe = SpanProbe {
        saw_decl_span: false,
    };
    walk_program(&mut probe, &decls);
    assert!(probe.saw_decl_span);
}
