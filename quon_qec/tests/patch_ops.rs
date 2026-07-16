//! Integration tests for the patch-operation planner (#281).
//!
//! These tests verify that the planner lowers correctly to physical rounds
//! and that the existing CX behavior is preserved through the new path.

use quon_qec::{
    LogicalBasis, LogicalQubitId, QecWorkload, SourceFamily, WorkloadBuilder, expand_workload,
    patch_ops::{
        PatchBoundary, PatchEdge, PatchKind, PatchOperation, PatchPlan, lower_patch_plan,
        plan_logical_cx, plan_measure_logical, plan_rough_merge_split, validate_plan,
    },
};

/// Build a surface d=3 CX workload (control + target + memory + CX + measure).
fn surface_d3_cx_workload() -> QecWorkload {
    let mut b = WorkloadBuilder::new();
    b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
        .unwrap();
    b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(1))
        .unwrap();
    b.memory_round(LogicalQubitId(0)).unwrap();
    b.memory_round(LogicalQubitId(1)).unwrap();
    b.logical_cx(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
    b.measure_logical(LogicalQubitId(0), LogicalBasis::Z)
        .unwrap();
    b.measure_logical(LogicalQubitId(1), LogicalBasis::Z)
        .unwrap();
    b.finish()
}

#[test]
fn cx_lowers_through_patch_planner_preserving_phases() {
    let workload = surface_d3_cx_workload();
    let expanded = expand_workload(&workload).expect("expand");

    // Must have all the merge/split/measure/frame phases
    use quon_qec::expand::{MergeBoundary, RoundKind};
    let kinds: Vec<RoundKind> = expanded.rounds.iter().map(|r| r.kind).collect();
    assert!(
        kinds.contains(&RoundKind::Merge(MergeBoundary::Rough)),
        "missing rough merge"
    );
    assert!(
        kinds.contains(&RoundKind::Merge(MergeBoundary::Smooth)),
        "missing smooth merge"
    );
    assert!(
        kinds.contains(&RoundKind::Split(MergeBoundary::Rough)),
        "missing rough split"
    );
    assert!(
        kinds.contains(&RoundKind::Split(MergeBoundary::Smooth)),
        "missing smooth split"
    );
    assert!(kinds.contains(&RoundKind::MeasureAncilla));
    assert!(kinds.contains(&RoundKind::FrameUpdate));
}

#[test]
fn cx_lowers_with_correct_l_shape_geometry() {
    let workload = surface_d3_cx_workload();
    let expanded = expand_workload(&workload).expect("expand");

    // 3 blocks: control + target + ancilla
    assert_eq!(expanded.blocks.len(), 3);

    let c = &expanded.blocks[0];
    let t = &expanded.blocks[1];
    let a = expanded
        .blocks
        .iter()
        .find(|b| b.logical_id == LogicalQubitId(2))
        .expect("ancilla");

    let c_max_x = c.coords.iter().map(|(x, _)| *x).max().unwrap();
    let a_min_x = a
        .coords
        .iter()
        .take(a.data_atoms.len())
        .map(|(x, _)| *x)
        .min()
        .unwrap();
    assert!(
        c_max_x < a_min_x,
        "control|ancilla L/R seam: {c_max_x} < {a_min_x}"
    );

    let a_max_y = a
        .coords
        .iter()
        .take(a.data_atoms.len())
        .map(|(_, y)| *y)
        .max()
        .unwrap();
    let t_min_y = t.coords.iter().map(|(_, y)| *y).min().unwrap();
    assert!(
        a_max_y < t_min_y,
        "ancilla/target top/bottom seam: {a_max_y} < {t_min_y}"
    );
}

#[test]
fn measure_logical_lowers_through_planner() {
    let mut b = WorkloadBuilder::new();
    b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
        .unwrap();
    b.memory_round(LogicalQubitId(0)).unwrap();
    b.measure_logical(LogicalQubitId(0), LogicalBasis::X)
        .unwrap();
    let workload = b.finish();
    let expanded = expand_workload(&workload).expect("expand");

    let measure = expanded
        .rounds
        .iter()
        .find(|r| r.kind == quon_qec::expand::RoundKind::MeasureLogical)
        .expect("measure logical round");
    // d=3 surface code has 9 data atoms
    assert_eq!(measure.terminal.len(), 9);
    // All terminals should be X-basis measurements
    for t in &measure.terminal {
        match t {
            quon_qec::expand::RoundTerminal::Measure { basis, .. } => {
                assert_eq!(*basis, LogicalBasis::X);
            }
            other => panic!("expected Measure terminal, got {other:?}"),
        }
    }
}

