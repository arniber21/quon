//! Fuzz `ConnectivityGraph::try_from_edges` with arbitrary qubit counts and
//! edge lists: validation must reject bad input rather than panic / index out
//! of bounds. Run with `cargo +nightly fuzz run fuzz_connectivity`.
#![no_main]

use arbitrary::Arbitrary;
use libfuzzer_sys::fuzz_target;

#[derive(Arbitrary, Debug)]
struct Input {
    num_qubits: u8,
    edges: Vec<(u8, u8)>,
}

fuzz_target!(|input: Input| {
    // Cap the qubit count so the distance matrix stays bounded.
    let n = (input.num_qubits % 64) as usize;
    let edges: Vec<(usize, usize)> = input
        .edges
        .iter()
        .map(|&(a, b)| (a as usize, b as usize))
        .collect();
    let _ = backend::target::ConnectivityGraph::try_from_edges(n, edges);
});
