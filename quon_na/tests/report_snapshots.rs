//! Insta regression snapshots for `ResourceReport` JSON and Markdown emitters.
//!
//! These goldens pin the **analytic** compiler artifact (ADR-0020). Sampled
//! Sinter CSV fields (`logical_failures`, …) must never appear here.
//!
//! Update goldens after intentional metric changes:
//! ```bash
//! INSTA_UPDATE=1 cargo test -p quon_na --test report_snapshots --no-default-features
//! ```
//! Compaction (#108) may change schedule numbers; refreshing `.snap` files is
//! intentional then.

use backend::NeutralAtomErrorModel;
use quon_na::{
    AodTrapRef, AtomId, AtomMove, BottleneckKind, CodeBlockId, CodeFamily, LogicalQubitId,
    MeasurementBasis, MovementGroup, NaScheduleOptions, NetRate, NeutralAtomAction,
    RESOURCE_REPORT_EVIDENCE_DISCLAIMER, RESOURCE_REPORT_EVIDENCE_KIND, ResourceReport,
    ScheduleLayer, SiteId, TransferDirection, TrapTransfer, build_resource_report,
    expand_code_block, resource_report_to_json, resource_report_to_markdown, run_from_qec_workload,
};
use quon_qec::{LogicalBasis, SourceFamily, WorkloadBuilder};
use serde_json::Value;

fn atom(id: u32) -> AtomId {
    AtomId(id)
}

fn site(id: u32) -> SiteId {
    SiteId(id)
}

fn aod() -> AodTrapRef {
    AodTrapRef {
        aod_id: 0,
        row: 1,
        col: 2,
    }
}

fn toy_move_entangle_measure_layers() -> Vec<ScheduleLayer> {
    vec![
        ScheduleLayer {
            cycle: 0,
            actions: vec![
                NeutralAtomAction::Move(MovementGroup {
                    duration_us: 10,
                    moves: vec![
                        AtomMove {
                            atom: atom(0),
                            from: site(0),
                            to: site(1),
                        },
                        AtomMove {
                            atom: atom(1),
                            from: site(2),
                            to: site(3),
                        },
                    ],
                }),
                NeutralAtomAction::Wait { duration_us: 4 },
            ],
        },
        ScheduleLayer {
            cycle: 1,
            actions: vec![
                NeutralAtomAction::Transfer(TrapTransfer {
                    atom: atom(0),
                    direction: TransferDirection::SlmToAod,
                    site: site(1),
                    aod: aod(),
                    duration_us: 6,
                }),
                NeutralAtomAction::Entangle2 {
                    atoms: [atom(0), atom(1)],
                    duration_us: 12,
                },
            ],
        },
        ScheduleLayer {
            cycle: 2,
            actions: vec![
                NeutralAtomAction::EntangleN {
                    atoms: vec![atom(0), atom(1), atom(2)],
                    duration_us: 8,
                },
                NeutralAtomAction::Measure {
                    atom: atom(0),
                    basis: MeasurementBasis::Z,
                    duration_us: 5,
                },
                NeutralAtomAction::Reset {
                    atom: atom(1),
                    duration_us: 7,
                },
            ],
        },
    ]
}

/// Keys that belong only to the Python/Sinter sampled CSV (ADR-0020).
const FORBIDDEN_SAMPLED_KEYS: &[&str] = &[
    "logical_failures",
    "logical_failure_rate",
    "shots",
    "sinter",
    "p_logical",
    "pL",
];

/// Positive threshold *claims* — anti-threshold disclaimer text may still say "threshold".
const FORBIDDEN_THRESHOLD_CLAIMS: &[&str] = &[
    "below the threshold",
    "below threshold",
    "above the threshold",
    "above threshold",
    "at the threshold",
    "at threshold",
];

const NOTES_ANALYTIC_BULLET: &str = "- Compiler analytic metrics only — not fused with Python/Sinter sampled CSV; neither artifact is a threshold claim (ADR-0020).";
const NOTES_BUDGET_BULLET: &str = "- Physical error budget lines are analytic schedule-count × rate contributions only — not sampled logical failure rates (Sinter) or threshold claims.";

