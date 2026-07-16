//! Issue #282 demo: QEC ancilla reuse across rounds with reduced peak atoms.
//!
//! This example builds two simplified surface-code-like syndrome-extraction
//! schedules — one that allocates a **fresh** ancilla each round (no reuse)
//! and one that **measures → resets → reuses** the same ancilla across rounds
//! — and prints the [`TemporalAtomMetrics`] comparison.
//!
//! The reuse variant has strictly fewer peak atoms because the ancilla is
//! recycled rather than consuming a new physical trap.
//!
//! Run with:
//! ```sh
//! cargo run -p quon_na --example qec_ancilla_reuse
//! ```

use quon_na::{
    AtomId, MeasurementBasis, NeutralAtomAction, ResourceReport, ReuseRegionId, ScheduleLayer,
};

const DATA_QUBIT: u32 = 0;
const ROUNDS: u32 = 3;

/// No-reuse schedule: each round allocates a fresh ancilla.
/// Atoms: data (0) + ancillae (1, 2, 3, …) → peak = 1 + ROUNDS.
fn no_reuse_schedule(rounds: u32) -> Vec<ScheduleLayer> {
    let mut layers = Vec::new();
    let mut cycle = 0u32;
    for r in 0..rounds {
        let ancilla = DATA_QUBIT + 1 + r;
        layers.push(ScheduleLayer {
            cycle,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [AtomId(DATA_QUBIT), AtomId(ancilla)],
                duration_us: 1,
            }],
        });
        cycle += 1;
        layers.push(ScheduleLayer {
            cycle,
            actions: vec![NeutralAtomAction::Measure {
                atom: AtomId(ancilla),
                basis: MeasurementBasis::Z,
                duration_us: 10,
            }],
        });
        cycle += 1;
    }
    layers
}

/// Reuse schedule: the same ancilla (atom 1) is measured, reset, and reused
/// each round. Atoms: data (0) + ancilla (1) → peak = 2 regardless of rounds.
fn reuse_schedule(rounds: u32) -> Vec<ScheduleLayer> {
    let ancilla = DATA_QUBIT + 1;
    let mut layers = Vec::new();
    let mut cycle = 0u32;
    for r in 0..rounds {
        layers.push(ScheduleLayer {
            cycle,
            actions: vec![NeutralAtomAction::Entangle2 {
                atoms: [AtomId(DATA_QUBIT), AtomId(ancilla)],
                duration_us: 1,
            }],
        });
        cycle += 1;
        layers.push(ScheduleLayer {
            cycle,
            actions: vec![NeutralAtomAction::Measure {
                atom: AtomId(ancilla),
                basis: MeasurementBasis::Z,
                duration_us: 10,
            }],
        });
        cycle += 1;
        // Reset + reuse after the first round (round 0 has nothing to reclaim).
        if r > 0 {
            layers.push(ScheduleLayer {
                cycle,
                actions: vec![NeutralAtomAction::Reset {
                    atom: AtomId(ancilla),
                    duration_us: 10,
                }],
            });
            cycle += 1;
            layers.push(ScheduleLayer {
                cycle,
                actions: vec![NeutralAtomAction::Reuse {
                    atom: AtomId(ancilla),
                    region: Some(ReuseRegionId(0)),
                    duration_us: 5,
                }],
            });
            cycle += 1;
        }
    }
    layers
}

fn main() {
    let no_reuse_layers = no_reuse_schedule(ROUNDS);
    let reuse_layers = reuse_schedule(ROUNDS);

    let no_reuse_report = ResourceReport::from_layers(&no_reuse_layers);
    let reuse_report = ResourceReport::from_layers(&reuse_layers);

    let nr = &no_reuse_report.temporal_atom_metrics;
    let re = &reuse_report.temporal_atom_metrics;

    println!(
        "=== QEC Ancilla Reuse vs No-Reuse ({} rounds) ===\n",
        ROUNDS
    );
    println!("{:<30} {:>12} {:>12}", "Metric", "No-Reuse", "Reuse");
    println!("{:-<54}", "");
    println!(
        "{:<30} {:>12} {:>12}",
        "peak_atoms", nr.peak_atoms, re.peak_atoms
    );
    println!(
        "{:<30} {:>12} {:>12}",
        "measurement_count", nr.measurement_count, re.measurement_count
    );
    println!(
        "{:<30} {:>12} {:>12}",
        "reset_count", nr.reset_count, re.reset_count
    );
    println!(
        "{:<30} {:>12} {:>12}",
        "reuse_count", nr.reuse_count, re.reuse_count
    );
    println!(
        "{:<30} {:>12} {:>12}",
        "reused_ancilla_count", nr.reused_ancilla_count, re.reused_ancilla_count
    );
    println!();
    println!(
        "Peak-atom reduction: {} → {} ({} fewer atoms)",
        nr.peak_atoms,
        re.peak_atoms,
        nr.peak_atoms - re.peak_atoms
    );
    println!();
    println!(
        "Allocated-atoms series (no-reuse): {:?}",
        nr.allocated_atoms_series
    );
    println!(
        "Allocated-atoms series (reuse):    {:?}",
        re.allocated_atoms_series
    );
}
