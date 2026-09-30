mod support;

use std::fs;
use std::path::PathBuf;
use std::process;

use frontend::analyze;
use tower_lsp::lsp_types::{GotoDefinitionResponse, Position, TextEdit, Url};

use quon_lsp::intel::{
    definition_in_workspace, prepare_rename_in_workspace, references_in_workspace,
    rename_in_workspace,
};
use quon_lsp::workspace::WorkspaceIndex;
use support::fixture::{position_after_marker, src_without_marker};

fn url(dir: &str, name: &str) -> Url {
    Url::from_file_path(format!("/tmp/quon-lsp-workspace/{dir}/{name}.qn")).expect("file url")
}

fn index_of(files: &[(&Url, &str)]) -> WorkspaceIndex {
    let mut index = WorkspaceIndex::default();
    for (uri, src) in files {
        index.insert((*uri).clone(), analyze(src).intelligence);
    }
    index
}

#[test]
fn definition_jumps_to_sibling_file() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let def_src = "fn helper(): Int = 1\n";
    let use_marked = "fn f(): Int = /*cursor*/helper()\n";
    let use_src = src_without_marker(use_marked);
    let index = index_of(&[(&def_url, def_src), (&use_url, &use_src)]);
    let analysis = analyze(&use_src).intelligence;
    let response = definition_in_workspace(
        &analysis,
        &use_url,
        position_after_marker(use_marked),
        &index,
    )
    .expect("definition");
    let GotoDefinitionResponse::Scalar(loc) = response else {
        panic!("expected one definition, got {response:?}");
    };
    assert_eq!(loc.uri, def_url);
    assert_eq!(loc.range.start.line, 0);
    assert_eq!(loc.range.start.character, 3);
}

#[test]
fn references_include_sibling_use_and_definition() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let def_marked = "fn /*cursor*/helper(): Int = 1\n";
    let def_src = src_without_marker(def_marked);
    let use_src = "fn f(): Int = helper()\n";
    let index = index_of(&[(&def_url, &def_src), (&use_url, use_src)]);
    let analysis = analyze(&def_src).intelligence;
    let locs = references_in_workspace(
        &analysis,
        &def_url,
        position_after_marker(def_marked),
        true,
        &index,
    )
    .expect("references");
    assert!(locs.iter().any(|loc| loc.uri == def_url));
    assert!(locs.iter().any(|loc| loc.uri == use_url));
}

#[test]
fn rename_edits_sibling_files() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let def_src = "fn helper(): Int = 1\n";
    let use_marked = "fn f(): Int = /*cursor*/helper()\n";
    let use_src = src_without_marker(use_marked);
    let index = index_of(&[(&def_url, def_src), (&use_url, &use_src)]);
    let analysis = analyze(&use_src).intelligence;
    let edit = rename_in_workspace(
        &analysis,
        &use_url,
        position_after_marker(use_marked),
        "helper2",
        &index,
    )
    .expect("rename ok")
    .expect("workspace edit");
    let changes = edit.changes.expect("changes");
    assert_eq!(changes.len(), 2);
    assert!(
        changes[&def_url]
            .iter()
            .any(|edit| edit.new_text == "helper2")
    );
    assert!(
        changes[&use_url]
            .iter()
            .any(|edit| edit.new_text == "helper2")
    );
}

fn edited_text(src: &str, edits: &[TextEdit]) -> String {
    let mut ranges = Vec::new();
    for edit in edits {
        let start = offset_at(src, edit.range.start);
        let end = offset_at(src, edit.range.end);
        ranges.push((start, end, edit.new_text.as_str()));
    }
    ranges.sort_by_key(|range| std::cmp::Reverse(range.0));
    let mut out = src.to_string();
    for (start, end, text) in ranges {
        out.replace_range(start..end, text);
    }
    out
}

fn offset_at(src: &str, position: Position) -> usize {
    let mut offset = 0;
    for (index, line) in src.split_inclusive('\n').enumerate() {
        if index == position.line as usize {
            return offset + position.character as usize;
        }
        offset += line.len();
    }
    offset
}

