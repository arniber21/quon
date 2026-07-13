mod support;

use support::fixture::references_at_marker;

#[test]
fn references_include_declaration_and_uses() {
    let src = "fn f(): Int = let x = 1 in /*cursor*/x + x\n";
    let locs = references_at_marker(src, true).expect("references");
    // declaration `x` + two uses
    assert_eq!(locs.len(), 3, "expected decl + 2 uses, got {locs:?}");
}

#[test]
fn references_exclude_declaration_when_requested() {
    let src = "fn f(): Int = let x = 1 in /*cursor*/x + x\n";
    let locs = references_at_marker(src, false).expect("references");
    assert_eq!(locs.len(), 2, "expected 2 uses only, got {locs:?}");
}

#[test]
fn references_for_function_call() {
    let src = "fn helper(): Int = 1\nfn f(): Int = /*cursor*/helper() + helper()\n";
    let locs = references_at_marker(src, true).expect("references");
    // fn name + two call sites
    assert_eq!(locs.len(), 3, "expected fn def + 2 calls, got {locs:?}");
}

#[test]
fn references_respect_shadowing() {
    // Inner `x` must not pick up the outer binding's use.
    let src = "fn f(): Int = let x = 1 in let x = 2 in /*cursor*/x\n";
    let locs = references_at_marker(src, true).expect("references");
    assert_eq!(
        locs.len(),
        2,
        "inner x should be decl + one use only, got {locs:?}"
    );
}

#[test]
fn builtin_has_no_references() {
    let src = "fn f(): Int = /*cursor*/range(3)\n";
    assert!(references_at_marker(src, true).is_none());
}
