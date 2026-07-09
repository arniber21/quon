//! Insta regression snapshots for `ResourceReport` JSON and Markdown emitters.
//!
//! Update goldens after intentional metric changes:
//! ```bash
//! INSTA_UPDATE=1 cargo test -p quon_na --test report_snapshots --no-default-features
//! ```
//! Compaction (#108) may change schedule numbers; refreshing `.snap` files is
//! intentional then.

use quon_na::{
    AodTrapRef, AtomId, AtomMove, BottleneckKind, CodeBlockId, CodeFamily, LogicalQubitId,
    MeasurementBasis, MovementGroup, NetRate, NeutralAtomAction, ResourceReport, ScheduleLayer,
    SiteId, TransferDirection, TrapTransfer, build_resource_report, expand_code_block,
    resource_report_to_json, resource_report_to_markdown,
};

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

fn assert_json_md(name: &str, report: &ResourceReport) {
    let json = match resource_report_to_json(report) {
        Ok(s) => s,
        Err(e) => panic!("json emit failed: {e}"),
    };
    let md = resource_report_to_markdown(report);
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
    assert!(md.contains("| Logical qubits | 12 |"));
    assert!(md.contains("| Physical atoms | 288 |"));
    assert!(md.contains("| Atoms per logical | 24 |"));
}