#[test]
fn parse_error_sibling_is_not_rewritten() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let def_marked = "fn /*cursor*/helper(): Int = 1\n";
    let def_src = src_without_marker(def_marked);
    let broken = "fn f(helper: Int): Int = helper +\n";
    let index = index_of(&[(&def_url, &def_src), (&use_url, broken)]);
    let analysis = analyze(&def_src).intelligence;
    let edit = rename_in_workspace(
        &analysis,
        &def_url,
        position_after_marker(def_marked),
        "helper2",
        &index,
    )
    .expect("rename ok")
    .expect("workspace edit");
    let changes = edit.changes.expect("changes");
    assert!(
        !changes.contains_key(&use_url),
        "a file that does not parse must not be lex-scraped"
    );
    assert!(
        changes[&def_url]
            .iter()
            .any(|edit| edit.new_text == "helper2")
    );
}

#[test]
fn let_rhs_is_renamed_and_body_stays() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let def_marked = "fn /*cursor*/helper(): Int = 1\n";
    let def_src = src_without_marker(def_marked);
    let use_src = "fn f(): Int = let helper = helper() in helper\n";
    let index = index_of(&[(&def_url, &def_src), (&use_url, use_src)]);
    let analysis = analyze(&def_src).intelligence;
    let edit = rename_in_workspace(
        &analysis,
        &def_url,
        position_after_marker(def_marked),
        "helper2",
        &index,
    )
    .expect("rename ok")
    .expect("workspace edit");
    let changes = edit.changes.expect("changes");
    let rewritten = edited_text(use_src, &changes[&use_url]);
    assert_eq!(
        rewritten,
        "fn f(): Int = let helper = helper2() in helper\n"
    );
}

#[test]
fn params_shadow_types_but_bare_call_is_renamed() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let def_marked = "fn /*cursor*/n(): Int = 1\n";
    let def_src = src_without_marker(def_marked);
    let use_src = "\
fn f<n: Nat>(x: QReg<n>): QReg<n> = x
type Box<n> = QReg<n>
fn g(n: Nat): Int = (0 : QReg<n>)
fn h(n: Nat): Int = id<n>(0)
fn call(): Int = n()
";
    let index = index_of(&[(&def_url, &def_src), (&use_url, use_src)]);
    let analysis = analyze(&def_src).intelligence;
    let edit = rename_in_workspace(
        &analysis,
        &def_url,
        position_after_marker(def_marked),
        "n2",
        &index,
    )
    .expect("rename ok")
    .expect("workspace edit");
    let changes = edit.changes.expect("changes");
    let rewritten = edited_text(use_src, &changes[&use_url]);
    assert_eq!(
        rewritten,
        "\
fn f<n: Nat>(x: QReg<n>): QReg<n> = x
type Box<n> = QReg<n>
fn g(n: Nat): Int = (0 : QReg<n>)
fn h(n: Nat): Int = id<n>(0)
fn call(): Int = n2()
"
    );
}

#[test]
fn borrow_annotation_is_shadowed() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let def_marked = "fn /*cursor*/n(): Int = 1\n";
    let def_src = src_without_marker(def_marked);
    let use_src = "\
fn f(): Q<Int> = run {
  borrow n: QReg<n>, m: QReg<n> in {
    return 0
  }
}
fn call(): Int = n()
";
    let index = index_of(&[(&def_url, &def_src), (&use_url, use_src)]);
    let analysis = analyze(&def_src).intelligence;
    let edit = rename_in_workspace(
        &analysis,
        &def_url,
        position_after_marker(def_marked),
        "n2",
        &index,
    )
    .expect("rename ok")
    .expect("workspace edit");
    let changes = edit.changes.expect("changes");
    let rewritten = edited_text(use_src, &changes[&use_url]);
    assert_eq!(
        rewritten,
        "\
fn f(): Q<Int> = run {
  borrow n: QReg<n>, m: QReg<n> in {
    return 0
  }
}
fn call(): Int = n2()
"
    );
}

