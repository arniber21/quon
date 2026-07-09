//! Shells out to `lit` to run the IR-level FileCheck suite in `test/lit/`
//! (issue #28, PRD story 38). `cargo test` alone can't drive this suite: the
//! oracle binaries `test/lit/lit.cfg.py` substitutes in (`circ_lower`,
//! `monadic_lower`, `sabre_route`, ...) live under `examples/` in `frontend`
//! and `mlir_bridge`, and plain `cargo test`/`cargo build` don't build
//! examples — CI runs `cargo build --examples --workspace` first. Locally,
//! where that's easy to forget, this test skips (rather than fails) when
//! `lit`, `FileCheck`, or an oracle binary is missing, so a bare `cargo test`
//! stays green on a fresh checkout.

use std::path::PathBuf;
use std::process::Command;

const ORACLE_BINARIES: &[&str] = &[
    "circ_roundtrip",
    "na_roundtrip",
    "circ_lower",
    "dynamic_roundtrip",
    "monadic_lower",
    "gate_cancel",
    "rotation_merge",
    "measurement_defer",
    "classical_region_fuse",
    "native_gate_decomp",
    "sabre_route",
    "depth_schedule",
];

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("quonc crate has a workspace parent")
        .to_path_buf()
}

fn exe_name(name: &str) -> String {
    if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn on_path(new_path: &std::ffi::OsStr, name: &str) -> bool {
    Command::new(name)
        .arg("--version")
        .env("PATH", new_path)
        .output()
        .is_ok()
}

#[test]
fn lit_suite_passes() {
    let root = workspace_root();
    let quonc_bin = PathBuf::from(env!("CARGO_BIN_EXE_quonc"));
    let bin_dir = quonc_bin
        .parent()
        .expect("quonc binary has a parent dir")
        .to_path_buf();
    let examples_dir = bin_dir.join("examples");

    let existing_path = std::env::var_os("PATH").unwrap_or_default();
    let mut search_dirs = vec![bin_dir.clone(), examples_dir.clone()];
    search_dirs.extend(std::env::split_paths(&existing_path));
    let new_path = std::env::join_paths(&search_dirs).expect("build PATH for lit subprocess");

    if !on_path(&new_path, "lit") || !on_path(&new_path, "FileCheck") {
        eprintln!("skipping lit_suite_passes: `lit` and/or `FileCheck` not on PATH");
        return;
    }
    let missing_oracle = ORACLE_BINARIES
        .iter()
        .find(|name| !examples_dir.join(exe_name(name)).is_file());
    if let Some(missing) = missing_oracle {
        eprintln!(
            "skipping lit_suite_passes: oracle binary `{missing}` not built \
             (run `cargo build --examples --workspace` first)"
        );
        return;
    }

    // lit resolves `test_exec_root` (where per-test Output/ dirs go) relative
    // to QUON_BUILD_DIR, defaulting to the process's cwd — which for a cargo
    // test is this crate's directory, not the workspace root. Point it at a
    // scratch dir so a bare `cargo test` doesn't leave `quonc/test/` behind.
    let scratch = std::env::temp_dir().join("quonc-lit-test-exec-root");

    let output = Command::new("lit")
        .arg(root.join("test").join("lit"))
        .arg("-v")
        .env("PATH", &new_path)
        .env("QUON_BUILD_DIR", &scratch)
        .output()
        .expect("failed to spawn lit");

    if !output.status.success() {
        panic!(
            "lit test/lit/ failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
