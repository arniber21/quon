// Formatter coverage for the QEC-era AST additions (ADR-0014):
// fn/type-alias type params, `Expr::TypeApp`, and `Type::QecBlock`.

use frontend::ast::{Decl, Expr};
use quonfmt::format_str;

fn roundtrip(src: &str) -> String {
    let f1 = format_str(src).expect("parse");
    let f2 = format_str(&f1).expect("re-parse");
    assert_eq!(f1, f2, "idempotency failed:\nf1:\n{f1}\nf2:\n{f2}");
    f1
}

#[test]
fn fn_type_params_print_kinded() {
    let out =
        roundtrip("fn rounds<F: CodeFamily, d: Nat>(b: QecBlock<F, d>): Q<QecBlock<F, d>> = b\n");
    assert!(
        out.contains("fn rounds<F: CodeFamily, d: Nat>("),
        "expected kinded type params:\n{out}"
    );
}

#[test]
fn qec_block_type_prints() {
    let out = roundtrip("type Encoded<F: CodeFamily, d: Nat> = QecBlock<F, d>\n");
    assert!(
        out.contains("type Encoded<F: CodeFamily, d: Nat> = QecBlock<F, d>"),
        "expected QecBlock alias:\n{out}"
    );
}

#[test]
fn bare_nat_type_param_stays_bare() {
    // `n` is Nat-only sugar (kind `None`); the formatter must not expand it to `n: Nat`.
    let out = roundtrip("type Oracle<n> = QReg<n>\n");
    assert!(
        out.contains("type Oracle<n> ="),
        "expected bare param:\n{out}"
    );
    assert!(!out.contains("n: Nat"), "must not expand Nat sugar:\n{out}");
}

#[test]
fn type_app_prints_postfix_and_reparses() {
    let out = roundtrip("fn f(): Int = repetition_code<3>()\n");
    assert!(
        out.contains("repetition_code<3>()"),
        "expected postfix type app:\n{out}"
    );
    let decls = frontend::parse_program(&out).unwrap();
    let Decl::Fn { body, .. } = &decls[0].0 else {
        panic!()
    };
    assert!(
        matches!(&body.0, Expr::App(f, x)
            if matches!(f.0, Expr::TypeApp { .. }) && matches!(x.0, Expr::Unit)),
        "expected App(TypeApp, Unit), got {:?}",
        body.0
    );
}

#[test]
fn type_app_multi_args_with_family() {
    // A CodeFamily arg at a use site is a bare name, parsed as a Nat var (`Surface`).
    let out = roundtrip("fn f(x: Int): Int = encode<Surface, 3>(x)\n");
    assert!(
        out.contains("encode<Surface, 3>(x)"),
        "expected multi-arg type app:\n{out}"
    );
}