#[test]
fn circuit_call_before_let_is_renamed() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let def_marked = "fn /*cursor*/helper(): Int = 1\n";
    let def_src = src_without_marker(def_marked);
    let use_src = "fn f(): Int =\n  circuit {\n    helper()\n    let helper = 1\n  }\n";
    let index = index_of(&[(&def_url, &def_src), (&use_url, use_src)]);
    let analysis = analyze(&def_src).intelligence;
    let edit = rename_in_workspace(
        &analysis,
        &def_url,
        position_after_marker(def_marked),
        "helper2",
        &index,
    )
    .expect("rename ok")
    .expect("workspace edit");
    let changes = edit.changes.expect("changes");
    let rewritten = edited_text(use_src, &changes[&use_url]);
    assert_eq!(
        rewritten,
        "fn f(): Int =\n  circuit {\n    helper2()\n    let helper = 1\n  }\n"
    );
}

#[test]
fn shadowed_parameter_stays_in_file() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let def_src = "fn helper(): Int = 1\n";
    let use_marked = "fn f(helper: Int): Int = /*cursor*/helper\n";
    let use_src = src_without_marker(use_marked);
    let index = index_of(&[(&def_url, def_src), (&use_url, &use_src)]);
    let analysis = analyze(&use_src).intelligence;
    let response = definition_in_workspace(
        &analysis,
        &use_url,
        position_after_marker(use_marked),
        &index,
    )
    .expect("local definition");
    let GotoDefinitionResponse::Scalar(loc) = response else {
        panic!("shadowed parameter should stay in-file");
    };
    assert_eq!(loc.uri, use_url);
}

#[test]
fn other_directory_is_a_different_namespace() {
    let def_url = url("other", "defs");
    let use_url = url("proj", "uses");
    let def_src = "fn helper(): Int = 1\n";
    let use_marked = "fn f(): Int = /*cursor*/helper()\n";
    let use_src = src_without_marker(use_marked);
    let index = index_of(&[(&def_url, def_src), (&use_url, &use_src)]);
    let analysis = analyze(&use_src).intelligence;
    assert!(
        definition_in_workspace(
            &analysis,
            &use_url,
            position_after_marker(use_marked),
            &index,
        )
        .is_none()
    );
}

#[test]
fn prepare_rename_accepts_unresolved_sibling_name() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let use_marked = "fn f(): Int = /*cursor*/helper()\n";
    let use_src = src_without_marker(use_marked);
    let index = index_of(&[(&def_url, "fn helper(): Int = 1\n"), (&use_url, &use_src)]);
    let analysis = analyze(&use_src).intelligence;
    let prepared = prepare_rename_in_workspace(
        &analysis,
        &use_url,
        position_after_marker(use_marked),
        &index,
    )
    .expect("prepare")
    .expect("range");
    match prepared {
        tower_lsp::lsp_types::PrepareRenameResponse::RangeWithPlaceholder {
            placeholder, ..
        } => {
            assert_eq!(placeholder, "helper");
        }
        other => panic!("unexpected prepare response: {other:?}"),
    }
}

#[test]
fn type_alias_definition_jumps_to_sibling_file() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let def_src = "type Alias = Int\n";
    let use_marked = "fn f(x: /*cursor*/Alias): Int = x\n";
    let use_src = src_without_marker(use_marked);
    let index = index_of(&[(&def_url, def_src), (&use_url, &use_src)]);
    let analysis = analyze(&use_src).intelligence;
    let response = definition_in_workspace(
        &analysis,
        &use_url,
        position_after_marker(use_marked),
        &index,
    )
    .expect("type definition");
    let GotoDefinitionResponse::Scalar(loc) = response else {
        panic!("expected one type definition, got {response:?}");
    };
    assert_eq!(loc.uri, def_url);
    assert_eq!(loc.range.start.character, 5);
}

