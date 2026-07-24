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
//!   run enforced in `just ci-rust` via the `--release --include-ignored`
//!   step with `QUON_RAP_TABLE_I_ENFORCE=1` set (same pattern as
//!   `quon_lsp/tests/smoke.rs`). Dump-only by default; under the enforce flag
//!   it hard-asserts the Phase-2b mechanism-legitimacy invariants (search
//!   completes, aware beats agnostic by the floor, agnostic matches the
//!   paper) while keeping the aware-vs-published *absolute* SOFT.
//!
//! A third test, [`ising_n98_preflight_and_dump_metrics`], covers the n = 98
//! row added by issue #306 (`test/na/ising_n98.qn`). Unlike n42, n = 98 is
//! **not** a CI anchor (issue #111's locked decision, reaffirmed by #306) —
//! this test is `#[ignore]`d in its entirety (structural pre-flight *and*
//! metric dump together, not split like n42's two tests) and is exercised by
//! `just na-rap-sweep` / `python/na_rap_table_i_sweep.py`, not `just ci-rust`.
//! See `docs/neutral_atom/rap_table_i_methodology.md`'s "n = 98" section.

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

/// Issue #306: n = 98 companion fixture. Same pinned target as n42
/// ([`na_target`]) — only the circuit changes.
fn source_n98() -> PathBuf {
    workspace_path("../test/na/ising_n98.qn")
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
/// Relative floor for "meaningful aware improvement": aware rearrangement_steps
/// must be at most this fraction of agnostic's (aware beats agnostic by ≥
/// 1 − this = 15%). Locked at Phase-2b sign-off: [RAP] reports −59% steps, but
/// this crate's reproduction (post-#297) lands at 18 vs 23 = −22%, an accepted
/// mechanism divergence (see docs/neutral_atom/rap_table_i_methodology.md
/// "Divergences from the paper's numbers (post-297)"). 0.85 lets that pass
/// (18/23 = 0.783 ≤ 0.85) while failing a no-op tie (23/23 = 1.0) — the real
/// guard is that aware genuinely beats agnostic, not that it hits the paper's
/// absolute.
const AWARE_IMPROVEMENT_FLOOR_FRAC: f64 = 0.85;

fn enforce_enabled() -> bool {
    std::env::var("QUON_RAP_TABLE_I_ENFORCE").as_deref() == Ok("1")
}

/// Run `quonc` on `src` with the given `--na-placer` mode and parse the
/// `--emit-resource-report` JSON from stdout. Generalized over the source
/// path (issue #306) so the n = 98 fixture can reuse this without
/// duplicating the plumbing.
fn resource_report_for(src: &std::path::Path, placer: &str) -> Value {
    let output = quonc()
        .arg(src)
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

/// n42-fixture-specific convenience wrapper over [`resource_report_for`].
fn resource_report(placer: &str) -> Value {
    resource_report_for(&source(), placer)
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

    // Issue #308 hard guard: the pinned RAP Table I target MUST keep the √-law
    // timing model — the Table I reproduction uses the paper's √-law, not the
    // jerk-limited model (rap_table_i_methodology.md "Timing model"; switching
    // it would silently stop reproducing this paper's numbers). Assert the kind
    // directly on the JSON so nobody flips it without failing this CI anchor.
    let target_json: Value = serde_json::from_str(
        &std::fs::read_to_string(na_target()).expect("read pinned target JSON"),
    )
    .expect("parse pinned target JSON");
    let kind = target_json
        .pointer("/movement/speed_model/kind")
        .and_then(Value::as_str)
        .expect("pinned target must carry movement.speed_model.kind");
    assert_eq!(
        kind, "sqrt",
        "rap_table_i.json speed_model.kind must stay `sqrt` (the [RAP] Table I √-law); \
         got `{kind}` — see docs/neutral_atom/rap_table_i_methodology.md"
    );
}

/// Dump + Phase-2b enforcement test: records both placers' rearrangement
/// metrics and compares them to the published row. Under
/// `QUON_RAP_TABLE_I_ENFORCE=1` (now wired into `just ci-rust`), hard-asserts
/// the mechanism-legitimacy invariants — see the enforce block below.
///
/// Structural checks (circuit/layer counts on both placers, and the
/// routing-aware search's completed-vs-fell-back diagnostic being present and
/// printed) are **hard** asserts regardless of `QUON_RAP_TABLE_I_ENFORCE` —
/// under `just ci-rust`'s `set -euo pipefail`, these gate CI. The
/// numeric-aware-vs-published-absolute comparison stays SOFT (eprintln
/// warning, never asserted) even under the enforce flag, because the
/// 18/2999-vs-9/1600 gap is an accepted mechanism divergence under Phase-2b
/// sign-off, not a regression (see the enforce-block comment and
/// docs/neutral_atom/rap_table_i_methodology.md "Divergences from the paper's
/// numbers (post-297)").
#[test]
#[ignore = "routing-aware A* is ~90s in --release, far slower in debug; \
            run enforced via `just ci-rust` (QUON_RAP_TABLE_I_ENFORCE=1, \
            --release --include-ignored) or locally with \
            `cargo test --release -p quonc --test rap_table_i ising_n42 -- --include-ignored --nocapture`"]
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

    // Issue #300: the routing-agnostic path now records which placement
    // mechanism produced the schedule (matching vs greedy_fallback) — a
    // zoned routing-agnostic compile must always emit it (Some), so a missing
    // field is a structural regression, not an absent-optional no-op.
    let agnostic_mechanism = agnostic
        .get("agnostic_placer_mechanism")
        .and_then(Value::as_str)
        .unwrap_or_else(|| {
            panic!(
                "routing-agnostic resource report missing `agnostic_placer_mechanism` \
                 (zoned NA backend must always emit it — see AgnosticPlacerMechanism): {agnostic}"
            )
        });

    let steps_ratio = agnostic_steps as f64 / aware_steps.max(1) as f64;
    let time_ratio = agnostic_time_us as f64 / aware_time_us.max(1) as f64;

    println!("--- RAP Table I (#111) — ising n=42 ---");
    println!(
        "{:<18} {:>8} {:>8} {:>14} {:>10} {:>16} {:>18} {:>16}",
        "placer",
        "steps",
        "(paper)",
        "time_us(move)",
        "(paper)",
        "transfer_us",
        "aware search",
        "agnostic mech"
    );
    println!(
        "{:<18} {:>8} {:>8} {:>14} {:>10} {:>16} {:>18} {:>16}",
        "routing-agnostic",
        agnostic_steps,
        PUBLISHED_AGNOSTIC_STEPS,
        agnostic_time_us,
        PUBLISHED_AGNOSTIC_TIME_US,
        agnostic_transfer_us,
        "n/a",
        agnostic_mechanism
    );
    println!(
        "{:<18} {:>8} {:>8} {:>14} {:>10} {:>16} {:>18} {:>16}",
        "routing-aware",
        aware_steps,
        PUBLISHED_AWARE_STEPS,
        aware_time_us,
        PUBLISHED_AWARE_TIME_US,
        aware_transfer_us,
        format!("{aware_completed} ok / {aware_fell_back} fell back"),
        "n/a"
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
             [RAP]'s reproduction requires a meaningful aware improvement — the \
             meaningful-improvement floor (aware ≤ {:.0}% of agnostic, locked at \
             Phase-2b sign-off) is hard-asserted under QUON_RAP_TABLE_I_ENFORCE=1.",
            AWARE_IMPROVEMENT_FLOOR_FRAC * 100.0,
        );
    }

    if !enforce_enabled() {
        return;
    }

    // Phase 2b: hard tolerance asserts (only reached with
    // QUON_RAP_TABLE_I_ENFORCE=1). See docs/neutral_atom/rap_table_i_methodology.md
    // "Tolerances". The enforceable invariants are:
    //   (a) the routing-aware A* search *completes*
    //       (`aware_search_fell_back_layers == 0`) — the #297 acceptance
    //       criterion; falling back to greedy would make "routing-aware"
    //       byte-for-byte indistinguishable from the routing-agnostic
    //       baseline, which is exactly the Phase 1 regression this gate
    //       exists to catch.
    //   (b) routing-aware *genuinely beats* routing-agnostic by at least the
    //       `AWARE_IMPROVEMENT_FLOOR_FRAC` floor — a tie / no-improvement is a
    //       real regression of the mechanism [RAP] reproduces.
    //   (c) the routing-agnostic baseline matches the paper's published
    //       22-step / 3.1 ms row — agnostic is the faithful,
    //       mechanism-complete baseline, so drift there is a real regression.
    // The routing-aware *absolute* (18 steps / 2999 µs vs published 9 / 1600)
    // is a documented mechanism divergence accepted under Phase-2b sign-off —
    // this crate's search reassigns entanglement-zone placement only, not
    // storage-zone; implements no Eq. (5) cross-layer look-ahead; and trades
    // completeness for speed via a beam width (see docs/neutral_atom/
    // rap_table_i_methodology.md "Divergences from the paper's numbers
    // (post-297)"). It is therefore NOT a CI-guardable regression and stays
    // SOFT (eprintln warning) even under the enforce flag.
    assert_eq!(
        aware_fell_back, 0,
        "routing-aware search fell back to greedy on {aware_fell_back} layer(s) — the \
         #297 acceptance criterion (search completes, 0 fallback) regressed; \
         routing-aware is no longer a completed A* search"
    );
    assert!(
        (agnostic_steps as i64 - PUBLISHED_AGNOSTIC_STEPS as i64).abs() <= STEP_TOLERANCE,
        "routing-agnostic steps {agnostic_steps} outside ±{STEP_TOLERANCE} of published \
         {PUBLISHED_AGNOSTIC_STEPS}"
    );
    assert_time_within_tolerance(
        agnostic_time_us,
        PUBLISHED_AGNOSTIC_TIME_US,
        "routing-agnostic",
    );
    assert!(
        (aware_steps as f64) <= (agnostic_steps as f64) * AWARE_IMPROVEMENT_FLOOR_FRAC,
        "routing-aware ({aware_steps} steps) does not meaningfully improve on \
         routing-agnostic ({agnostic_steps} steps) — expected ≤ {:.0}% \
         (meaningful-improvement floor locked at Phase-2b sign-off)",
        AWARE_IMPROVEMENT_FLOOR_FRAC * 100.0
    );

    // SOFT: the aware-vs-published *absolute* comparison. Never asserted — the
    // 18/2999 vs 9/1600 gap is an accepted mechanism divergence (see the
    // comment above), not a regression. Printed as a warning for visibility so
    // drift is observable in CI logs without gating on the paper's number.
    let aware_steps_gap = aware_steps as i64 - PUBLISHED_AWARE_STEPS as i64;
    if aware_steps_gap.abs() > STEP_TOLERANCE {
        eprintln!(
            "SOFT WARNING (not asserted): routing-aware steps {aware_steps} are outside \
             ±{STEP_TOLERANCE} of the published {PUBLISHED_AWARE_STEPS} \
             (gap = {aware_steps_gap:+}). This is an accepted mechanism divergence under \
             Phase-2b sign-off, not a regression: this crate's search reassigns \
             entanglement-zone placement only (not storage-zone), implements no Eq. (5) \
             cross-layer look-ahead, and trades completeness for speed via a beam width. \
             See docs/neutral_atom/rap_table_i_methodology.md 'Divergences from the paper's \
             numbers (post-297)'."
        );
    }
    let aware_time_published = PUBLISHED_AWARE_TIME_US as f64;
    let aware_time_rel_err =
        (aware_time_us as f64 - aware_time_published).abs() / aware_time_published;
    if aware_time_rel_err > TIME_TOLERANCE_FRAC {
        eprintln!(
            "SOFT WARNING (not asserted): routing-aware rearrangement_time_us \
             {aware_time_us} is outside ±{:.0}% of the published {PUBLISHED_AWARE_TIME_US} \
             (rel err = {:.1}%). This is an accepted mechanism divergence under Phase-2b \
             sign-off, not a regression: this crate's search reassigns entanglement-zone \
             placement only (not storage-zone), implements no Eq. (5) cross-layer look-ahead, \
             and trades completeness for speed via a beam width. See \
             docs/neutral_atom/rap_table_i_methodology.md 'Divergences from the paper's \
             numbers (post-297)'.",
            TIME_TOLERANCE_FRAC * 100.0,
            aware_time_rel_err * 100.0,
        );
    }
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

/// [RAP] Table I `ising` n = 98 pre-flight structure: two Trotter steps of a
/// 98-qubit chain, even (49-gate) / odd (48-gate) matchings per step —
/// 2*(49+48) = 194 two-qubit gates over 4 entangling layers. See
/// `test/na/ising_n98.qn`'s header comment for the full construction
/// rationale (identical to `ising_n42.qn`, scaled).
const EXPECTED_N98_ENTANGLE2_COUNT: u64 = 194;
const EXPECTED_N98_RYDBERG_STAGES: u64 = 4;

/// Published [RAP] Table I anchors, `ising` n = 98 (Sec. VI-B): 23 -> 12
/// rearrangement steps. [RAP] does not publish a rearrangement-*time* number
/// for this row (only steps), so this test — unlike the n42 dump test —
/// prints just the step comparison, not a time_us comparison against a
/// published figure.
const PUBLISHED_N98_AGNOSTIC_STEPS: u64 = 23;
const PUBLISHED_N98_AWARE_STEPS: u64 = 12;

/// Issue #306: n = 98 companion to [`ising_n42_dumps_both_placer_rearrangement_metrics`].
/// Combines the structural pre-flight and the metric dump into one test
/// (unlike n42's split) because n = 98 is **not** a CI anchor at all (issue
/// #111's locked decision, reaffirmed by #306) — there is no fast/un-ignored
/// variant to keep out of `--release`. Dump-only: never hard-fails on
/// numeric drift from the published row (no Phase 2 enforcement flag exists
/// for this row; see docs/neutral_atom/rap_table_i_methodology.md's "n = 98"
/// section). Exercised via `just na-rap-sweep` /
/// `cargo test --release -p quonc --test rap_table_i -- --ignored ising_n98 --nocapture`,
/// not `just ci-rust`.
#[test]
#[ignore = "n=98 is local-only (not a CI anchor, issue #111/#306); routing-aware is a \
            double-digit-second run in --release. Run via `just na-rap-sweep` or \
            `cargo test --release -p quonc --test rap_table_i -- --ignored ising_n98 --nocapture`"]
fn ising_n98_preflight_and_dump_metrics() {
    let src = source_n98();
    let agnostic = resource_report_for(&src, "routing-agnostic");
    let aware = resource_report_for(&src, "routing-aware");

    for (label, report) in [("routing-agnostic", &agnostic), ("routing-aware", &aware)] {
        assert_eq!(
            u64_field(report, "entangle2_count"),
            EXPECTED_N98_ENTANGLE2_COUNT,
            "{label}: ising_n98.qn pre-flight gate count regressed"
        );
        assert_eq!(
            u64_field(report, "rydberg_stages"),
            EXPECTED_N98_RYDBERG_STAGES,
            "{label}: ising_n98.qn pre-flight layer count regressed"
        );
    }
    assert_eq!(u64_field(&agnostic, "logical_qubits"), 98);

    let agnostic_steps = u64_field(&agnostic, "rearrangement_steps");
    let aware_steps = u64_field(&aware, "rearrangement_steps");
    let agnostic_time_us = u64_field(&agnostic, "rearrangement_time_us");
    let aware_time_us = u64_field(&aware, "rearrangement_time_us");
    let agnostic_transfer_us = u64_field(&agnostic, "transfer_time_us");
    let aware_transfer_us = u64_field(&aware, "transfer_time_us");

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

    println!("--- RAP Table I (#111/#306) — ising n=98 (local-only) ---");
    println!(
        "{:<18} {:>10} {:>10} {:>14} {:>18} {:>18}",
        "placer", "steps", "(paper)", "time_us(move)", "transfer_us", "aware search"
    );
    println!(
        "{:<18} {:>10} {:>10} {:>14} {:>18} {:>18}",
        "routing-agnostic",
        agnostic_steps,
        PUBLISHED_N98_AGNOSTIC_STEPS,
        agnostic_time_us,
        agnostic_transfer_us,
        "n/a"
    );
    println!(
        "{:<18} {:>10} {:>10} {:>14} {:>18} {:>18}",
        "routing-aware",
        aware_steps,
        PUBLISHED_N98_AWARE_STEPS,
        aware_time_us,
        aware_transfer_us,
        format!("{aware_completed} ok / {aware_fell_back} fell back")
    );

    if aware_fell_back > 0 {
        eprintln!(
            "FALLBACK: routing-aware fell back to greedy on {aware_fell_back} of \
             {} layer-assignment call(s) on ising_n98 ({aware_completed} completed). At this \
             larger scale (up to 49 simultaneous gates/layer, vs n42's 20-21), the default \
             AwareSearchParams (node_budget=100_000, beam_width=2_000 — tuned empirically on \
             ising_n42, see docs/neutral_atom/rap_table_i_methodology.md's Phase 2a status) \
             are not necessarily sufficient; a real finding, not a bug, and left as a Phase 2b \
             / follow-up tuning question rather than fixed here.",
            aware_completed + aware_fell_back,
        );
    }
    if aware_steps >= agnostic_steps {
        eprintln!(
            "WARNING: routing-aware ({aware_steps} steps) is not better than routing-agnostic \
             ({agnostic_steps} steps) on ising_n98 — see the FALLBACK note above if \
             aware_search_fell_back_layers > 0; this is expected when the search doesn't \
             complete, not a new/separate finding."
        );
    }
}
