//! Minimal Flux refinement-type checks for the Quon workspace.
//!
//! Run with `cargo flux -p flux_verify` (requires `cargo-flux` on PATH).
//! This crate uses a nightly toolchain; the rest of the workspace stays on stable.

use flux_rs::attrs::*;

/// Returns a value strictly greater than the input.
#[spec(fn(x: i32) -> i32{v: x < v})]
pub fn inc(x: i32) -> i32 {
    x + 1
}

/// Natural numbers are non-negative.
#[spec(fn(n: usize) -> usize{v: 0 <= v})]
pub fn id_usize(n: usize) -> usize {
    n
}
