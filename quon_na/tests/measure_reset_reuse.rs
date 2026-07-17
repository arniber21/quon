//! Issue #282 tests: TemporalAtomMetrics and QEC reuse-vs-no-reuse comparison.
//!
//! These tests cover acceptance criteria #3 and #4:
//! - #3: Resource reports distinguish peak atoms, allocated atoms over time,
//!   reset count, measurement count, and reused ancilla count.
//! - #4: At least one QEC example reuses ancilla regions across rounds and
//!   emits a reduced peak-atom report compared with a no-reuse variant.

use quon_na::{
    AtomId, MeasurementBasis, NeutralAtomAction, ReuseRegionId, ScheduleLayer, TemporalAtomMetrics,
};

fn atom(id: u32) -> AtomId {
    AtomId(id)
}

// ---------------------------------------------------------------------------
// Criterion #3: TemporalAtomMetrics field coverage
// ---------------------------------------------------------------------------

#[test]
fn temporal_metrics_empty_schedule() {
    let metrics = TemporalAtomMetrics::from_layers(&[]);
    assert_eq!(metrics.peak_atoms, 0);
    assert!(metrics.allocated_atoms_series.is_empty());
    assert_eq!(metrics.measurement_count, 0);
    assert_eq!(metrics.reset_count, 0);
    assert_eq!(metrics.reuse_count, 0);
    assert_eq!(metrics.reused_ancilla_count, 0);
}

#[test]
fn temporal_metrics_counts_measure_reset_reuse_ops() {
    let layers = vec![
        ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::Measure {
                atom: atom(0),
                basis: MeasurementBasis::Z,
                duration_us: 10,
            }],
        },
        ScheduleLayer {
            cycle: 1,
            actions: vec![NeutralAtomAction::Reset {
                atom: atom(0),
                duration_us: 10,
            }],
        },
        ScheduleLayer {
            cycle: 2,
            actions: vec![NeutralAtomAction::Reuse {
                atom: atom(0),
                region: Some(ReuseRegionId(0)),
                duration_us: 5,
            }],
        },
    ];
    let metrics = TemporalAtomMetrics::from_layers(&layers);
    assert_eq!(metrics.measurement_count, 1);
    assert_eq!(metrics.reset_count, 1);
    assert_eq!(metrics.reuse_count, 1);
    assert_eq!(metrics.reused_ancilla_count, 1);
}

#[test]
fn temporal_metrics_peak_atoms_tracks_distinct_atoms() {
    // Three distinct atoms across layers → peak = 3.
    let layers = vec![
        ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [atom(0), atom(1)],
                duration_us: 1,
            }],
        },
        ScheduleLayer {
            cycle: 1,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [atom(1), atom(2)],
                duration_us: 1,
            }],
        },
    ];
    let metrics = TemporalAtomMetrics::from_layers(&layers);
    assert_eq!(metrics.peak_atoms, 3);
    assert_eq!(metrics.allocated_atoms_series, vec![2, 3]);
}

#[test]
fn temporal_metrics_allocated_series_is_monotonic() {
    // Atoms accumulate: [1, 2, 2, 3].
    let layers = vec![
        ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::Measure {
                atom: atom(0),
                basis: MeasurementBasis::Z,
                duration_us: 10,
            }],
        },
        ScheduleLayer {
            cycle: 1,
            actions: vec![NeutralAtomAction::Reset {
                atom: atom(1),
                duration_us: 10,
            }],
        },
        ScheduleLayer {
            cycle: 2,
            actions: vec![NeutralAtomAction::Wait { duration_us: 1 }],
        },
        ScheduleLayer {
            cycle: 3,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [atom(0), atom(2)],
                duration_us: 1,
            }],
        },
    ];
    let metrics = TemporalAtomMetrics::from_layers(&layers);
    assert_eq!(metrics.allocated_atoms_series, vec![1, 2, 2, 3]);
    assert_eq!(metrics.peak_atoms, 3);
}

