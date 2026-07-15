//! RAP Table I regression anchor (#111), Phase 1.
//!
//! Reproduces one row of [RAP] (Stade, Lin, Cong, Wille, ICCAD 2025,
//! arXiv:2505.22715) Table I, Sec. VI-B: `ising` n = 42. See
//! `docs/neutral_atom/rap_table_i_methodology.md` for the full metric
//! mapping, timing model, and Phase 1/2 split. Distinct from the QEC
//! compiler-ablation benchmarks of #254 (`qec_repetition_memory.rs`,
//! `python/quon_qec_benchmarks.py`) — this file is the only place that may
//! compare against the published 22/9 steps or 3.1/1.6 ms numbers.
//!
//! Two tests:
//! - [`ising_n42_preflight_gate_and_layer_counts`] — fast, part of the
//!   default `cargo test --workspace` gate. Fails first if the circuit or
//!   pinned target regresses, before any placer comparison.
//! - [`ising_n42_dumps_both_placer_rearrangement_metrics`] — `#[ignore]`d
//!   (routing-aware search is ~90 s in `--release`, far slower in debug);
//!   run via `just ci-rust`'s dedicated `--release --include-ignored` step
//!   (same pattern as `quon_lsp/tests/smoke.rs`). Dump-only by default;
//!   `QUON_RAP_TABLE_I_ENFORCE=1` switches on Phase 2 hard tolerance asserts.

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn quonc() -> Command {
    Command::new(env!("CARGO_BIN_EXE_quonc"))
}

fn workspace_path(rel: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(rel)
}

/// Pinned freeze of `generic_rna_v0.json` (issue #111) — CI for this
/// regression must never load the drifting generic target directly.
fn na_target() -> PathBuf {
    workspace_path("../targets/neutral_atom/rap_table_i.json")
}

fn source() -> PathBuf {
    workspace_path("../test/na/ising_n42.qn")
}

/// Published [RAP] Table I anchors, `ising` n = 42 (Sec. VI-B).
const PUBLISHED_AGNOSTIC_STEPS: u64 = 22;
const PUBLISHED_AWARE_STEPS: u64 = 9;
const PUBLISHED_AGNOSTIC_TIME_US: u64 = 3100;
const PUBLISHED_AWARE_TIME_US: u64 = 1600;

/// Pre-flight circuit structure (placer-independent): 82 two-qubit gates
/// over 4 entangling layers.
const EXPECTED_ENTANGLE2_COUNT: u64 = 82;
const EXPECTED_RYDBERG_STAGES: u64 = 4;

/// Locked Phase 2 tolerances (documented; not enforced unless
/// `QUON_RAP_TABLE_I_ENFORCE=1` — see docs/neutral_atom/rap_table_i_methodology.md).
const STEP_TOLERANCE: i64 = 2;
const TIME_TOLERANCE_FRAC: f64 = 0.10;
/// Placeholder relative floor for "meaningful aware improvement" (aware
/// rearrangement_steps must be at most this fraction of agnostic's). [RAP]
/// reports −59%; Phase 2 sign-off picks the real floor once the Phase 1
/// finding (aware == agnostic on this target/circuit pair; see the
/// methodology doc) is resolved. Deliberately loose so this constant itself
/// is not mistaken for a validated tolerance.
const AWARE_IMPROVEMENT_FLOOR_FRAC: f64 = 0.70;

fn enforce_enabled() -> bool {
    std::env::var("QUON_RAP_TABLE_I_ENFORCE").as_deref() == Ok("1")
}

