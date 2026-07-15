//! Acceptance / unit tests for schedule compaction (#108).
//!
//! Lives under `tests/` (T1) so Taskless `no-unwrap-expect-in-src` stays clean
//! on `quon_na/src/compaction.rs`.

use quon_na::{
    AodTrapRef, AtomBinding, AtomId, AtomMove, AtomSite, CompactionError, CompactionOptions,
    CriticalPathReport, DEFAULT_GAMMA, GraphScheduleRequest, InteractionGraph, LegalityLimits,
    LogicalQubitId, MeasurementBasis, MovementGroup, NeutralAtomAction, NeutralAtomLayout,
    Position, ScheduleDependency, ScheduleDependencyKind, ScheduleLayer, SiteId, TransferDirection,
    TrapBinding, TrapTransfer, ZoneKind, asap_schedule_layers, compact_schedule,
    feed_forward_dependencies, force_merge_layers, infer_atom_dependencies, schedule_from_graph,
    toy_zoned_architecture,
};

/// Placeholder AOD ref emitted by #107 `schedule_zoned` (all zeros).
const PLACEHOLDER_AOD: AodTrapRef = AodTrapRef {
    aod_id: 0,
    row: 0,
    col: 0,
};

fn empty_req(n_qubits: u32) -> GraphScheduleRequest {
    let vertices: Vec<_> = (0..n_qubits).map(LogicalQubitId).collect();
    let graph = InteractionGraph::from_interactions(vertices, vec![], vec![], DEFAULT_GAMMA)
        .expect("empty graph");
    schedule_from_graph(graph).expect("stub")
}

fn entangle(a: u32, b: u32) -> NeutralAtomAction {
    NeutralAtomAction::Entangle2 {
        atoms: [AtomId(a), AtomId(b)],
        duration_us: 1,
    }
}

fn measure(a: u32) -> NeutralAtomAction {
    NeutralAtomAction::Measure {
        atom: AtomId(a),
        basis: MeasurementBasis::Z,
        duration_us: 1,
    }
}

fn layer(cycle: u32, actions: Vec<NeutralAtomAction>) -> ScheduleLayer {
    ScheduleLayer { cycle, actions }
}

/// Locked AC3 fixture (§6.4): Measure(q0); L2 Entangle(q2,q3); L3 Entangle(q2,q4).
fn ac3_fixture() -> GraphScheduleRequest {
    let mut req = empty_req(5);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(1, vec![measure(0)]),
        layer(2, vec![entangle(2, 3)]),
        layer(3, vec![entangle(2, 4)]),
    ];
    req
}

fn close_pair_layout() -> NeutralAtomLayout {
    NeutralAtomLayout {
        sites: vec![
            AtomSite {
                id: SiteId(0),
                position: Position {
                    x_um: 0.0,
                    y_um: 0.0,
                },
            },
            AtomSite {
                id: SiteId(1),
                position: Position {
                    x_um: 5.0,
                    y_um: 0.0,
                },
            },
            AtomSite {
                id: SiteId(2),
                position: Position {
                    x_um: 6.0,
                    y_um: 0.0,
                },
            },
            AtomSite {
                id: SiteId(3),
                position: Position {
                    x_um: 11.0,
                    y_um: 0.0,
                },
            },
        ],
        initial_bindings: (0..4)
            .map(|i| AtomBinding {
                atom: AtomId(i),
                trap: TrapBinding::Slm { site: SiteId(i) },
            })
            .collect(),
    }
}

fn far_pair_layout() -> NeutralAtomLayout {
    NeutralAtomLayout {
        sites: vec![
            AtomSite {
                id: SiteId(0),
                position: Position {
                    x_um: 0.0,
                    y_um: 0.0,
                },
            },
            AtomSite {
                id: SiteId(1),
                position: Position {
                    x_um: 5.0,
                    y_um: 0.0,
                },
            },
            AtomSite {
                id: SiteId(2),
                position: Position {
                    x_um: 40.0,
                    y_um: 0.0,
                },
            },
            AtomSite {
                id: SiteId(3),
                position: Position {
                    x_um: 45.0,
                    y_um: 0.0,
                },
            },
        ],
        initial_bindings: (0..4)
            .map(|i| AtomBinding {
                atom: AtomId(i),
                trap: TrapBinding::Slm { site: SiteId(i) },
            })
            .collect(),
    }
}

