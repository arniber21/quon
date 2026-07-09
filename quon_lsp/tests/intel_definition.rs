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