#[test]
fn two_cx_examples_different_patch_ids() {
    // Example 1: CX(0, 1)
    let mut b1 = WorkloadBuilder::new();
    b1.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
        .unwrap();
    b1.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(1))
        .unwrap();
    b1.logical_cx(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
    let e1 = expand_workload(&b1.finish()).expect("expand1");

    // Example 2: CX(3, 4) — different patches, same planner
    let mut b2 = WorkloadBuilder::new();
    b2.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(3))
        .unwrap();
    b2.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(4))
        .unwrap();
    b2.logical_cx(LogicalQubitId(3), LogicalQubitId(4)).unwrap();
    let e2 = expand_workload(&b2.finish()).expect("expand2");

    // Both have the same number of blocks (3: control + target + ancilla)
    assert_eq!(e1.blocks.len(), e2.blocks.len());
    // Same number of rounds
    assert_eq!(e1.rounds.len(), e2.rounds.len());
    // Same round kinds
    let k1: Vec<_> = e1.rounds.iter().map(|r| r.kind).collect();
    let k2: Vec<_> = e2.rounds.iter().map(|r| r.kind).collect();
    assert_eq!(k1, k2);
}

#[test]
fn bare_merge_split_lowers_without_cx_phases() {
    let plan = plan_rough_merge_split(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
    validate_plan(&plan).expect("valid");

    let mut layouts;
    let mut next_atom;

    // Need to set up the patches first
    let mut b = WorkloadBuilder::new();
    b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(0))
        .unwrap();
    b.construct(SourceFamily::Surface, 3, LogicalBasis::Z, LogicalQubitId(1))
        .unwrap();
    let workload = b.finish();

    // Expand just the blocks (no ops)
    let block_workload = QecWorkload {
        blocks: workload.blocks.clone(),
        ops: vec![],
    };
    let expanded = expand_workload(&block_workload).expect("expand blocks");
    let max_atom = expanded
        .blocks
        .iter()
        .map(|b| b.atoms.iter().map(|a| a.0 + 1).max().unwrap_or(0))
        .max()
        .unwrap_or(0);
    layouts = expanded.blocks;
    next_atom = max_atom;

    let mut rounds = Vec::new();
    lower_patch_plan(&plan, &mut layouts, &mut next_atom, &mut rounds).expect("lower");

    use quon_qec::expand::{MergeBoundary, RoundKind};
    let kinds: Vec<RoundKind> = rounds.iter().map(|r| r.kind).collect();
    assert!(kinds.contains(&RoundKind::Merge(MergeBoundary::Rough)));
    assert!(kinds.contains(&RoundKind::Split(MergeBoundary::Rough)));
    // Bare merge/split does NOT have CX-only phases
    assert!(!kinds.contains(&RoundKind::MeasureAncilla));
    assert!(!kinds.contains(&RoundKind::FrameUpdate));
}

#[test]
fn invalid_plan_rejects_boundary_mismatch() {
    let mut plan = PatchPlan::new();
    plan.patches.push(quon_qec::patch_ops::Patch {
        logical_id: LogicalQubitId(0),
        kind: PatchKind::Data,
    });
    plan.patches.push(quon_qec::patch_ops::Patch {
        logical_id: LogicalQubitId(1),
        kind: PatchKind::Data,
    });
    plan.push(PatchOperation::Merge {
        primary: LogicalQubitId(0),
        partner: LogicalQubitId(1),
        boundary: PatchBoundary::Rough,
        primary_edge: PatchEdge::Left, // rough
        partner_edge: PatchEdge::Top,  // smooth — mismatch!
    });
    let err = validate_plan(&plan).unwrap_err();
    assert!(matches!(
        err,
        quon_qec::patch_ops::PlanError::BoundaryMismatch { .. }
    ));
}

#[test]
fn invalid_plan_rejects_unknown_patch() {
    let mut plan = PatchPlan::new();
    plan.patches.push(quon_qec::patch_ops::Patch {
        logical_id: LogicalQubitId(0),
        kind: PatchKind::Data,
    });
    plan.push(PatchOperation::MeasurePatch {
        logical_id: LogicalQubitId(99),
        basis: LogicalBasis::Z,
    });
    let err = validate_plan(&plan).unwrap_err();
    assert!(matches!(
        err,
        quon_qec::patch_ops::PlanError::UnknownPatch(99)
    ));
}

#[test]
fn cx_plan_produces_frame_updates_with_condition_atoms() {
    let workload = surface_d3_cx_workload();
    let expanded = expand_workload(&workload).expect("expand");

    let frame = expanded
        .rounds
        .iter()
        .find(|r| r.kind == quon_qec::expand::RoundKind::FrameUpdate)
        .expect("frame update");
    assert!(!frame.frame_updates.is_empty());
    for upd in &frame.frame_updates {
        assert!(
            !upd.condition_atoms.is_empty(),
            "byproduct must be outcome-conditioned: {upd:?}"
        );
    }
}

