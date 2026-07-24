//! End-to-end `--emit-naviz` tests (Quon issue #303): a zoned neutral-atom
//! compile must write a `.naviz` instruction file plus a sibling `.namachine`,
//! with the `#target` directive matching the `.namachine` file-name stem.
//!
//! Golden snapshots of the emitter text itself live in `quon_na` (`naviz.rs`
//! unit tests, insta). These tests cover the CLI wiring: dual-emit, sibling
//! pairing, target gating, and stdout refusal.

use std::fs;
use std::path::PathBuf;

use std::process::Command;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

fn na_target() -> PathBuf {
    workspace_path("../targets/neutral_atom/generic_rna_v0.json")
}

fn na_source() -> PathBuf {
    workspace_path("../test/na/bell.qn")
}

/// A zoned compile writes both `.naviz` and sibling `.namachine`, the
/// `#target` matches the file stem, and both files carry the expected blocks.
#[test]
fn zoned_compile_emits_naviz_and_namachine() {
    let tmp = tempfile::tempdir().expect("tmp");
    let naviz_path = tmp.path().join("bell.naviz");
    let namachine_path = tmp.path().join("bell.namachine");

    let out = quonc()
        .arg(na_source())
        .arg("--target")
        .arg(na_target())
        .arg("--emit-naviz")
        .arg(&naviz_path)
        .arg("--quiet")
        .output()
        .expect("spawn quonc");
    assert!(
        out.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    assert!(naviz_path.exists(), ".naviz not written");
    assert!(namachine_path.exists(), "sibling .namachine not written");

    let naviz = fs::read_to_string(&naviz_path).unwrap();
    let namachine = fs::read_to_string(&namachine_path).unwrap();

    // #target directive references the .namachine stem ("bell").
    assert!(naviz.starts_with("#target bell\n"), "naviz header: {naviz}");
    // At least one atom setup and one timed instruction group.
    assert!(naviz.contains("atom ("), "no atom setup in: {naviz}");
    assert!(naviz.contains("@+ ["), "no timed group in: {naviz}");

    // .namachine carries the architecture blocks.
    assert!(
        namachine.starts_with("name: \""),
        "namachine header: {namachine}"
    );
    assert!(namachine.contains("movement {"), "{namachine}");
    assert!(namachine.contains("time {"), "{namachine}");
    assert!(namachine.contains("distance {"), "{namachine}");
    assert!(namachine.contains("zone "), "{namachine}");
    assert!(namachine.contains("trap "), "{namachine}");
    // The rydberg range from the target is the interaction distance.
    assert!(namachine.contains("interaction: 7.5"), "{namachine}");
}

/// `--emit-naviz -` is refused (dual-emit needs a filesystem path).
#[test]
fn emit_naviz_rejects_stdout() {
    let out = quonc()
        .arg(na_source())
        .arg("--target")
        .arg(na_target())
        .arg("--emit-naviz")
        .arg("-")
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("--emit-naviz"), "stderr: {stderr}");
    assert!(stderr.contains("PATH"), "stderr: {stderr}");
}

/// `--emit-naviz` requires a neutral-atom target.
#[test]
fn emit_naviz_requires_na_target() {
    let source = workspace_path("../frontend/tests/fixtures/bell_state.qn");
    let out = quonc()
        .arg(&source)
        .arg("--emit-naviz")
        .arg("/tmp/should_not_write.naviz")
        .output()
        .expect("spawn");
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("neutral_atom_reconfigurable") || stderr.contains("--emit-naviz"),
        "stderr: {stderr}"
    );
}