fn default_legality() -> LegalityLimits {
    LegalityLimits {
        rydberg_range_um: 7.5,
        min_rydberg_spacing_um: 18.75,
        aod_min_separation_um: 2.0,
    }
}

#[test]
fn empty_schedule_errors() {
    let req = empty_req(2);
    let err = asap_schedule_layers(req, &[]).unwrap_err();
    assert!(matches!(err, CompactionError::EmptySchedule));
}

#[test]
fn feed_forward_not_inferred() {
    let layers = vec![layer(0, vec![measure(0)]), layer(1, vec![entangle(2, 3)])];
    let deps = infer_atom_dependencies(&layers);
    assert!(
        deps.iter()
            .all(|d| d.kind != ScheduleDependencyKind::FeedForward)
    );
    assert!(deps.is_empty(), "disjoint atoms → no AtomHazard either");
}

#[test]
fn infer_atom_dependencies_shared_atom() {
    let layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(1, vec![entangle(1, 2)]),
    ];
    let deps = infer_atom_dependencies(&layers);
    assert_eq!(deps.len(), 1);
    assert_eq!(deps[0].before, 0);
    assert_eq!(deps[0].after, 1);
    assert_eq!(deps[0].kind, ScheduleDependencyKind::AtomHazard);
}

#[test]
fn asap_dependency_chain_matches_critical_path() {
    // Chain coincidence with lower bound — not an Enola-optimality claim.
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(1, vec![entangle(1, 2)]),
        layer(2, vec![entangle(2, 3)]),
    ];
    let result = asap_schedule_layers(req, &[]).expect("asap");
    assert_eq!(result.asap_makespan_cycles, 3);
    assert_eq!(result.critical_path.critical_path_length, 3);
    assert_eq!(result.request.layers.len(), 3);
    assert!(!result.compacted);
}

#[test]
fn asap_exclusive_cycle_serializes_independent() {
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(0, vec![entangle(2, 3)]),
    ];
    let result = asap_schedule_layers(req, &[]).expect("asap");
    assert_eq!(result.asap_makespan_cycles, 2);
    assert_eq!(result.critical_path.critical_path_length, 1);
    assert!(result.asap_makespan_cycles > result.critical_path.critical_path_length);
}

#[test]
fn asap_does_not_union_actions() {
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(0, vec![entangle(2, 3)]),
    ];
    let n = req.layers.len();
    let result = asap_schedule_layers(req, &[]).expect("asap");
    assert_eq!(result.request.layers.len(), n);
    for l in &result.request.layers {
        assert_eq!(l.actions.len(), 1);
    }
}

#[test]
fn greedy_reduces_vs_asap_e0() {
    // Default opts omit legality (B3 contract b — geometry unchecked).
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(0, vec![entangle(2, 3)]),
    ];
    let opts = CompactionOptions {
        greedy: true,
        ..Default::default()
    };
    let result = compact_schedule(req, &[], &opts).expect("compact");
    assert_eq!(result.asap_makespan_cycles, 2);
    assert_eq!(result.compacted_makespan_cycles, 1);
    assert!(result.compacted_makespan_cycles < result.asap_makespan_cycles);
    assert!(result.compacted);
    assert_eq!(result.request.layers.len(), 1);
    assert_eq!(result.request.layers[0].actions.len(), 2);
}

#[test]
fn measure_feed_forward_disjoint() {
    // AC3: FeedForward alone protects measure→correction (atoms disjoint from q0).
    let req = ac3_fixture();
    let ff = feed_forward_dependencies(1, &[3]);
    let opts = CompactionOptions {
        greedy: true,
        ..Default::default()
    };
    let result = compact_schedule(req, &ff, &opts).expect("compact");
    let c_meas = result
        .request
        .layers
        .iter()
        .find(|l| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Measure { atom, .. } if atom.0 == 0))
        })
        .map(|l| l.cycle)
        .expect("measure layer");
    let c_corr = result
        .request
        .layers
        .iter()
        .find(|l| {
            l.actions.iter().any(|a| {
                matches!(
                    a,
                    NeutralAtomAction::Entangle2 { atoms, .. }
                        if atoms[0] == AtomId(2) && atoms[1] == AtomId(4)
                )
            })
        })
        .map(|l| l.cycle)
        .expect("correction layer");
    assert!(c_meas < c_corr);
}

