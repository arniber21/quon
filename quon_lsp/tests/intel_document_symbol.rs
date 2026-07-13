mod support;

use support::fixture::document_symbol_names;

#[test]
fn outline_includes_functions_and_types() {
    let src = r#"
type Oracle<n> = Circuit<n, n, 1, Clifford>

fn bell(): Circuit<2, 2, 1, Clifford> = circuit {
  H @ 0
  CNOT @ (0, 1)
}

fn id(x: Int): Int = x
"#;
    let names = document_symbol_names(src);
    assert!(names.contains(&"Oracle".into()), "missing type: {names:?}");
    assert!(names.contains(&"bell".into()), "missing fn: {names:?}");
    assert!(names.contains(&"id".into()), "missing fn: {names:?}");
    assert!(
        names.contains(&"n".into()) || names.contains(&"x".into()),
        "expected nested locals/params: {names:?}"
    );
}

#[test]
fn outline_nests_let_binding_under_fn() {
    let src = "fn f(): Int = let x = 1 in x\n";
    let names = document_symbol_names(src);
    assert_eq!(names[0], "f");
    assert!(names.contains(&"x".into()), "expected local x: {names:?}");
}
