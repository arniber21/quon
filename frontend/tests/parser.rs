// Parser unit tests — issue #7 acceptance criteria: operator precedence/associativity,
// RunBlock (not Bind) nodes, par tensor syntax, desugaring, and error spans.

mod support;

use frontend::ast::{Decl, Expr, Stmt, Type};
use frontend::lexer::lex;
use frontend::parser::parse;
use frontend::pretty::pretty;
use support::parse_stripped;

/// Parse `expr_src` as a function body and return the (span-stripped) body expression.
fn body(expr_src: &str) -> Expr {
    let src = format!("fn f(): Int = {expr_src}");
    let decls = parse_stripped(&src);
    match decls.into_iter().next().map(|(d, _)| d) {
        Some(Decl::Fn { body, .. }) => body.0,
        other => panic!("expected fn decl, got {other:?}"),
    }
}

/// Parse `expr_src` as a function body and pretty-print it. Because the printer fully
/// parenthesizes operator forms, the resulting string reveals precedence + associativity.
fn shape(expr_src: &str) -> String {
    let src = format!("fn f(): Int = {expr_src}");
    let decls = parse_stripped(&src);
    let printed = pretty(&decls);
    printed
        .strip_prefix("fn f(): Int = ")
        .unwrap_or(&printed)
        .replace('\n', " ")
        .trim()
        .to_string()
}

/// Parse `type_src` as a type alias target and pretty-print it. This exposes type-level
/// Nat-expression precedence because the printer parenthesizes every Nat operator.
fn type_shape(type_src: &str) -> String {
    let src = format!("type T = {type_src}");
    let decls = parse_stripped(&src);
    let printed = pretty(&decls);
    printed
        .strip_prefix("type T = ")
        .unwrap_or(&printed)
        .replace('\n', " ")
        .trim()
        .to_string()
}

#[test]
fn pipe_is_left_associative() {
    assert_eq!(shape("a |> b |> c"), "(a |> b) |> c");
}

#[test]
fn pipe_binds_looser_than_at() {
    // `|>` looser than `@`: each gate application groups before composition.
    assert_eq!(shape("H @ 0 |> X @ 1"), "(H @ 0) |> (X @ 1)");
}

#[test]
fn pipe_bridges_newlines() {
    assert_eq!(shape("a\n|>\nb"), "a |> b");
}

#[test]
fn at_is_left_associative() {
    assert_eq!(shape("X @ 0 @ q"), "(X @ 0) @ q");
}

#[test]
fn at_does_not_cross_newline() {
    let tokens = lex("fn f(): Int = circuit { H\n@ 0 }").expect("lexes fine");
    assert!(
        parse(&tokens).is_err(),
        "gate application must stay on one logical line"
    );
}

#[test]
fn juxtaposition_does_not_cross_newline() {
    let b = body("circuit { f\nx }");
    let stmts = match b {
        Expr::CircuitBlock(stmts) => stmts,
        other => panic!("expected CircuitBlock, got {other:?}"),
    };
    assert_eq!(stmts.len(), 2);
    assert!(matches!(&stmts[0].0, Stmt::Expr((Expr::Var(name), _)) if name == "f"));
    assert!(matches!(&stmts[1].0, Stmt::Expr((Expr::Var(name), _)) if name == "x"));
}

#[test]
fn multiplicative_binds_tighter_than_additive() {
    assert_eq!(shape("a + b * c"), "a + (b * c)");
    assert_eq!(shape("a * b + c"), "(a * b) + c");
}

#[test]
fn additive_is_left_associative() {
    assert_eq!(shape("a - b - c"), "(a - b) - c");
}

#[test]
fn unary_minus_binds_tighter_than_multiply() {
    assert_eq!(shape("-a * b"), "(- a) * b");
}

#[test]
fn exponent_is_right_associative() {
    assert_eq!(shape("a ^ b ^ c"), "a ^ (b ^ c)");
}

#[test]
fn nat_exponent_is_right_associative() {
    assert_eq!(
        type_shape("Circuit<1, 1, n ^ m ^ p, Clifford>"),
        "Circuit<1, 1, (n) ^ ((m) ^ (p)), Clifford>"
    );
}