/// FeedForward side-merge: greedy must not pull the correction onto/before the
/// measure cycle by merging an early independent entangle with the correction.
///
/// Fixture: L0 Entangle(2,3)@0, L1 Measure(0)@1, L2 Entangle(4,5)@2 with
/// FeedForward(1→2). Without a post-merge cycle-order check, merging L0∥L2
/// would place the correction at cycle 0 while measure stays at cycle 1.
#[test]
fn feed_forward_side_merge_preserves_order() {
    let mut req = empty_req(6);
    req.layers = vec![
        layer(0, vec![entangle(2, 3)]),
        layer(1, vec![measure(0)]),
        layer(2, vec![entangle(4, 5)]),
    ];
    let ff = feed_forward_dependencies(1, &[2]);
    let opts = CompactionOptions {
        greedy: true,
        ..Default::default()
    };
    let result = compact_schedule(req, &ff, &opts).expect("compact");

    let cycle_of = |pred: &dyn Fn(&NeutralAtomAction) -> bool| -> u32 {
        result
            .request
            .layers
            .iter()
            .find(|l| l.actions.iter().any(pred))
            .map(|l| l.cycle)
            .expect("layer")
    };
    let c_meas = cycle_of(&|a| matches!(a, NeutralAtomAction::Measure { atom, .. } if atom.0 == 0));
    let c_corr = cycle_of(&|a| {
        matches!(
            a,
            NeutralAtomAction::Entangle2 { atoms, .. }
                if atoms[0] == AtomId(4) && atoms[1] == AtomId(5)
        )
    });
    assert!(
        c_meas < c_corr,
        "FeedForward side-merge must not put correction before measure (meas={c_meas}, corr={c_corr})"
    );
    // Forced L0∥L2 merge must HardFail with DependencyViolation.
    let mut req2 = empty_req(6);
    req2.layers = vec![
        layer(0, vec![entangle(2, 3)]),
        layer(1, vec![measure(0)]),
        layer(2, vec![entangle(4, 5)]),
    ];
    let err = force_merge_layers(req2, &ff, 0, 2, &opts).unwrap_err();
    assert!(matches!(err, CompactionError::DependencyViolation));
}

#[test]
fn cannot_merge_measure_with_correction() {
    let mut req = empty_req(5);
    req.layers = vec![layer(0, vec![measure(0)]), layer(1, vec![entangle(2, 4)])];
    let ff = feed_forward_dependencies(0, &[1]);
    let opts = CompactionOptions::default();
    let err = force_merge_layers(req, &ff, 0, 1, &opts).unwrap_err();
    assert!(matches!(err, CompactionError::DependencyViolation));
}

#[test]
fn ac3_l2_l3_blocked_by_atom_hazard() {
    // Locked fixture without FeedForward: L2∥L3 still blocked (shared q2).
    // Do NOT assert that L2∥L3 merge succeeds — AtomHazard / conflicts forbid it.
    let req = ac3_fixture();
    let inferred = infer_atom_dependencies(&req.layers);
    assert!(
        inferred.iter().any(|d| {
            d.before == 2 && d.after == 3 && d.kind == ScheduleDependencyKind::AtomHazard
        }),
        "L2 and L3 share q2 → AtomHazard must connect them"
    );
    assert!(
        inferred
            .iter()
            .all(|d| d.kind != ScheduleDependencyKind::FeedForward),
        "FeedForward must not be invented"
    );
    // Measure(q0) shares no atom with L3 Entangle(q2,q4) → no AtomHazard measure→correction.
    assert!(
        !inferred.iter().any(|d| d.before == 1 && d.after == 3),
        "without FeedForward, measure→correction is unprotected by AtomHazard"
    );

    let opts = CompactionOptions {
        greedy: true,
        ..Default::default()
    };
    let result = compact_schedule(req, &[], &opts).expect("compact");
    let cycle_of_pair = |a: u32, b: u32| -> Option<u32> {
        result.request.layers.iter().find_map(|l| {
            l.actions.iter().find_map(|action| match action {
                NeutralAtomAction::Entangle2 { atoms, .. }
                    if atoms[0] == AtomId(a) && atoms[1] == AtomId(b) =>
                {
                    Some(l.cycle)
                }
                _ => None,
            })
        })
    };
    // L2 (q2,q3) and L3 (q2,q4) must remain on distinct cycles.
    assert_ne!(
        cycle_of_pair(2, 3),
        cycle_of_pair(2, 4),
        "L2∥L3 blocked by shared q2 — not a FeedForward demo"
    );
}

