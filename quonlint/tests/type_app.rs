// `Expr::TypeApp` traversal (ADR-0014): linting a program whose fn body contains a
// type application must walk through it without derailing rules elsewhere.

mod support;

use support::{has_rule, lint_snippet};

#[test]
fn type_app_in_fn_body_does_not_break_rules() {
    let src = r#"fn mem(): Q<QecBlock<Repetition, 3>> = repetition_code<3>()
fn f(): Circuit<2, 2, 1, Clifford> = circuit { SWAP @(0, 1) }"#;
    assert!(
        frontend::check_program(src).is_ok(),
        "snippet must typecheck: {:?}",
        frontend::check_program(src).err()
    );
    let diags = lint_snippet(src);
    assert!(has_rule(&diags, "gates/swap-in-source"));
}
