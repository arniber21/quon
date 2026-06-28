// Desugaring tests — issue #8 acceptance criteria:
//   1. `run { x <- e1; e2; return v }` desugars to nested Bind/Return nodes.
//   2. `let (a, b) = q` inside `run { }` passes through as a Let, not a Bind.
//   3. Spans on desugared nodes point at the original source location.
//   4. Each statement form (bind, bare expr, let, return) desugars correctly.
// Plus the error path: a malformed `run` block reports diagnostics instead of
// panicking (the bidirectional verifier in #9 relies on never being handed a
// panic from an earlier stage).

mod support;

use frontend::ast::{Decl, Expr, Pat};
use frontend::desugar::desugar_decls;
use support::strip_decls;

/// Parse `body_src` as the body of `fn f`, desugar it, and return the
/// span-stripped body expression for structural assertions.
fn desugar_body(body_src: &str) -> Expr {
    let src = format!("fn f(): Int = {body_src}");
    let decls = frontend::parse_program(&src).expect("parse failed");
    let mut desugared = desugar_decls(decls).expect("desugar reported errors");
    strip_decls(&mut desugared);
    match desugared.into_iter().next().map(|(d, _)| d) {
        Some(Decl::Fn { body, .. }) => body.0,
        other => panic!("expected fn decl, got {other:?}"),
    }
}

fn var(name: &str) -> Expr {
    Expr::Var(name.to_string())
}

#[test]
fn run_block_desugars_to_nested_binds() {
    // run { x <- e1; e2; return v }
    //   => Bind(e1, "x", Bind(e2, "_", Return(v)))
    let body = desugar_body("run {\n  x <- e1\n  e2\n  return v\n}");

    let Expr::Bind { rhs, param, body } = body else {
        panic!("expected outer Bind, got {body:?}");
    };
    assert_eq!(param, "x");
    assert_eq!(rhs.0, var("e1"));

    let Expr::Bind { rhs, param, body } = body.0 else {
        panic!("expected inner Bind, got {:?}", body.0);
    };
    assert_eq!(
        param, "_",
        "a bare expression statement discards its result"
    );
    assert_eq!(rhs.0, var("e2"));

    let Expr::Return(v) = body.0 else {
        panic!("expected Return, got {:?}", body.0);
    };
    assert_eq!(v.0, var("v"));
}

#[test]
fn let_binding_in_run_block_stays_a_let() {
    // acceptance #2: `let (a, b) = q` is a Let, not a Bind.
    let body = desugar_body("run {\n  let (a, b) = q\n  return a\n}");

    let Expr::Let { pat, rhs, body } = body else {
        panic!("expected Let, got {body:?}");
    };
    assert!(
        matches!(pat.0, Pat::Tuple(_)),
        "tuple let-pattern is preserved, got {:?}",
        pat.0
    );
    assert_eq!(rhs.0, var("q"));
    assert!(
        matches!(body.0, Expr::Return(_)),
        "let body continues into the rest of the block"
    );
}

#[test]
fn single_return_desugars_to_return() {
    // acceptance #4: the `return` statement form on its own.
    let body = desugar_body("run {\n  return v\n}");
    let Expr::Return(v) = body else {
        panic!("expected Return, got {body:?}");
    };
    assert_eq!(v.0, var("v"));
}

#[test]
fn nested_run_blocks_are_desugared() {
    // A `run` block in a bind right-hand side must itself be lowered.
    let body = desugar_body("run {\n  x <- run {\n    return inner\n  }\n  return x\n}");
    let Expr::Bind { rhs, .. } = body else {
        panic!("expected Bind, got {body:?}");
    };
    assert!(
        matches!(rhs.0, Expr::Return(_)),
        "inner run block lowered to Return, got {:?}",
        rhs.0
    );
}

#[test]
fn desugared_nodes_preserve_source_spans() {
    // acceptance #3: spans survive the rewrite and point at the original source.
    let src = "fn f(): Int = run {\n  x <- e1\n  return v\n}";
    let decls = frontend::parse_program(src).expect("parse failed");
    let desugared = desugar_decls(decls).expect("desugar reported errors");

    let Decl::Fn { body, .. } = &desugared[0].0 else {
        panic!("expected fn decl");
    };
    let Expr::Bind { rhs, body, .. } = &body.0 else {
        panic!("expected Bind");
    };

    let e1 = src.find("e1").unwrap();
    assert_eq!(
        (rhs.1.start, rhs.1.end),
        (e1, e1 + 2),
        "rhs keeps `e1` span"
    );

    let Expr::Return(v) = &body.0 else {
        panic!("expected Return");
    };
    let v_at = src.find("return v").unwrap() + "return ".len();
    assert_eq!(
        (v.1.start, v.1.end),
        (v_at, v_at + 1),
        "return value keeps `v` span"
    );
}

#[test]
fn run_block_ending_in_bind_is_a_diagnostic_not_a_panic() {
    // A trailing `<-` bind has no continuation: report it, don't panic.
    let src = "fn f(): Int = run {\n  x <- e\n}";
    let decls = frontend::parse_program(src).expect("parse failed");
    let errs = desugar_decls(decls).expect_err("trailing bind should be rejected");
    assert_eq!(errs.len(), 1);
    assert!(
        errs[0].message.contains("must end in an expression"),
        "got: {}",
        errs[0].message
    );
}

#[test]
fn tuple_bind_pattern_destructures_via_a_fresh_let() {
    // `(a, b) <- e; return a` ⟶ Bind(e, $t, let (a, b) = $t in Return(a)).
    // The Bind node holds one name, so a tuple pattern binds a fresh variable that is
    // immediately destructured with a `let` — the form `hello_bell`/`teleport` rely on.
    let body = desugar_body("run {\n  (a, b) <- e\n  return a\n}");

    let Expr::Bind { rhs, param, body } = body else {
        panic!("expected Bind, got {body:?}");
    };
    assert_eq!(rhs.0, var("e"));
    assert!(
        param.starts_with('$'),
        "tuple bind introduces a fresh, non-collidable name, got {param:?}"
    );

    let Expr::Let { pat, rhs, body } = body.0 else {
        panic!("expected destructuring Let, got {:?}", body.0);
    };
    assert!(
        matches!(pat.0, Pat::Tuple(_)),
        "the original tuple pattern is preserved in the let, got {:?}",
        pat.0
    );
    assert_eq!(
        rhs.0,
        var(&param),
        "the let destructures the bind's fresh var"
    );
    assert!(
        matches!(body.0, Expr::Return(_)),
        "continuation is preserved"
    );
}

#[test]
fn desugar_program_runs_the_pass_end_to_end() {
    // The pipeline seam (lib.rs) actually invokes the pass — guards against the
    // pass silently never running.
    let src = "fn f(): Int = run {\n  x <- e\n  return x\n}";
    let decls = frontend::desugar_program(src).expect("desugar_program failed");
    let Decl::Fn { body, .. } = &decls[0].0 else {
        panic!("expected fn decl");
    };
    assert!(
        matches!(body.0, Expr::Bind { .. }),
        "run block lowered to Bind, got {:?}",
        body.0
    );
}