#[test]
fn barrier_blocks_cross_merge() {
    let mut req = empty_req(6);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(1, vec![entangle(4, 5)]),
        layer(2, vec![entangle(2, 3)]),
    ];
    let deps = vec![ScheduleDependency {
        before: 0,
        after: 2,
        kind: ScheduleDependencyKind::Barrier,
    }];
    let opts = CompactionOptions {
        greedy: true,
        ..Default::default()
    };
    let result = compact_schedule(req, &deps, &opts).expect("compact");
    let cycle_of_pair = |a: u32, b: u32| -> Option<u32> {
        result.request.layers.iter().find_map(|l| {
            l.actions.iter().find_map(|action| match action {
                NeutralAtomAction::Entangle2 { atoms, .. }
                    if atoms[0] == AtomId(a) && atoms[1] == AtomId(b) =>
                {
                    Some(l.cycle)
                }
                _ => None,
            })
        })
    };
    assert_ne!(cycle_of_pair(0, 1), cycle_of_pair(2, 3));
}

/// Barrier pre∥post: an independent mid-barrier layer must not side-merge with
/// a post-barrier layer in a way that inverts `cycle(pre) < cycle(post)`.
///
/// L0 pre Entangle(0,1), L1 mid Entangle(2,3), L2 post Entangle(4,5), Barrier
/// cut at L1 (`before: 1` → expands to pre→post edges including 0→2). Merging
/// L0∥L2 would place post at the pre cycle.
#[test]
fn barrier_side_merge_preserves_pre_post_order() {
    let mut req = empty_req(6);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(1, vec![entangle(2, 3)]),
        layer(2, vec![entangle(4, 5)]),
    ];
    let deps = vec![ScheduleDependency {
        before: 1,
        after: 2,
        kind: ScheduleDependencyKind::Barrier,
    }];
    let opts = CompactionOptions {
        greedy: true,
        ..Default::default()
    };
    let result = compact_schedule(req.clone(), &deps, &opts).expect("compact");
    let cycle_of_pair = |layers: &[ScheduleLayer], a: u32, b: u32| -> u32 {
        layers
            .iter()
            .find_map(|l| {
                l.actions.iter().find_map(|action| match action {
                    NeutralAtomAction::Entangle2 { atoms, .. }
                        if atoms[0] == AtomId(a) && atoms[1] == AtomId(b) =>
                    {
                        Some(l.cycle)
                    }
                    _ => None,
                })
            })
            .expect("pair")
    };
    let c_pre = cycle_of_pair(&result.request.layers, 0, 1);
    let c_post = cycle_of_pair(&result.request.layers, 4, 5);
    assert!(
        c_pre < c_post,
        "Barrier side-merge must keep pre before post (pre={c_pre}, post={c_post})"
    );

    let err = force_merge_layers(req, &deps, 0, 2, &opts).unwrap_err();
    assert!(matches!(err, CompactionError::DependencyViolation));
}

#[test]
fn forbidden_merge_transfer_layers() {
    let mut req = empty_req(2);
    let xfer = |atom: u32, site: u32| {
        NeutralAtomAction::Transfer(TrapTransfer {
            atom: AtomId(atom),
            direction: TransferDirection::SlmToAod,
            site: SiteId(site),
            aod: PLACEHOLDER_AOD,
            duration_us: 15,
        })
    };
    req.layers = vec![layer(0, vec![xfer(0, 0)]), layer(1, vec![xfer(1, 1)])];
    let opts = CompactionOptions::default();
    let err = force_merge_layers(req, &[], 0, 1, &opts).unwrap_err();
    assert!(matches!(err, CompactionError::ForbiddenMergeClass(_)));
}

#[test]
fn merge_rejected_when_r2_r3_violated() {
    // layout + legality both set (contract b) → illegal geometry rejected.
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(0, vec![entangle(2, 3)]),
    ];
    req.layout = Some(close_pair_layout());
    let opts = CompactionOptions {
        greedy: true,
        legality: Some(default_legality()),
        ..Default::default()
    };
    let result = compact_schedule(req, &[], &opts).expect("compact");
    assert_eq!(result.compacted_makespan_cycles, 2);
    assert!(!result.compacted);
}