fn assert_no_threshold_claims(text: &str) {
    let lower = text.to_ascii_lowercase();
    for claim in FORBIDDEN_THRESHOLD_CLAIMS {
        assert!(
            !lower.contains(claim),
            "must not make threshold claim `{claim}` (ADR-0020)"
        );
    }
}

fn assert_analytic_only_json(json: &str) {
    let value: Value = match serde_json::from_str(json) {
        Ok(v) => v,
        Err(e) => panic!("resource report JSON parse failed: {e}"),
    };
    let obj = match value.as_object() {
        Some(o) => o,
        None => panic!("resource report JSON must be an object"),
    };
    assert_eq!(
        obj.get("evidence_kind").and_then(|v| v.as_str()),
        Some(RESOURCE_REPORT_EVIDENCE_KIND),
        "JSON ResourceReport must label evidence_kind=analytic (ADR-0020)"
    );
    assert_eq!(
        obj.get("evidence_disclaimer").and_then(|v| v.as_str()),
        Some(RESOURCE_REPORT_EVIDENCE_DISCLAIMER),
        "JSON ResourceReport must carry the ADR-0020 evidence disclaimer"
    );
    for key in FORBIDDEN_SAMPLED_KEYS {
        assert!(
            !obj.contains_key(*key),
            "ResourceReport must not contain sampled/Sinter field `{key}` (ADR-0020)"
        );
    }
    let text = json.to_ascii_lowercase();
    assert!(
        !text.contains("logical_failure"),
        "ResourceReport JSON must not mention logical_failure* (ADR-0020)"
    );
    // Disclaimer may contain the word "threshold"; ban positive claims only.
    assert_no_threshold_claims(json);
}

fn assert_markdown_notes(md: &str, expect_budget_note: bool) {
    assert!(
        md.contains("# Neutral-atom analytic resource report"),
        "Markdown H1 must label the report as analytic"
    );
    assert!(
        md.contains(NOTES_ANALYTIC_BULLET),
        "Markdown must include the exact analytic Notes bullet (ADR-0020)"
    );
    if expect_budget_note {
        assert!(
            md.contains(NOTES_BUDGET_BULLET),
            "Markdown with error_budget must include the exact budget Notes bullet"
        );
    } else {
        assert!(
            !md.contains("Physical error budget lines"),
            "Markdown without error_budget must omit the budget Notes bullet"
        );
    }
    assert_no_threshold_claims(md);
    assert!(
        !md.contains("logical_failures"),
        "Markdown must not include Sinter logical_failures"
    );
}

fn assert_json_md(name: &str, report: &ResourceReport) {
    let json = match resource_report_to_json(report) {
        Ok(s) => s,
        Err(e) => panic!("json emit failed: {e}"),
    };
    assert_analytic_only_json(&json);
    let md = resource_report_to_markdown(report);
    assert_markdown_notes(&md, report.error_budget.is_some());
    assert!(
        md.contains("analytic"),
        "Markdown must label the report as analytic (ADR-0020)"
    );
    insta::assert_snapshot!(format!("{name}_json"), json);
    insta::assert_snapshot!(format!("{name}_md"), md);
}

#[test]
fn empty_schedule() {
    let report = ResourceReport::from_layers(&[]);
    assert_eq!(report.estimated_cycles, 0);
    assert_eq!(report.bottleneck, BottleneckKind::None);
    assert_json_md("empty_schedule", &report);
}

#[test]
fn toy_move_entangle_measure() {
    let layers = toy_move_entangle_measure_layers();
    let report = ResourceReport::from_layers(&layers);
    assert_eq!(report.estimated_cycles, 3);
    assert_eq!(report.bottleneck, BottleneckKind::Rearrangement);
    assert_json_md("toy_move_entangle_measure", &report);
}

#[test]
fn qec_repetition_d3() {
    let block = expand_code_block(
        CodeBlockId(0),
        CodeFamily::RepetitionCodeToy { distance: 3 },
        vec![LogicalQubitId(0)],
        0,
    );
    let block = match block {
        Ok(b) => b,
        Err(e) => panic!("expand: {e}"),
    };
    let report = match build_resource_report(&[], Some(&[block]), None) {
        Ok(r) => r,
        Err(e) => panic!("build: {e}"),
    };
    assert_eq!(report.logical_qubits, 1);
    assert_eq!(report.physical_atoms, 5);
    assert_json_md("qec_repetition_d3", &report);
}

