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

/// `aware_search_completed_layers` / `aware_search_fell_back_layers` (#111
/// review finding) are only emitted for the zoned NA backend and are always
/// present there (`Some(0)` under routing-agnostic) — treat a missing field
/// as a structural regression, not an absent-optional-field no-op.
fn required_u64_field(report: &Value, field: &str) -> u64 {
    report.get(field).and_then(Value::as_u64).unwrap_or_else(|| {
        panic!(
            "resource report missing required u64 field `{field}` (zoned NA backend must always \
             emit routing-aware search diagnostics — see docs/neutral_atom/rap_table_i_methodology.md): {report}"
        )
    })
}

/// Fails first, before any placer comparison, if `ising_n42.qn` or the
/// pinned target regress. Uses the (fast) default routing-agnostic placer:
/// for this fixture/target pairing there is no placement-induced deferral
/// (ample entanglement-zone capacity, no spacing conflicts — see the
/// methodology doc), so `rydberg_stages` here is the same placer-independent
/// circuit property either mode would report, and `entangle2_count` is
/// placer-independent by construction (one Entangle2 action per gate).
/// Deliberately does **not** also run routing-aware (too slow for the
/// default gate, see `Runtime` in the methodology doc); the slower ignored
/// dump test below hard-asserts both placers see the same 82/4 structure
/// directly, so that check is not skipped, only deferred to the release-mode
/// job (finding #5 of the #111 review).
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
///
/// Structural checks (circuit/layer counts on both placers, and the
/// routing-aware search's completed-vs-fell-back diagnostic being present
/// and printed) are **hard** asserts regardless of `QUON_RAP_TABLE_I_ENFORCE`
/// — under `just ci-rust`'s `set -euo pipefail`, this is what actually gates
/// CI. Only the numeric closeness-to-the-published-row comparisons stay soft
/// (println + WARNING, no assert) until Phase 2 sign-off.
#[test]
#[ignore = "routing-aware A* is ~90s in --release, far slower in debug; \
            run via `just ci-rust` (--release --include-ignored) or \
            `cargo test --release -p quonc --test rap_table_i -- --include-ignored --nocapture`"]
fn ising_n42_dumps_both_placer_rearrangement_metrics() {
    let agnostic = resource_report("routing-agnostic");
    let aware = resource_report("routing-aware");

    // Hard, structural: both placers must see the identical 82-gate/4-layer
    // circuit (#111 review finding #5) — a wrong circuit invalidates the
    // whole comparison before it starts, for *either* placer, not just
    // agnostic (which is all the fast preflight test above can check).
    for (label, report) in [("routing-agnostic", &agnostic), ("routing-aware", &aware)] {
        assert_eq!(
            u64_field(report, "entangle2_count"),
            EXPECTED_ENTANGLE2_COUNT,
            "{label}: pre-flight gate count regressed"
        );
        assert_eq!(
            u64_field(report, "rydberg_stages"),
            EXPECTED_RYDBERG_STAGES,
            "{label}: pre-flight layer count regressed"
        );
    }

    let agnostic_steps = u64_field(&agnostic, "rearrangement_steps");
    let aware_steps = u64_field(&aware, "rearrangement_steps");
    // Move-only √-law time (see docs/neutral_atom/rap_table_i_methodology.md
    // "Timing model"): this is what Phase 1 compares to the published
    // "Rearrangement time" column. `transfer_time_us` (load+store trap
    // transfers) is a separate per-atom-instance aggregate, printed for
    // transparency but deliberately *not* folded into the comparison.
    let agnostic_time_us = u64_field(&agnostic, "rearrangement_time_us");
    let aware_time_us = u64_field(&aware, "rearrangement_time_us");
    let agnostic_transfer_us = u64_field(&agnostic, "transfer_time_us");
    let aware_transfer_us = u64_field(&aware, "transfer_time_us");

    // Hard, structural: the routing-aware search's completed-vs-fell-back
    // status must be present (#111 review finding #1/#6) — this is the
    // instrumentation that replaces the retracted "no routing contention"
    // claim. Not soft: a missing field here means the diagnostic silently
    // regressed out of the resource report, which is exactly the kind of
    // silent mischaracterization the review flagged.
    let aware_completed = required_u64_field(&aware, "aware_search_completed_layers");
    let aware_fell_back = required_u64_field(&aware, "aware_search_fell_back_layers");
    let agnostic_completed = required_u64_field(&agnostic, "aware_search_completed_layers");
    let agnostic_fell_back = required_u64_field(&agnostic, "aware_search_fell_back_layers");
    assert_eq!(
        (agnostic_completed, agnostic_fell_back),
        (0, 0),
        "routing-agnostic never runs the aware search; both counters must be 0"
    );
    assert!(
        aware_completed + aware_fell_back >= 1,
        "routing-aware must report at least one layer-assignment outcome (completed or \
         fell back); got 0 of both — aware_search_status wiring regressed"
    );

    let steps_ratio = agnostic_steps as f64 / aware_steps.max(1) as f64;
    let time_ratio = agnostic_time_us as f64 / aware_time_us.max(1) as f64;

    println!("--- RAP Table I (#111) — ising n=42 ---");
    println!(
        "{:<18} {:>10} {:>10} {:>14} {:>14} {:>18} {:>18}",
        "placer", "steps", "(paper)", "time_us(move)", "(paper)", "transfer_us", "aware search"
    );
    println!(
        "{:<18} {:>10} {:>10} {:>14} {:>14} {:>18} {:>18}",
        "routing-agnostic",
        agnostic_steps,
        PUBLISHED_AGNOSTIC_STEPS,
        agnostic_time_us,
        PUBLISHED_AGNOSTIC_TIME_US,
        agnostic_transfer_us,
        "n/a"
    );
    println!(
        "{:<18} {:>10} {:>10} {:>14} {:>14} {:>18} {:>18}",
        "routing-aware",
        aware_steps,
        PUBLISHED_AWARE_STEPS,
        aware_time_us,
        PUBLISHED_AWARE_TIME_US,
        aware_transfer_us,
        format!("{aware_completed} ok / {aware_fell_back} fell back")
    );
    println!(
        "×-agnostic-over-aware ratio: steps={steps_ratio:.2} time={time_ratio:.2} \
         (published: steps={:.2} time={:.2})",
        PUBLISHED_AGNOSTIC_STEPS as f64 / PUBLISHED_AWARE_STEPS as f64,
        PUBLISHED_AGNOSTIC_TIME_US as f64 / PUBLISHED_AWARE_TIME_US as f64,
    );

    if aware_fell_back > 0 {
        eprintln!(
            "FALLBACK: routing-aware fell back from its best-first search to the \
             routing-agnostic greedy assignment on {aware_fell_back} of \
             {} layer-assignment call(s) ({aware_completed} completed the search). On this \
             fixture/target pair the per-layer search space (up to 21 gates × 340 candidate \
             entanglement-zone pairs, uniform-cost h=0) vastly exceeds AWARE_NODE_BUDGET, so \
             this is the *expected*, now-measured mechanism — not evidence that \
             routing-aware and routing-agnostic agree because there is 'no routing \
             contention'. See docs/neutral_atom/rap_table_i_methodology.md 'Phase 1 finding'.",
            aware_completed + aware_fell_back,
        );
    }
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
    assert_time_within_tolerance(
        agnostic_time_us,
        PUBLISHED_AGNOSTIC_TIME_US,
        "routing-agnostic",
    );
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