#[test]
fn cx_plan_ancilla_measurement_is_top_row() {
    let workload = surface_d3_cx_workload();
    let expanded = expand_workload(&workload).expect("expand");

    let mz_anc = expanded
        .rounds
        .iter()
        .find(|r| r.kind == quon_qec::expand::RoundKind::MeasureAncilla)
        .expect("ancilla mz");
    // d=3 → top row has 3 data atoms
    assert_eq!(
        mz_anc.terminal.len(),
        3,
        "ancilla logical Z is top-row (d atoms), not all data"
    );
}

#[test]
fn cx_plan_seam_atoms_attached_to_ancilla_block() {
    let workload = surface_d3_cx_workload();
    let expanded = expand_workload(&workload).expect("expand");

    let ancilla = expanded
        .blocks
        .iter()
        .find(|b| b.logical_id == LogicalQubitId(2))
        .expect("ancilla");
    // d=3 surface code: 17 data/check + 2*3 seam atoms = 23
    assert_eq!(ancilla.atoms.len(), 23);
    // Seam checks must carry StabilizerDefs
    let orphan_checks: Vec<_> = ancilla
        .check_atoms
        .iter()
        .filter(|c| !ancilla.stabilizers.iter().any(|s| s.check == **c))
        .collect();
    assert!(
        orphan_checks.is_empty(),
        "orphan seam checks without StabilizerDef: {orphan_checks:?}"
    );
}

#[test]
fn plan_and_validate_round_trip() {
    // All three planners produce validatable plans
    let cx = plan_logical_cx(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
    validate_plan(&cx).expect("cx valid");

    let measure = plan_measure_logical(LogicalQubitId(0), LogicalBasis::Z).unwrap();
    validate_plan(&measure).expect("measure valid");

    let merge_split = plan_rough_merge_split(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
    validate_plan(&merge_split).expect("merge/split valid");
}

#[test]
fn validate_rejects_split_before_merge() {
    let mut plan = PatchPlan::new();
    plan.patches.push(quon_qec::patch_ops::Patch {
        logical_id: LogicalQubitId(0),
        kind: PatchKind::Data,
    });
    plan.patches.push(quon_qec::patch_ops::Patch {
        logical_id: LogicalQubitId(1),
        kind: PatchKind::Data,
    });
    plan.push(PatchOperation::Split {
        primary: LogicalQubitId(0),
        partner: Some(LogicalQubitId(1)),
        boundary: PatchBoundary::Rough,
    });
    let err = validate_plan(&plan).unwrap_err();
    assert!(matches!(
        err,
        quon_qec::patch_ops::PlanError::OrderingViolation { .. }
    ));
}

#[test]
fn validate_rejects_merge_with_unprepared_ancilla() {
    let mut plan = PatchPlan::new();
    plan.patches.push(quon_qec::patch_ops::Patch {
        logical_id: LogicalQubitId(0),
        kind: PatchKind::Data,
    });
    plan.patches.push(quon_qec::patch_ops::Patch {
        logical_id: LogicalQubitId(1),
        kind: PatchKind::Ancilla,
    });
    // Merge without PrepareAncilla first
    plan.push(PatchOperation::Merge {
        primary: LogicalQubitId(0),
        partner: LogicalQubitId(1),
        boundary: PatchBoundary::Rough,
        primary_edge: PatchEdge::Right,
        partner_edge: PatchEdge::Left,
    });
    let err = validate_plan(&plan).unwrap_err();
    assert!(matches!(
        err,
        quon_qec::patch_ops::PlanError::OrderingViolation { .. }
    ));
}

#[test]
fn lower_rejects_unsupported_edge_combination() {
    let plan = plan_rough_merge_split(LogicalQubitId(0), LogicalQubitId(1)).unwrap();
    // Manually create a plan with unsupported edges
    let mut bad_plan = PatchPlan::new();
    bad_plan.patches.push(quon_qec::patch_ops::Patch {
        logical_id: LogicalQubitId(0),
        kind: PatchKind::Data,
    });
    bad_plan.patches.push(quon_qec::patch_ops::Patch {
        logical_id: LogicalQubitId(1),
        kind: PatchKind::Data,
    });
    bad_plan.push(PatchOperation::Merge {
        primary: LogicalQubitId(0),
        partner: LogicalQubitId(1),
        boundary: PatchBoundary::Rough,
        primary_edge: PatchEdge::Top,  // smooth edge
        partner_edge: PatchEdge::Left, // rough edge — mismatch
    });
    // This should be caught by validate_plan's boundary check
    let err = validate_plan(&bad_plan).unwrap_err();
    assert!(matches!(
        err,
        quon_qec::patch_ops::PlanError::BoundaryMismatch { .. }
    ));
}