#[test]
fn qec_repetition_d3_hybrid_schedule_report() {
    // Analytic hybrid QEC path (schedule + QEC metadata + error_budget).
    // Surface-code hybrid snapshot: deferred until #249 lands.
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../targets/neutral_atom/generic_rna_v0.json"
    ));
    let loaded = backend::json::load(path).expect("load target");
    let na = loaded.neutral_atom_target().expect("na target").clone();

    let mut b = WorkloadBuilder::new();
    b.construct(
        SourceFamily::Repetition,
        3,
        LogicalBasis::Z,
        LogicalQubitId(0),
    )
    .expect("construct");
    b.memory_round(LogicalQubitId(0)).expect("r1");
    b.memory_round(LogicalQubitId(0)).expect("r2");
    b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
        .expect("mz");
    let workload = b.finish();

    let artifacts = run_from_qec_workload(
        &workload,
        &na,
        NaScheduleOptions {
            compact: true,
            dump_ir: false,
            ..Default::default()
        },
    )
    .expect("schedule");
    let report = &artifacts.resource_report;

    // Pin analytic QEC fields required for the hybrid path (#246 / ADR-0020).
    assert_eq!(report.logical_qubits, 1);
    assert_eq!(report.physical_atoms, 5);
    assert_eq!(report.atoms_per_logical, Some(5));
    assert_eq!(report.distance, Some(3));
    assert_eq!(report.memory_rounds, Some(2));
    assert_eq!(report.code_family.as_deref(), Some("repetition_code_toy"));
    assert_eq!(report.evidence_kind, RESOURCE_REPORT_EVIDENCE_KIND);
    assert!(report.estimated_cycles > 0);
    assert_ne!(report.bottleneck, BottleneckKind::None);
    let budget = report
        .error_budget
        .expect("hybrid QEC report attaches analytic error_budget");
    assert!(budget.rydberg > 0.0);
    assert!(budget.measurement > 0.0);
    assert!(budget.reset > 0.0);
    assert!(budget.movement > 0.0);
    assert!(budget.transfer > 0.0);

    let md = resource_report_to_markdown(report);
    assert!(md.contains("| Distance | 3 |"));
    assert!(md.contains("| Memory rounds | 2 |"));
    assert!(md.contains("## Physical error budget"));
    assert!(md.contains("| Measurement | 0.009 |"));
    assert!(!md.contains("0.009000000000000001"));
    // Exact Notes bullets — not a vacuous "contains not && threshold".
    assert_markdown_notes(&md, true);

    let json = match resource_report_to_json(report) {
        Ok(s) => s,
        Err(e) => panic!("json: {e}"),
    };
    assert!(!json.contains("0.009000000000000001"));
    assert!(json.contains("\"measurement\": 0.009"));

    assert_json_md("qec_repetition_d3_hybrid", report);
}

#[test]
fn qec_surface_d3_hybrid_schedule_report() {
    // Analytic hybrid surface path (#249 / ADR-0020) — closes the #246 gap.
    let path = std::path::Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../targets/neutral_atom/generic_rna_v0.json"
    ));
    let loaded = backend::json::load(path).expect("load target");
    let na = loaded.neutral_atom_target().expect("na target").clone();

    let mut b = WorkloadBuilder::new();
    b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
        .expect("construct");
    b.memory_round(LogicalQubitId(0)).expect("r1");
    b.memory_round(LogicalQubitId(0)).expect("r2");
    b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
        .expect("mz");
    let workload = b.finish();

    let artifacts = run_from_qec_workload(
        &workload,
        &na,
        NaScheduleOptions {
            compact: true,
            dump_ir: false,
            ..Default::default()
        },
    )
    .expect("schedule");
    let report = &artifacts.resource_report;

    assert_eq!(report.logical_qubits, 1);
    assert_eq!(report.physical_atoms, 17);
    assert_eq!(report.atoms_per_logical, Some(17));
    assert_eq!(report.distance, Some(3));
    assert_eq!(report.memory_rounds, Some(2));
    assert_eq!(report.code_family.as_deref(), Some("surface_code_like"));
    assert_eq!(report.evidence_kind, RESOURCE_REPORT_EVIDENCE_KIND);
    assert_eq!(report.entangle2_count, 48);
    assert_eq!(report.measurement_rounds, 3);
    assert_eq!(report.reset_rounds, 2);
    assert!(report.estimated_cycles > 0);
    assert_ne!(report.bottleneck, BottleneckKind::None);
    let budget = report
        .error_budget
        .expect("hybrid QEC report attaches analytic error_budget");
    assert!(budget.rydberg > 0.0);
    assert!(budget.measurement > 0.0);
    assert!(budget.reset > 0.0);

    let md = resource_report_to_markdown(report);
    assert!(md.contains("| Distance | 3 |"));
    assert!(md.contains("| Memory rounds | 2 |"));
    assert!(md.contains("surface_code_like"));
    assert_markdown_notes(&md, true);

    assert_json_md("qec_surface_d3_hybrid", report);
}

