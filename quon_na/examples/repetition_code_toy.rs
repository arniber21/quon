//! Toy demo: expand a distance-3 repetition-code logical qubit into atoms.
//!
//! This is a resource-accounting demo only (issue #109) — not a decoder.
//! Formula: `N(d) = 2d − 1` (architecture_model.md §10.2 / [Kelly15]).

use quon_na::qec::{CodeBlockId, CodeFamily, LogicalOp, LogicalQubitId, expand_code_block};

fn main() {
    let distance = 3;
    let block = expand_code_block(
        CodeBlockId(0),
        CodeFamily::RepetitionCodeToy { distance },
        vec![LogicalQubitId(0)],
        0,
    )
    .unwrap_or_else(|error| {
        eprintln!("expand_code_block failed: {error}");
        std::process::exit(1);
    });

    println!("RepetitionCodeToy distance={distance}");
    println!("logical qubits: {:?}", block.logical_qubits);
    println!("physical atoms (N=2d-1): {}", block.atoms.len());
    println!("atom ids: {:?}", block.atoms);
    println!(
        "demo logical ops (no decoder): {:?}, {:?}, {:?}",
        LogicalOp::LogicalX,
        LogicalOp::SyndromeRound,
        LogicalOp::MeasureLogical,
    );
}
