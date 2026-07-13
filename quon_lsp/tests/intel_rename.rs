mod support;

use support::fixture::{prepare_rename_at_marker, rename_at_marker};
use tower_lsp::lsp_types::PrepareRenameResponse;

#[test]
fn prepare_rename_local_returns_range() {
    let src = "fn f(): Int = let x = 1 in /*cursor*/x\n";
    let resp = prepare_rename_at_marker(src)
        .expect("prepare ok")
        .expect("renameable");
    match resp {
        PrepareRenameResponse::RangeWithPlaceholder { placeholder, .. } => {
            assert_eq!(placeholder, "x");
        }
        other => panic!("expected placeholder response, got {other:?}"),
    }
}

#[test]
fn prepare_rename_refuses_builtin() {
    let src = "fn f(): Int = /*cursor*/range(3)\n";
    let err = prepare_rename_at_marker(src).expect_err("builtin not renameable");
    assert!(
        err.message.contains("built-in"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn rename_local_updates_decl_and_uses() {
    let src = "fn f(): Int = let x = 1 in /*cursor*/x + x\n";
    let edit = rename_at_marker(src, "y")
        .expect("rename ok")
        .expect("workspace edit");
    let changes = edit.changes.expect("changes");
    let edits = changes.values().next().expect("file edits");
    assert_eq!(edits.len(), 3, "decl + 2 uses, got {edits:?}");
    assert!(edits.iter().all(|e| e.new_text == "y"));
}

#[test]
fn rename_function_updates_def_and_calls() {
    let src = "fn helper(): Int = 1\nfn f(): Int = /*cursor*/helper() + helper()\n";
    let edit = rename_at_marker(src, "aid")
        .expect("rename ok")
        .expect("workspace edit");
    let edits = edit.changes.expect("changes").into_values().next().unwrap();
    assert_eq!(edits.len(), 3, "def + 2 calls, got {edits:?}");
}

#[test]
fn rename_param_and_type_alias() {
    let src = "type MyInt = Int\nfn f(/*cursor*/x: MyInt): MyInt = x\n";
    let edit = rename_at_marker(src, "n")
        .expect("rename param")
        .expect("edit");
    let edits = edit.changes.expect("changes").into_values().next().unwrap();
    assert_eq!(edits.len(), 2, "param decl + use, got {edits:?}");

    let src = "type MyInt = Int\nfn f(): /*cursor*/MyInt = 1\n";
    let edit = rename_at_marker(src, "Alias")
        .expect("rename alias")
        .expect("edit");
    let edits = edit.changes.expect("changes").into_values().next().unwrap();
    assert_eq!(edits.len(), 2, "alias def + use, got {edits:?}");
}

#[test]
fn rename_refuses_keyword() {
    let src = "fn f(): Int = let x = 1 in /*cursor*/x\n";
    let err = rename_at_marker(src, "let").expect_err("keyword refused");
    assert!(
        err.message.contains("keyword"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn rename_refuses_invalid_ident() {
    let src = "fn f(): Int = let x = 1 in /*cursor*/x\n";
    let err = rename_at_marker(src, "1bad").expect_err("invalid ident");
    assert!(
        err.message.contains("invalid identifier"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn rename_refuses_shadow_collision() {
    // Renaming outer `x` to `y` would make the use inside the inner let resolve to inner `y`.
    let src = "fn f(): Int = let x = 1 in let y = 2 in /*cursor*/x\n";
    let err = rename_at_marker(src, "y").expect_err("shadow refused");
    assert!(
        err.message.contains("shadow") || err.message.contains("collide"),
        "unexpected message: {}",
        err.message
    );
}

#[test]
fn rename_inner_to_outer_name_allowed() {
    // Inner shadows outer; renaming inner to the outer's name is safe.
    let src = "fn f(): Int = let x = 1 in let y = 2 in /*cursor*/y\n";
    let edit = rename_at_marker(src, "x")
        .expect("rename ok")
        .expect("edit");
    let edits = edit.changes.expect("changes").into_values().next().unwrap();
    assert_eq!(edits.len(), 2, "inner decl + use, got {edits:?}");
}

#[test]
fn rename_respects_shadowing_scope() {
    // Cursor on inner `x`; only inner decl + use should rename.
    let src = "fn f(): Int = let x = 1 in let x = 2 in /*cursor*/x\n";
    let edit = rename_at_marker(src, "z")
        .expect("rename ok")
        .expect("edit");
    let edits = edit.changes.expect("changes").into_values().next().unwrap();
    assert_eq!(edits.len(), 2, "inner only, got {edits:?}");
}