#[test]
fn r2_r3_skipped_without_legality() {
    // Same illegal geometry with layout but legality: None → merge may succeed
    // (documents B3 contract b: geometry unchecked without legality).
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(0, vec![entangle(2, 3)]),
    ];
    req.layout = Some(close_pair_layout());
    let opts = CompactionOptions {
        greedy: true,
        legality: None,
        ..Default::default()
    };
    let result = compact_schedule(req, &[], &opts).expect("compact");
    assert_eq!(result.compacted_makespan_cycles, 1);
    assert!(result.compacted);
}

#[test]
fn r2_r3_runs_without_mlir_feature() {
    // Compiles under --no-default-features; far-apart pairs merge under legality.
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(0, vec![entangle(2, 3)]),
    ];
    req.layout = Some(far_pair_layout());
    let opts = CompactionOptions {
        greedy: true,
        legality: Some(default_legality()),
        ..Default::default()
    };
    let result = compact_schedule(req, &[], &opts).expect("compact");
    assert_eq!(result.compacted_makespan_cycles, 1);
}

#[test]
fn critical_path_report_populated() {
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(1, vec![entangle(1, 2)]),
        layer(2, vec![entangle(2, 3)]),
    ];
    let result = asap_schedule_layers(req, &[]).expect("asap");
    assert_eq!(result.critical_path.makespan_cycles, 3);
    assert_eq!(result.critical_path.critical_path_length, 3);
    assert_eq!(result.critical_path.critical_layer_indices, vec![0, 1, 2]);
    let json = serde_json::to_string(&result.critical_path).expect("ser");
    let back: CriticalPathReport = serde_json::from_str(&json).expect("de");
    assert_eq!(back, result.critical_path);
}

#[test]
fn zoned_entangle_only_passthrough() {
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(0, vec![entangle(2, 3)]),
    ];
    let arch = toy_zoned_architecture();
    let ent = arch
        .zones
        .iter()
        .find(|z| z.kind == ZoneKind::Entanglement)
        .expect("ent zone");
    let y = ent.origin_um.1;
    let sites = vec![
        AtomSite {
            id: SiteId(0),
            position: Position { x_um: 0.0, y_um: y },
        },
        AtomSite {
            id: SiteId(1),
            position: Position { x_um: 2.0, y_um: y },
        },
        AtomSite {
            id: SiteId(2),
            position: Position {
                x_um: 40.0,
                y_um: y,
            },
        },
        AtomSite {
            id: SiteId(3),
            position: Position {
                x_um: 42.0,
                y_um: y,
            },
        },
    ];
    req.layout = Some(NeutralAtomLayout {
        sites,
        initial_bindings: (0..4)
            .map(|i| AtomBinding {
                atom: AtomId(i),
                trap: TrapBinding::Slm { site: SiteId(i) },
            })
            .collect(),
    });
    let opts = CompactionOptions {
        greedy: true,
        arch: Some(arch),
        legality: Some(default_legality()),
    };
    let result = compact_schedule(req, &[], &opts).expect("compact");
    assert_eq!(result.compacted_makespan_cycles, 1);
}

/// Greedy must not pull an entangle earlier than its Move AtomHazard by merging
/// it with an unrelated earlier E0 layer (occupancy/geometry bug at lower).
#[test]
fn greedy_merge_respects_atom_hazard_cycle_order() {
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(
            1,
            vec![NeutralAtomAction::Move(MovementGroup {
                moves: vec![AtomMove {
                    atom: AtomId(2),
                    from: SiteId(2),
                    to: SiteId(3),
                }],
                duration_us: 10,
            })],
        ),
        layer(2, vec![entangle(2, 3)]),
    ];
    let deps = infer_atom_dependencies(&req.layers);
    assert!(
        deps.iter().any(|d| {
            d.before == 1 && d.after == 2 && d.kind == ScheduleDependencyKind::AtomHazard
        }),
        "Move(atom2) must AtomHazard-precede Entangle(2,3)"
    );
    let opts = CompactionOptions {
        greedy: true,
        ..Default::default()
    };
    let result = compact_schedule(req, &deps, &opts).expect("compact");
    let e2_cycles: Vec<u32> = result
        .request
        .layers
        .iter()
        .filter(|l| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Entangle2 { .. }))
        })
        .map(|l| l.cycle)
        .collect();
    assert_eq!(
        e2_cycles.len(),
        2,
        "E2(0,1) must not merge with E2(2,3) across an intervening Move"
    );
    let move_cycle = result
        .request
        .layers
        .iter()
        .find(|l| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Move(_)))
        })
        .expect("move")
        .cycle;
    let e23_cycle = result
        .request
        .layers
        .iter()
        .find(|l| {
            l.actions.iter().any(|a| {
                matches!(
                    a,
                    NeutralAtomAction::Entangle2 { atoms, .. }
                        if atoms[0] == AtomId(2) || atoms[1] == AtomId(2)
                )
            })
        })
        .expect("e23")
        .cycle;
    assert!(
        move_cycle < e23_cycle,
        "Move cycle {move_cycle} must precede Entangle(2,3) cycle {e23_cycle}"
    );
}

