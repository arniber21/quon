// ZX-calculus rewrite rules — see issue #20, SPEC.md §7.2
// Applies rules to fixpoint via a worklist algorithm on ZXGraph.

#![allow(dead_code)] // stub rewrite rules; implemented in issue #20

use crate::graph::ZXGraph;

/// Opaque reference to a gate op in the MLIR IR (passed across the crate boundary).
pub struct GateRef(pub String); // placeholder — will be an MLIR value wrapper

/// Apply all rewrite rules to fixpoint. Returns the number of rewrites applied.
pub fn simplify(_zx: &mut ZXGraph) -> usize {
    todo!("ZX rewrite worklist — see issue #20")
}

// Individual rules — each returns true if a rewrite was applied.

fn spider_fusion(_zx: &mut ZXGraph) -> bool {
    todo!()
}
fn identity_removal(_zx: &mut ZXGraph) -> bool {
    todo!()
}
fn pi_copy(_zx: &mut ZXGraph) -> bool {
    todo!()
}
fn bialgebra(_zx: &mut ZXGraph) -> bool {
    todo!()
}
fn euler_decomposition(_zx: &mut ZXGraph) -> bool {
    todo!()
}
fn color_change(_zx: &mut ZXGraph) -> bool {
    todo!()
}
fn state_copy(_zx: &mut ZXGraph) -> bool {
    todo!()
}