#[test]
fn temporal_metrics_reused_ancilla_counts_distinct_atoms() {
    // Two reuse events on the same atom → reused_ancilla_count = 1.
    // (This is the within-one-schedule count; the atom is reused once but
    // only counted as one distinct reused ancilla.)
    let layers = vec![
        ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::Measure {
                atom: atom(0),
                basis: MeasurementBasis::Z,
                duration_us: 10,
            }],
        },
        ScheduleLayer {
            cycle: 1,
            actions: vec![NeutralAtomAction::Reset {
                atom: atom(0),
                duration_us: 10,
            }],
        },
        ScheduleLayer {
            cycle: 2,
            actions: vec![NeutralAtomAction::Reuse {
                atom: atom(0),
                region: Some(ReuseRegionId(0)),
                duration_us: 5,
            }],
        },
        ScheduleLayer {
            cycle: 3,
            actions: vec![NeutralAtomAction::Measure {
                atom: atom(0),
                basis: MeasurementBasis::Z,
                duration_us: 10,
            }],
        },
        ScheduleLayer {
            cycle: 4,
            actions: vec![NeutralAtomAction::Reset {
                atom: atom(0),
                duration_us: 10,
            }],
        },
        ScheduleLayer {
            cycle: 5,
            actions: vec![NeutralAtomAction::Reuse {
                atom: atom(0),
                region: Some(ReuseRegionId(1)),
                duration_us: 5,
            }],
        },
    ];
    let metrics = TemporalAtomMetrics::from_layers(&layers);
    assert_eq!(metrics.reuse_count, 2);
    assert_eq!(metrics.reused_ancilla_count, 1);
}

#[test]
fn temporal_metrics_wait_does_not_allocate_atoms() {
    let layers = vec![ScheduleLayer {
        cycle: 0,
        actions: vec![NeutralAtomAction::Wait { duration_us: 100 }],
    }];
    let metrics = TemporalAtomMetrics::from_layers(&layers);
    assert_eq!(metrics.peak_atoms, 0);
    assert_eq!(metrics.allocated_atoms_series, vec![0]);
}

// ---------------------------------------------------------------------------
// Criterion #4: QEC reuse vs no-reuse — reduced peak atoms
// ---------------------------------------------------------------------------

/// Build a **no-reuse** QEC schedule: 2 rounds, each round allocates a fresh
/// ancilla (atoms 1, 2) for syndrome extraction. The data qubit (atom 0)
/// persists across rounds. Peak atoms = 3 (data + 2 distinct ancillae).
fn qec_no_reuse_layers() -> Vec<ScheduleLayer> {
    vec![
        // Round 1: entangle data (0) with ancilla (1).
        ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [atom(0), atom(1)],
                duration_us: 1,
            }],
        },
        // Measure ancilla 1.
        ScheduleLayer {
            cycle: 1,
            actions: vec![NeutralAtomAction::Measure {
                atom: atom(1),
                basis: MeasurementBasis::Z,
                duration_us: 10,
            }],
        },
        // Round 2: entangle data (0) with a FRESH ancilla (2).
        ScheduleLayer {
            cycle: 2,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [atom(0), atom(2)],
                duration_us: 1,
            }],
        },
        // Measure ancilla 2.
        ScheduleLayer {
            cycle: 3,
            actions: vec![NeutralAtomAction::Measure {
                atom: atom(2),
                basis: MeasurementBasis::Z,
                duration_us: 10,
            }],
        },
    ]
}

