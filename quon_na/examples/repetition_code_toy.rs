//! Demo: expand a distance-3 repetition-code logical qubit through the
//! production QEC expansion narrative (ADR-0015 / ADR-0030, issue #319).
//!
//! Single path: `WorkloadBuilder` → `quon_qec::expand_workload` →
//! `ExpandedWorkload` → `quon_na::qec::code_blocks_from_expanded`. There is no
//! separate "code block expansion" toy expander; report sizing flows from the
//! same expansion IR that drives hybrid NA scheduling and Stim/experiment emit.
//!
//! Formula: `N(d) = 2d − 1` (architecture_model.md §10.2 / [Kelly15]).
//! Resource-accounting demo only — not a decoder.

use quon_na::qec::{CodeBlockId, code_blocks_from_expanded};
use quon_qec::{LogicalBasis, SourceFamily, WorkloadBuilder, expand_workload};

fn main() {
    let distance = 3;

    let mut builder = WorkloadBuilder::new();
    if let Err(error) = builder.construct(
        SourceFamily::Repetition,
        distance,
        LogicalBasis::Z,
        quon_qec::LogicalQubitId(0),
    ) {
        eprintln!("construct failed: {error}");
        std::process::exit(1);
    }

    let expanded = match expand_workload(&builder.finish()) {
        Ok(expanded) => expanded,
        Err(error) => {
            eprintln!("expand_workload failed: {error}");
            std::process::exit(1);
        }
    };

    let blocks = code_blocks_from_expanded(&expanded);
    let block = match blocks.first() {
        Some(b) => b,
        None => {
            eprintln!("expansion produced no code blocks");
            std::process::exit(1);
        }
    };

    println!("RepetitionCodeToy distance={distance}");
    println!("logical qubits: {:?}", block.logical_qubits);
    println!("physical atoms (N=2d-1): {}", block.atoms.len());
    println!("atom ids: {:?}", block.atoms);
    println!("block id: {:?}", CodeBlockId(0) == block.id);
    println!(
        "expanded rounds: {} (construct + memory rounds from the workload ops)",
        expanded.rounds.len()
    );
}
