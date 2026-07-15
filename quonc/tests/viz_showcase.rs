//! Pins the checked-in viz showcase goldens under
//! `samples/visualization/goldens/` (issue #189) against regeneration: each
//! test below re-runs the exact `quonc` invocation documented in
//! `samples/visualization/refresh_goldens.sh` and compares the result
//! against the committed golden, so the goldens can't silently drift out
//! from under the showcase README's prose without a test failing here
//! first. Mirrors `quonc/tests/na_showcase.rs`'s pinning shape (#192).
//!
//! Regenerate goldens after an intentional change with
//! `samples/visualization/refresh_goldens.sh`.

use std::path::PathBuf;
use std::process::{Command, Output};

use serde_json::Value;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(rel)
}

fn golden(rel: &str) -> String {
    let path = workspace_path(&format!("samples/visualization/goldens/{rel}"));
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("failed to read golden {}: {e}", path.display()))
}

fn run(args: &[&str]) -> Output {
    quonc()
        .current_dir(workspace_path("."))
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("failed to spawn quonc with {args:?}: {e}"))
}

fn stdout_text(output: &Output) -> String {
    assert!(
        output.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// Metrics goldens only pin the stable `metrics` object (depth, gate_count,
/// t_count, qubit_count, swap_count) — the full `--metrics-json` snapshot
/// also carries `toolchain.git_commit` and `compile.compile_ms`, which are
/// not reproducible across commits/runs and must never be part of a golden.
fn metrics_object(args: &[&str]) -> Value {
    let output = run(args);
    let text = stdout_text(&output);
    let full: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse metrics JSON: {e}\n{text}"));
    full["metrics"].clone()
}

fn assert_metrics_golden(golden_rel: &str, args: &[&str]) {
    let expected: Value = serde_json::from_str(&golden(golden_rel))
        .unwrap_or_else(|e| panic!("parse golden {golden_rel}: {e}"));
    let actual = metrics_object(args);
    assert_eq!(
        expected["metrics"], actual,
        "regenerated metrics for {golden_rel} no longer match the committed golden — \
         refresh with samples/visualization/refresh_goldens.sh"
    );
}

fn assert_stdout_golden(golden_rel: &str, args: &[&str]) {
    let expected = golden(golden_rel);
    let output = run(args);
    let actual = stdout_text(&output);
    assert_eq!(
        expected, actual,
        "regenerated output for {golden_rel} no longer matches the committed golden — \
         refresh with samples/visualization/refresh_goldens.sh"
    );
}

/// Replicates `refresh_goldens.sh`'s awk extraction: lines strictly between
/// (not including) `start_marker` and `end_marker`.
fn extract_stage(dump: &str, start_marker: &str, end_marker: &str) -> String {
    let mut flag = false;
    let mut out = String::new();
    for line in dump.lines() {
        if line == start_marker {
            flag = true;
            continue;
        }
        if line == end_marker {
            flag = false;
            continue;
        }
        if flag {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

const IBM_TARGET: &str = "targets/ibm/fake_manila_v2.json";
const NA_TARGET: &str = "targets/neutral_atom/generic_rna_v0.json";

#[test]
fn dense_swap_mismatch_goldens_match_regeneration() {
    assert_stdout_golden(
        "dense_swap_mismatch/qaoa_manila.qasm",
        &["--target", IBM_TARGET, "--emit-qasm", "-q", "test/verify/qaoa.qn"],
    );
    assert_metrics_golden(
        "dense_swap_mismatch/metrics.json",
        &["--target", IBM_TARGET, "--metrics-json", "-", "-q", "test/verify/qaoa.qn"],
    );

    // The showcase's specific claim (#135 review-relevant honesty check):
    // the 3-CNOT SWAP-network pattern for the non-adjacent (0,2) interaction
    // is really in the QASM, and `swap_count` really is 0 despite it.
    let qasm = golden("dense_swap_mismatch/qaoa_manila.qasm");
    assert!(
        qasm.contains("cx q[1], q[2];\ncx q[2], q[1];\ncx q[1], q[2];"),
        "expected a literal 3-CNOT SWAP-network pattern in the QASM: {qasm}"
    );
    let metrics = metrics_object(&[
        "--target",
        IBM_TARGET,
        "--metrics-json",
        "-",
        "-q",
        "test/verify/qaoa.qn",
    ]);
    assert_eq!(
        metrics["swap_count"], 0,
        "swap_count must stay 0 even though a SWAP network is present — this mismatch \
         is the point of the showcase entry, see samples/visualization/README.md §1"
    );
}

#[test]
fn teleport_dynamic_goldens_match_regeneration() {
    assert_stdout_golden(
        "teleport_dynamic/teleport.qasm",
        &["--emit-qasm", "-q", "test/verify/teleport.qn"],
    );
    assert_metrics_golden(
        "teleport_dynamic/metrics.json",
        &["--metrics-json", "-", "-q", "test/verify/teleport.qn"],
    );

    // The showcase's claim: measurement_deferral means no literal `if` makes
    // it into the compiled QASM, despite the source having real branches.
    let qasm = golden("teleport_dynamic/teleport.qasm");
    assert!(
        !qasm.contains("if ("),
        "teleport.qn's compiled QASM must not contain a literal feed-forward `if` \
         by default (measurement_deferral, SPEC §7.1): {qasm}"
    );
}

#[test]
fn qft_depth_goldens_match_regeneration() {
    let output = run(&["--emit-qasm", "--dump-ir", "-q", "test/verify/qft.qn"]);
    assert!(
        output.status.success(),
        "quonc failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let dump = String::from_utf8_lossy(&output.stderr).into_owned();

    let before = extract_stage(&dump, "--- after lowering ---", "--- after circ passes ---");
    let after = extract_stage(
        &dump,
        "--- after circ passes ---",
        "--- after monadic lowering ---",
    );

    assert_eq!(
        golden("qft_depth/before_optimization.mlir"),
        before,
        "regenerated 'after lowering' MLIR no longer matches the committed golden — \
         refresh with samples/visualization/refresh_goldens.sh"
    );
    assert_eq!(
        golden("qft_depth/after_optimization.mlir"),
        after,
        "regenerated 'after circ passes' MLIR no longer matches the committed golden — \
         refresh with samples/visualization/refresh_goldens.sh"
    );
    assert_metrics_golden(
        "qft_depth/metrics.json",
        &["--metrics-json", "-", "-q", "test/verify/qft.qn"],
    );

    // The showcase's claim: the round trip's dedicated function body
    // collapses to a bare pass-through — no gates left in that function.
    assert!(
        before.contains("gate_name = \"SWAP\""),
        "expected the pre-optimization QFT round trip to contain SWAP gates \
         (from swap_reverse): {before}"
    );
    let sym_marker = "sym_name = \"qft_roundtrip__elab0\"";
    let sym_pos = after
        .find(sym_marker)
        .unwrap_or_else(|| panic!("expected {sym_marker} in post-optimization MLIR: {after}"));
    let func_marker = "\"quantum.circ.func\"() ({";
    let func_start = after[..sym_pos]
        .rfind(func_marker)
        .unwrap_or_else(|| panic!("expected a preceding {func_marker} before {sym_marker}"));
    let roundtrip_fn_after = &after[func_start..sym_pos];
    assert!(
        roundtrip_fn_after.contains("\"quantum.circ.return\"(%arg0, %arg1, %arg2)")
            && !roundtrip_fn_after.contains("quantum.circ.gate"),
        "expected the post-optimization qft_roundtrip function to have collapsed to a \
         bare pass-through with no gates left: {after}"
    );
}

#[test]
fn na_interaction_graph_golden_matches_regeneration() {
    assert_stdout_golden(
        "na_interaction_graph/qaoa_graph.dot",
        &[
            "--target",
            NA_TARGET,
            "--na-backend",
            "zoned",
            "--emit-na-graph",
            "-",
            "-q",
            "test/na/qaoa_graph.qn",
        ],
    );
}

#[test]
fn na_schedule_metrics_goldens_match_regeneration() {
    assert_stdout_golden(
        "na_schedule_metrics/bell_zoned.resource_report.json",
        &[
            "--target",
            NA_TARGET,
            "--na-backend",
            "zoned",
            "--emit-resource-report",
            "-",
            "-q",
            "--verify-na",
            "test/na/bell.qn",
        ],
    );
    assert_stdout_golden(
        "na_schedule_metrics/qaoa_graph_zoned.resource_report.json",
        &[
            "--target",
            NA_TARGET,
            "--na-backend",
            "zoned",
            "--emit-resource-report",
            "-",
            "-q",
            "--verify-na",
            "test/na/qaoa_graph.qn",
        ],
    );
}

#[test]
fn noise_aware_target_overlay_goldens_match_regeneration() {
    assert_stdout_golden(
        "noise_target_overlay/ising_manila.qasm",
        &["--target", IBM_TARGET, "--emit-qasm", "-q", "test/verify/ising.qn"],
    );
    assert_metrics_golden(
        "noise_target_overlay/metrics.json",
        &["--target", IBM_TARGET, "--metrics-json", "-", "-q", "test/verify/ising.qn"],
    );

    // The showcase's claim: this circuit maps with zero SWAPs (deliberately
    // clean, so noise — not routing — is the interesting data).
    let metrics = metrics_object(&[
        "--target",
        IBM_TARGET,
        "--metrics-json",
        "-",
        "-q",
        "test/verify/ising.qn",
    ]);
    assert_eq!(
        metrics["swap_count"], 0,
        "ising.qn should map onto fake_manila_v2's line with zero SWAPs"
    );

    // The fixed target's per-edge noise data this entry points at must
    // actually exist and be keyed per-edge, not just asserted in prose.
    let target_text = std::fs::read_to_string(workspace_path(IBM_TARGET))
        .unwrap_or_else(|e| panic!("read {IBM_TARGET}: {e}"));
    let target: Value =
        serde_json::from_str(&target_text).unwrap_or_else(|e| panic!("parse {IBM_TARGET}: {e}"));
    assert!(
        target["noise"]["two_qubit_fidelity"]["cx"]
            .as_object()
            .is_some_and(|m| !m.is_empty()),
        "expected {IBM_TARGET} to carry non-empty per-edge two_qubit_fidelity data"
    );
    assert!(
        target["noise"]["readout_error"]
            .as_object()
            .is_some_and(|m| !m.is_empty()),
        "expected {IBM_TARGET} to carry non-empty per-qubit readout_error data"
    );
}

/// Every showcase entry's golden files must actually be declared in
/// `catalog.yaml`'s `artifacts` list — `samples_catalog.rs`'s
/// `entry_artifacts_exist_when_declared` proves declared artifacts exist,
/// but not that every golden on disk is declared somewhere. This is the
/// converse check.
#[test]
fn every_golden_file_on_disk_is_declared_in_catalog() {
    let goldens_root = workspace_path("samples/visualization/goldens");
    let catalog_text = std::fs::read_to_string(workspace_path("samples/catalog.yaml"))
        .unwrap_or_else(|e| panic!("read catalog.yaml: {e}"));

    let mut on_disk = Vec::new();
    collect_files(&goldens_root, &mut on_disk);
    assert!(!on_disk.is_empty(), "expected at least one viz golden on disk");

    for path in &on_disk {
        let rel = path
            .strip_prefix(workspace_path("."))
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        assert!(
            catalog_text.contains(rel.as_str()),
            "golden file {rel} exists on disk but is not declared in any catalog entry's \
             `artifacts` list"
        );
    }
}

fn collect_files(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display()));
    for entry in entries {
        let entry = entry.unwrap_or_else(|e| panic!("dir entry: {e}"));
        let path = entry.path();
        if path.is_dir() {
            collect_files(&path, out);
        } else {
            out.push(path);
        }
    }
}
