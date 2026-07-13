mod support;

use support::fixture::definition_at_marker;

#[test]
fn local_use_goes_to_let_binding() {
    let src = "fn f(): Int = let x = 1 in /*cursor*/x\n";
    let range = definition_at_marker(src);
    assert!(range.is_some(), "expected definition for local x");
}

#[test]
fn builtin_has_no_definition() {
    let src = "fn f(): Int = /*cursor*/range(3)\n";
    assert!(definition_at_marker(src).is_none());
}

#[test]
fn call_site_goes_to_fn_name() {
    let src = "fn helper(): Int = 1\nfn f(): Int = /*cursor*/helper()\n";
    let range = definition_at_marker(src).expect("definition for helper call");
    // `helper` name starts at line 0, character 3.
    assert_eq!(range.start.line, 0);
    assert_eq!(range.start.character, 3);
    assert_eq!(range.end.character, 9);
}

#[test]
fn param_use_goes_to_param() {
    let src = "fn f(x: Int): Int = /*cursor*/x\n";
    let range = definition_at_marker(src).expect("definition for param x");
    assert_eq!(range.start.line, 0);
    assert_eq!(range.start.character, 5);
}

#[test]
fn type_alias_use_goes_to_alias_name() {
    let src = "type MyInt = Int\nfn f(): /*cursor*/MyInt = 1\n";
    let range = definition_at_marker(src).expect("definition for MyInt");
    assert_eq!(range.start.line, 0);
    assert_eq!(range.start.character, 5);
    assert_eq!(range.end.character, 10);
}

#[test]
fn circuit_local_use_goes_to_let() {
    let src = r#"
fn f(): Circuit<1, 1, 1, Clifford> = circuit {
  let q = 0
  H @ /*cursor*/q
}
"#;
    let range = definition_at_marker(src).expect("definition for circuit local q");
    assert_eq!(range.start.line, 2);
}