#[test]
fn qec_qldpc_144_12_12_rate() {
    let logicals: Vec<_> = (0..12).map(LogicalQubitId).collect();
    let block = expand_code_block(
        CodeBlockId(0),
        CodeFamily::HighRateQldpcLike {
            net_rate: NetRate {
                numerator: 1,
                denominator: 24,
            },
        },
        logicals,
        0,
    );
    let block = match block {
        Ok(b) => b,
        Err(e) => panic!("expand: {e}"),
    };
    let layers = toy_move_entangle_measure_layers();
    let report = match build_resource_report(&layers, Some(&[block]), None) {
        Ok(r) => r,
        Err(e) => panic!("build: {e}"),
    };
    assert_eq!(report.logical_qubits, 12);
    assert_eq!(report.physical_atoms, 288);
    assert_eq!(report.atoms_per_logical, Some(24));
    assert_json_md("qec_qldpc_144_12_12_rate", &report);
}

#[test]
fn non_qec_physical_only() {
    let layers = toy_move_entangle_measure_layers();
    let report = match build_resource_report(&layers, None, Some(8)) {
        Ok(r) => r,
        Err(e) => panic!("build: {e}"),
    };
    assert_eq!(report.logical_qubits, 8);
    assert_eq!(report.physical_atoms, 8);
    assert_eq!(report.atoms_per_logical, None);
    assert_eq!(report.code_family, None);
    let md = resource_report_to_markdown(&report);
    assert!(!md.contains("Atoms per logical"));
    assert!(!md.contains("Code family"));
    assert!(!md.contains("N/A"));
    assert_json_md("non_qec_physical_only", &report);
}

#[test]
fn toy_with_error_budget() {
    let layers = toy_move_entangle_measure_layers();
    let model = NeutralAtomErrorModel {
        rydberg: 0.002,
        measurement: 0.003,
        reset: 0.004,
        movement: 0.0005,
        transfer: 0.0007,
        idle_per_us: 2e-9,
    };
    let report = ResourceReport::from_layers(&layers).with_error_budget(&model);
    assert!(report.error_budget.is_some());
    assert_json_md("toy_with_error_budget", &report);
}

#[test]
fn markdown_headline_matches_json_for_qldpc() {
    let logicals: Vec<_> = (0..12).map(LogicalQubitId).collect();
    let block = expand_code_block(
        CodeBlockId(0),
        CodeFamily::HighRateQldpcLike {
            net_rate: NetRate {
                numerator: 1,
                denominator: 24,
            },
        },
        logicals,
        0,
    );
    let block = match block {
        Ok(b) => b,
        Err(e) => panic!("expand: {e}"),
    };
    let report = match build_resource_report(&[], Some(&[block]), None) {
        Ok(r) => r,
        Err(e) => panic!("build: {e}"),
    };
    let json = match resource_report_to_json(&report) {
        Ok(s) => s,
        Err(e) => panic!("json: {e}"),
    };
    let md = resource_report_to_markdown(&report);
    assert!(json.contains("\"logical_qubits\": 12"));
    assert!(json.contains("\"physical_atoms\": 288"));
    assert!(json.contains("\"evidence_kind\": \"analytic\""));
    assert!(md.contains("| Logical qubits | 12 |"));
    assert!(md.contains("| Physical atoms | 288 |"));
    assert!(md.contains("| Atoms per logical | 24 |"));
    assert!(md.contains("# Neutral-atom analytic resource report"));
}
