#![allow(dead_code)]

#[path = "../../../frontend/tests/support/mod.rs"]
mod strip_support;

use strip_support::parse_stripped;

pub fn assert_ast_stable(before: &str, after: &str) {
    let a = parse_stripped(before);
    let b = parse_stripped(after);
    assert_eq!(a, b, "AST changed after format");
}

pub fn all_corpus() -> Vec<(&'static str, String)> {
    [
        ("decls.qn", include_str!("../corpus/input/decls.qn")),
        (
            "circuit_compose.qn",
            include_str!("../corpus/input/circuit_compose.qn"),
        ),
        ("run_binds.qn", include_str!("../corpus/input/run_binds.qn")),
        ("borrow.qn", include_str!("../corpus/input/borrow.qn")),
        ("types.qn", include_str!("../corpus/input/types.qn")),
        (
            "expr_precedence.qn",
            include_str!("../corpus/input/expr_precedence.qn"),
        ),
        (
            "par_repeat.qn",
            include_str!("../corpus/input/par_repeat.qn"),
        ),
        ("match_if.qn", include_str!("../corpus/input/match_if.qn")),
        ("lambdas.qn", include_str!("../corpus/input/lambdas.qn")),
        (
            "application.qn",
            include_str!("../corpus/input/application.qn"),
        ),
    ]
    .into_iter()
    .map(|(n, s)| (n, s.to_string()))
    .collect()
}