#[test]
fn multi_arg_call_curries() {
    // f(a, b) desugars to App(App(f, a), b).
    assert_eq!(shape("f(a, b)"), "(f(a))(b)");
}

#[test]
fn empty_call_applies_unit() {
    assert_eq!(shape("f()"), "f(())");
}

#[test]
fn backtick_infix_desugars_to_application() {
    // a `g` b  ==  g(a, b)  ==  App(App(g, a), b)
    assert_eq!(shape("a `g` b"), shape("g(a, b)"));
}

#[test]
fn index_desugars_to_index_call() {
    assert_eq!(shape("q[i]"), "(index(q))(i)");
}

#[test]
fn method_desugars_to_ufcs() {
    assert_eq!(shape("xs.take(p)"), "(take(xs))(p)");
}

#[test]
fn run_block_produces_runblock_with_bind_not_desugared() {
    let b = body("run { x <- e\nreturn x }");
    let stmts = match b {
        Expr::RunBlock(stmts) => stmts,
        other => panic!("expected RunBlock, got {other:?}"),
    };
    // First statement must be a *statement* Bind, not a desugared Expr::Bind.
    assert!(
        matches!(stmts[0].0, Stmt::Bind { .. }),
        "expected Stmt::Bind, got {:?}",
        stmts[0].0
    );
    assert!(matches!(stmts[1].0, Stmt::Expr(_)));
}

#[test]
fn par_tensor_syntax() {
    let b = body("par { H @ 0 } * 4");
    assert!(matches!(b, Expr::Par(_, _)), "expected Par, got {b:?}");
    assert_eq!(shape("par { H @ 0 } * 4"), "par { H @ 0 } * 4");
}

#[test]
fn par_count_absorbs_multiplicative_chain() {
    // `par { c } * 4 * 2` binds the whole `* …` chain into the count: `par { c } * (4 * 2)`,
    // not `(par { c } * 4) * 2`.
    let b = body("par { x } * 4 * 2");
    match b {
        Expr::Par(_, count) => assert!(
            matches!(
                count.0,
                Expr::BinOp {
                    op: frontend::ast::BinOp::Mul,
                    ..
                }
            ),
            "expected count to be a multiplication, got {:?}",
            count.0
        ),
        other => panic!("expected Par, got {other:?}"),
    }
    assert_eq!(shape("par { x } * 4 * 2"), "par { x } * (4 * 2)");
}

#[test]
fn bare_type_name_is_var_parameterized_is_named() {
    // Bare non-builtin name -> Type::Var; `Name<args>` -> Type::Named.
    let decls = parse_stripped("fn f(x: Bell): Oracle<n> = ()");
    let (params, ret) = match &decls[0].0 {
        Decl::Fn { params, ret, .. } => (params, ret),
        other => panic!("expected fn, got {other:?}"),
    };
    assert!(
        matches!(&params[0].1 .0, Type::Var(n) if n == "Bell"),
        "bare Bell should be Type::Var, got {:?}",
        params[0].1.0
    );
    assert!(
        matches!(&ret.0, Type::Named { name, .. } if name == "Oracle"),
        "Oracle<n> should be Type::Named, got {:?}",
        ret.0
    );
}

#[test]
fn tuple_and_unit() {
    assert!(matches!(body("()"), Expr::Unit));
    assert!(matches!(body("(a, b)"), Expr::Tuple(ref v) if v.len() == 2));
    // A single parenthesized expression is grouping, not a 1-tuple.
    assert!(matches!(body("(a)"), Expr::Var(_)));
}

#[test]
fn parse_error_carries_span() {
    // `=` with no body, then a stray close paren — must be Err with a span, not a panic.
    let src = "fn f(): Int = )";
    let tokens = lex(src).expect("lexes fine");
    let errs = parse(&tokens).expect_err("expected a parse error");
    let close_paren = src.rfind(')').expect("test source contains a close paren");
    assert!(
        errs.iter()
            .any(|(_, span)| span.start == close_paren && span.end == close_paren + 1),
        "expected an error at byte {close_paren}, got {errs:?}"
    );
}
