use frontend::ast::{Decl, Expr};
use quonfmt::format_str;

#[test]
fn tuple_arg_uses_call_form_not_juxtaposition() {
    let src = "fn f(): Int = o((h, false))\n";
    let f1 = format_str(src).unwrap();
    let f2 = format_str(&f1).unwrap();
    assert_eq!(f1, f2);
    let decls = frontend::parse_program(&f1).unwrap();
    let Decl::Fn { body, .. } = &decls[0].0 else {
        panic!()
    };
    assert!(
        matches!(&body.0, Expr::App(f, x) if matches!(f.0, Expr::Var(_)) && matches!(x.0, Expr::Tuple(_))),
        "expected App(Var, Tuple), got {:?}",
        body.0
    );
}

#[test]
fn list_arg_uses_call_form_not_juxtaposition() {
    // `f [0]` would re-parse as index sugar; must print `f([0])`.
    let decls = frontend::parse_program("fn f(): Int = g([0])\n").unwrap();
    let f1 = quonfmt::format_decls(&decls);
    let f2 = format_str(&f1).unwrap();
    assert_eq!(f1, f2, "f1:\n{f1}\nf2:\n{f2}");
    assert!(f1.contains("g([0])"), "expected call form:\n{f1}");
}

#[test]
fn return_match_in_circuit_keeps_parens() {
    let src = "fn f(): Int = circuit {\n        return match 1 {\n_ => 0\n}\n}\n";
    let f1 = format_str(src).unwrap();
    let f2 = format_str(&f1).unwrap();
    assert_eq!(f1, f2, "f1:\n{f1}\nf2:\n{f2}");
    assert!(
        f1.contains("(match "),
        "expected parenthesized match stmt:\n{f1}"
    );
}