/// Run `quonc` on the fixture with the given `--na-placer` mode and parse the
/// `--emit-resource-report` JSON from stdout.
fn resource_report(placer: &str) -> Value {
    let output = quonc()
        .arg(source())
        .arg("--target")
        .arg(na_target())
        .arg("--na-placer")
        .arg(placer)
        .arg("--emit-resource-report")
        .arg("-")
        .arg("--quiet")
        .output()
        .expect("spawn quonc");
    assert!(
        output.status.success(),
        "quonc failed for --na-placer {placer}; stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(&stdout)
        .unwrap_or_else(|e| panic!("parse resource report JSON for {placer}: {e}\n{stdout}"))
}

fn u64_field(report: &Value, field: &str) -> u64 {
    report[field]
        .as_u64()
        .unwrap_or_else(|| panic!("resource report missing u64 field `{field}`: {report}"))
}

/// Fails first, before any placer comparison, if `ising_n42.qn` or the
/// pinned target regress. Uses the (fast) default routing-agnostic placer:
/// for this fixture/target pairing there is no placement-induced deferral
/// (ample entanglement-zone capacity, no spacing conflicts — see the
/// methodology doc), so `rydberg_stages` here is the same placer-independent
/// circuit property either mode would report, and `entangle2_count` is
/// placer-independent by construction (one Entangle2 action per gate).
#[test]
fn ising_n42_preflight_gate_and_layer_counts() {
    let report = resource_report("routing-agnostic");
    assert_eq!(
        u64_field(&report, "entangle2_count"),
        EXPECTED_ENTANGLE2_COUNT,
        "ising_n42.qn must compile to exactly {EXPECTED_ENTANGLE2_COUNT} two-qubit gates \
         ([RAP] Table I ising n=42); got report: {report}"
    );
    assert_eq!(
        u64_field(&report, "rydberg_stages"),
        EXPECTED_RYDBERG_STAGES,
        "ising_n42.qn must schedule into exactly {EXPECTED_RYDBERG_STAGES} entangling layers \
         ([RAP] Table I ising n=42); got report: {report}"
    );
    assert_eq!(u64_field(&report, "logical_qubits"), 42);
}

/// Dump/soft regression test: records both placers' rearrangement metrics
/// and compares them to the published row without failing CI on mismatch,
/// unless `QUON_RAP_TABLE_I_ENFORCE=1` (Phase 2).
#[test]
#[ignore = "routing-aware A* is ~90s in --release, far slower in debug; \
            run via `just ci-rust` (--release --include-ignored) or \
            `cargo test --release -p quonc --test rap_table_i -- --include-ignored --nocapture`"]
fn ising_n42_dumps_both_placer_rearrangement_metrics() {
    let agnostic = resource_report("routing-agnostic");
    let aware = resource_report("routing-aware");

    // Belt-and-suspenders: both modes must still see the same circuit.
    for (label, report) in [("routing-agnostic", &agnostic), ("routing-aware", &aware)] {
        assert_eq!(
            u64_field(report, "entangle2_count"),
            EXPECTED_ENTANGLE2_COUNT,
            "{label}: pre-flight gate count regressed"
        );
    }

    let agnostic_steps = u64_field(&agnostic, "rearrangement_steps");
    let aware_steps = u64_field(&aware, "rearrangement_steps");
    let agnostic_time_us = u64_field(&agnostic, "rearrangement_time_us");
    let aware_time_us = u64_field(&aware, "rearrangement_time_us");

    println!("--- RAP Table I (#111) — ising n=42 ---");
    println!(
        "{:<18} {:>18} {:>18} {:>10}",
        "placer", "steps (paper)", "time_us (paper)", "vs paper"
    );
    println!(
        "{:<18} {:>10} ({:>4}) {:>12} ({:>4}) {:>10}",
        "routing-agnostic",
        agnostic_steps,
        PUBLISHED_AGNOSTIC_STEPS,
        agnostic_time_us,
        PUBLISHED_AGNOSTIC_TIME_US,
        format!("{:+}", agnostic_steps as i64 - PUBLISHED_AGNOSTIC_STEPS as i64)
    );
    println!(
        "{:<18} {:>10} ({:>4}) {:>12} ({:>4}) {:>10}",
        "routing-aware",
        aware_steps,
        PUBLISHED_AWARE_STEPS,
        aware_time_us,
        PUBLISHED_AWARE_TIME_US,
        format!("{:+}", aware_steps as i64 - PUBLISHED_AWARE_STEPS as i64)
    );
    if aware_steps >= agnostic_steps {
        eprintln!(
            "WARNING: routing-aware ({aware_steps} steps) is not better than \
             routing-agnostic ({agnostic_steps} steps) on this target/circuit pair. \
             [RAP]'s reproduction requires a meaningful aware improvement — this is a \
             known Phase 1 finding (see docs/neutral_atom/rap_table_i_methodology.md), \
             not a test failure. Phase 2 sign-off must resolve this before enabling \
             QUON_RAP_TABLE_I_ENFORCE=1."
        );
    }

    if !enforce_enabled() {
        return;
    }

    // Phase 2: hard tolerance asserts (only reached with
    // QUON_RAP_TABLE_I_ENFORCE=1). See docs/neutral_atom/rap_table_i_methodology.md
    // "Tolerances" — these constants are locked decisions; the improvement
    // floor is a placeholder pending Phase 2 sign-off on the Phase 1 finding.
    assert!(
        (agnostic_steps as i64 - PUBLISHED_AGNOSTIC_STEPS as i64).abs() <= STEP_TOLERANCE,
        "routing-agnostic steps {agnostic_steps} outside ±{STEP_TOLERANCE} of published \
         {PUBLISHED_AGNOSTIC_STEPS}"
    );
    assert!(
        (aware_steps as i64 - PUBLISHED_AWARE_STEPS as i64).abs() <= STEP_TOLERANCE,
        "routing-aware steps {aware_steps} outside ±{STEP_TOLERANCE} of published \
         {PUBLISHED_AWARE_STEPS}"
    );
    assert_time_within_tolerance(agnostic_time_us, PUBLISHED_AGNOSTIC_TIME_US, "routing-agnostic");
    assert_time_within_tolerance(aware_time_us, PUBLISHED_AWARE_TIME_US, "routing-aware");
    assert!(
        (aware_steps as f64) <= (agnostic_steps as f64) * AWARE_IMPROVEMENT_FLOOR_FRAC,
        "routing-aware ({aware_steps} steps) does not meaningfully improve on \
         routing-agnostic ({agnostic_steps} steps) — expected ≤ {:.0}% \
         (placeholder floor pending Phase 2 sign-off)",
        AWARE_IMPROVEMENT_FLOOR_FRAC * 100.0
    );
}

fn assert_time_within_tolerance(measured_us: u64, published_us: u64, label: &str) {
    let published = published_us as f64;
    let measured = measured_us as f64;
    let rel_err = (measured - published).abs() / published;
    assert!(
        rel_err <= TIME_TOLERANCE_FRAC,
        "{label} rearrangement_time_us {measured_us} outside ±{:.0}% of published {published_us}",
        TIME_TOLERANCE_FRAC * 100.0
    );
}