#[test]
fn rename_refuses_an_existing_sibling_export() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let use_marked = "fn f(): Int = /*cursor*/helper()\n";
    let use_src = src_without_marker(use_marked);
    let index = index_of(&[
        (&def_url, "fn helper(): Int = 1\nfn taken(): Int = 2\n"),
        (&use_url, &use_src),
    ]);
    let analysis = analyze(&use_src).intelligence;
    let err = rename_in_workspace(
        &analysis,
        &use_url,
        position_after_marker(use_marked),
        "taken",
        &index,
    )
    .expect_err("collision");
    let message = err.message.as_ref();
    assert!(
        message.contains("taken"),
        "error should name the collision, got {message}"
    );
}

#[test]
fn rename_of_local_does_not_edit_sibling_export() {
    let def_url = url("proj", "defs");
    let use_url = url("proj", "uses");
    let def_src = "fn helper(): Int = 1\n";
    let use_marked = "fn f(helper: Int): Int = /*cursor*/helper\n";
    let use_src = src_without_marker(use_marked);
    let index = index_of(&[(&def_url, def_src), (&use_url, &use_src)]);
    let analysis = analyze(&use_src).intelligence;
    let edit = rename_in_workspace(
        &analysis,
        &use_url,
        position_after_marker(use_marked),
        "helper2",
        &index,
    )
    .expect("rename ok")
    .expect("edit");
    let changes = edit.changes.expect("changes");
    assert!(!changes.contains_key(&def_url));
    assert_eq!(changes[&use_url].len(), 2);
}

#[test]
fn stale_or_closed_version_does_not_become_the_open_buffer() {
    let uri = url("proj", "defs");
    let mut docs = quon_lsp::document::DocumentStore::default();
    let mut index = WorkspaceIndex::default();
    docs.open(uri.clone(), "fn live(): Int = 1\n".into(), 2);
    let live = analyze("fn live(): Int = 1\n").intelligence;
    assert!(index.commit_open_analysis(&docs, uri.clone(), 2, live));
    assert!(index.is_open_buffer(&uri));

    let stale = analyze("fn stale(): Int = 1\n").intelligence;
    assert!(!index.commit_open_analysis(&docs, uri.clone(), 1, stale.clone()));
    assert_eq!(index.definition_locations(&uri, "live").len(), 1);
    assert!(index.definition_locations(&uri, "stale").is_empty());

    docs.close(&uri);
    index.note_closed(&uri);
    assert!(!index.commit_open_analysis(&docs, uri.clone(), 2, stale));
    assert!(!index.is_open_buffer(&uri));
}

#[test]
fn open_buffer_wins_over_disk_rescan() {
    let live = url("proj", "defs");
    let mut index = WorkspaceIndex::default();
    index.upsert_open(live.clone(), analyze("fn live(): Int = 1\n").intelligence);
    index.replace_disk_units(vec![(
        live.clone(),
        analyze("fn disk(): Int = 2\n").intelligence,
    )]);
    assert_eq!(index.definition_locations(&live, "live").len(), 1);
    assert!(index.definition_locations(&live, "disk").is_empty());
}

#[test]
fn disk_scan_skips_target_directory() {
    let root: PathBuf = std::env::temp_dir().join(format!("quon-lsp-scan-{}", process::id()));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::create_dir_all(root.join("target")).expect("target dir");
    fs::write(root.join("src/a.qn"), "fn helper(): Int = 1\n").expect("write src");
    fs::write(root.join("target/skip.qn"), "fn skip(): Int = 1\n").expect("write target");
    let scanned = WorkspaceIndex::scan_paths(std::slice::from_ref(&root));
    let _ = fs::remove_dir_all(&root);
    assert_eq!(scanned.len(), 1);
    let name = scanned[0].0.path();
    assert!(name.contains("src/a.qn") || name.contains("src\\a.qn"));
}