/// Build a **reuse** QEC schedule: 2 rounds, but the same ancilla (atom 1) is
/// measured, reset, and reused in round 2. Peak atoms = 2 (data + 1 reused
/// ancilla).
fn qec_reuse_layers() -> Vec<ScheduleLayer> {
    vec![
        // Round 1: entangle data (0) with ancilla (1).
        ScheduleLayer {
            cycle: 0,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [atom(0), atom(1)],
                duration_us: 1,
            }],
        },
        // Measure ancilla 1.
        ScheduleLayer {
            cycle: 1,
            actions: vec![NeutralAtomAction::Measure {
                atom: atom(1),
                basis: MeasurementBasis::Z,
                duration_us: 10,
            }],
        },
        // Reset ancilla 1.
        ScheduleLayer {
            cycle: 2,
            actions: vec![NeutralAtomAction::Reset {
                atom: atom(1),
                duration_us: 10,
            }],
        },
        // Reuse ancilla 1 (reclaim into region 0).
        ScheduleLayer {
            cycle: 3,
            actions: vec![NeutralAtomAction::Reuse {
                atom: atom(1),
                region: Some(ReuseRegionId(0)),
                duration_us: 5,
            }],
        },
        // Round 2: entangle data (0) with the SAME ancilla (1).
        ScheduleLayer {
            cycle: 4,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [atom(0), atom(1)],
                duration_us: 1,
            }],
        },
        // Measure ancilla 1 again.
        ScheduleLayer {
            cycle: 5,
            actions: vec![NeutralAtomAction::Measure {
                atom: atom(1),
                basis: MeasurementBasis::Z,
                duration_us: 10,
            }],
        },
    ]
}

#[test]
fn qec_reuse_reduces_peak_atoms_vs_no_reuse() {
    let no_reuse = TemporalAtomMetrics::from_layers(&qec_no_reuse_layers());
    let reuse = TemporalAtomMetrics::from_layers(&qec_reuse_layers());

    // The reuse variant must have strictly fewer peak atoms.
    assert!(
        reuse.peak_atoms < no_reuse.peak_atoms,
        "reuse peak ({}) should be < no-reuse peak ({})",
        reuse.peak_atoms,
        no_reuse.peak_atoms
    );

    // Concrete values: no-reuse = 3 (data + 2 ancillae), reuse = 2 (data + 1).
    assert_eq!(no_reuse.peak_atoms, 3);
    assert_eq!(reuse.peak_atoms, 2);

    // Reuse variant has explicit reuse events and a reused ancilla.
    assert_eq!(reuse.reuse_count, 1);
    assert_eq!(reuse.reused_ancilla_count, 1);
    assert_eq!(no_reuse.reuse_count, 0);
    assert_eq!(no_reuse.reused_ancilla_count, 0);

    // Both have the same number of measurement ops.
    assert_eq!(no_reuse.measurement_count, 2);
    assert_eq!(reuse.measurement_count, 2);

    // Reuse variant has a reset that the no-reuse variant lacks.
    assert_eq!(reuse.reset_count, 1);
    assert_eq!(no_reuse.reset_count, 0);
}

#[test]
fn qec_reuse_allocated_series_grows_slower() {
    let no_reuse = TemporalAtomMetrics::from_layers(&qec_no_reuse_layers());
    let reuse = TemporalAtomMetrics::from_layers(&qec_reuse_layers());

    // No-reuse series: [2, 2, 3, 3] — grows to 3 when fresh ancilla 2 appears.
    assert_eq!(no_reuse.allocated_atoms_series, vec![2, 2, 3, 3]);

    // Reuse series: [2, 2, 2, 2, 2, 2] — never exceeds 2 because ancilla 1 is
    // recycled rather than allocating a new atom.
    assert_eq!(reuse.allocated_atoms_series, vec![2, 2, 2, 2, 2, 2]);
}

#[test]
fn resource_report_carries_temporal_metrics() {
    // Verify the full ResourceReport pipeline carries temporal metrics.
    let report = quon_na::ResourceReport::from_layers(&qec_reuse_layers());
    let metrics = &report.temporal_atom_metrics;
    assert_eq!(metrics.peak_atoms, 2);
    assert_eq!(metrics.measurement_count, 2);
    assert_eq!(metrics.reset_count, 1);
    assert_eq!(metrics.reuse_count, 1);
    assert_eq!(metrics.reused_ancilla_count, 1);
}