/// Within-round atom-disjoint E0 still merges when no Wait/round cut separates them.
#[test]
fn within_round_disjoint_e0_merges() {
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(1, vec![entangle(2, 3)]),
    ];
    let opts = CompactionOptions {
        greedy: true,
        ..Default::default()
    };
    let result = compact_schedule(req, &[], &opts).expect("compact");
    assert!(result.compacted);
    assert_eq!(result.request.layers.len(), 1);
    assert_eq!(
        result.request.layers[0]
            .actions
            .iter()
            .filter(|a| matches!(a, NeutralAtomAction::Entangle2 { .. }))
            .count(),
        2
    );
}

/// Wait / round Barrier cuts are causal: without them, atom-disjoint E0 across a
/// Wait merges; with them, it does not. Deleting only the cuts must fail this test.
#[test]
fn wait_barrier_cuts_are_causal_for_cross_round_e0() {
    let mut req = empty_req(4);
    req.layers = vec![
        layer(0, vec![entangle(0, 1)]),
        layer(1, vec![NeutralAtomAction::Wait { duration_us: 1 }]),
        layer(2, vec![entangle(2, 3)]),
    ];
    let atom_deps = infer_atom_dependencies(&req.layers);
    assert!(
        atom_deps.is_empty(),
        "cross-Wait E2s are atom-disjoint → no AtomHazard"
    );
    let opts = CompactionOptions {
        greedy: true,
        ..Default::default()
    };

    let without = compact_schedule(req.clone(), &atom_deps, &opts).expect("without cuts");
    let without_e2_layers = without
        .request
        .layers
        .iter()
        .filter(|l| {
            l.actions
                .iter()
                .any(|a| matches!(a, NeutralAtomAction::Entangle2 { .. }))
        })
        .count();
    assert_eq!(
        without_e2_layers, 1,
        "without Wait Barrier cuts, atom-disjoint E2∥E2 across Wait must merge"
    );

    let cuts = quon_na::round_barrier_cuts(&req.layers);
    assert_eq!(cuts, vec![(1, 2)], "Wait → next Entangle cut");
    let mut with_deps = atom_deps;
    for &(before, after) in &cuts {
        with_deps.push(ScheduleDependency {
            before,
            after,
            kind: ScheduleDependencyKind::Barrier,
        });
    }
    let with = compact_schedule(req, &with_deps, &opts).expect("with cuts");
    let with_e2_same_cycle = with.request.layers.iter().any(|l| {
        l.actions
            .iter()
            .filter(|a| matches!(a, NeutralAtomAction::Entangle2 { .. }))
            .count()
            >= 2
    });
    assert!(
        !with_e2_same_cycle,
        "with Wait Barrier cuts, cross-round E2∥E2 must not share a cycle"
    );
    assert_eq!(
        with.request.layers.len(),
        3,
        "Wait cut must preserve three layers (no cross-round E0 merge)"
    );
}

#[test]
fn zone_reject_move_merge_without_simulator() {
    let mut req = empty_req(4);
    req.layers = vec![
        layer(
            0,
            vec![NeutralAtomAction::Move(MovementGroup {
                moves: vec![AtomMove {
                    atom: AtomId(0),
                    from: SiteId(0),
                    to: SiteId(1),
                }],
                duration_us: 10,
            })],
        ),
        layer(1, vec![entangle(2, 3)]),
    ];
    let arch = toy_zoned_architecture();
    req.layout = Some(far_pair_layout());
    let opts = CompactionOptions {
        arch: Some(arch),
        ..Default::default()
    };
    let err = force_merge_layers(req, &[], 0, 1, &opts).unwrap_err();
    assert!(matches!(err, CompactionError::ForbiddenMergeClass(_)));
}
