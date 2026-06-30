// ZX-calculus rewrite rules — issue #20, SPEC.md §7.2

use std::collections::VecDeque;
use std::f64::consts::PI;

use petgraph::stable_graph::NodeIndex;

use crate::graph::{PHASE_EPS, WireKind, ZXGraph};

/// Apply rewrite rules to fixpoint. Returns the number of rewrites applied.
pub fn simplify(zx: &mut ZXGraph) -> usize {
    let mut total = 0usize;
    loop {
        let mut worklist: VecDeque<NodeIndex> = zx.graph.node_indices().collect();
        let mut changed = 0usize;
        while let Some(node) = worklist.pop_front() {
            if spider_fusion(zx, node) {
                changed += 1;
                worklist = zx.graph.node_indices().collect();
                continue;
            }
            if identity_removal(zx, node) {
                changed += 1;
                worklist = zx.graph.node_indices().collect();
            }
        }
        if changed == 0 {
            break;
        }
        total += changed;
    }
    total
}

/// Fuse `node` with an adjacent same-colour spider joined by a Regular edge,
/// folding the partner's phase and edges into `node` and deleting the partner.
///
/// Boundaries are never fused or deleted: doing so would leave `inputs` or
/// `outputs` pointing at a removed node. Because the partner that gets removed
/// is required to be interior, `inputs`/`outputs` always stay valid.
fn spider_fusion(zx: &mut ZXGraph, node: NodeIndex) -> bool {
    if zx.is_boundary(node) {
        return false;
    }
    let Some(spider) = zx.graph.node_weight(node).cloned() else {
        return false;
    };
    // Pick an interior, same-colour, Regular-edge partner to absorb.
    let Some((other, _)) = zx.neighbors(node).into_iter().find(|(neighbor, kind)| {
        *kind == WireKind::Regular
            && !zx.is_boundary(*neighbor)
            && zx
                .graph
                .node_weight(*neighbor)
                .is_some_and(|partner| partner.color == spider.color)
    }) else {
        return false;
    };

    let other_phase = zx.graph.node_weight(other).expect("partner").phase;
    if let Some(merged) = zx.graph.node_weight_mut(node) {
        merged.phase = normalize_phase(spider.phase + other_phase);
    }
    for (neighbor, kind) in zx.neighbors(other) {
        if neighbor != node {
            zx.graph.add_edge(node, neighbor, kind);
        }
    }
    zx.graph.remove_node(other);
    true
}

/// Remove a phase-0, degree-2, Regular-wired interior spider, bridging its two
/// neighbours. Boundaries and Hadamard-edged spiders are left untouched.
fn identity_removal(zx: &mut ZXGraph, node: NodeIndex) -> bool {
    if zx.is_boundary(node) {
        return false;
    }
    let Some(spider) = zx.graph.node_weight(node).cloned() else {
        return false;
    };
    let neighbors = zx.neighbors(node);
    if spider.phase.abs() > PHASE_EPS || neighbors.len() != 2 {
        return false;
    }
    let (left, left_kind) = neighbors[0].clone();
    let (right, right_kind) = neighbors[1].clone();
    if left_kind != WireKind::Regular || right_kind != WireKind::Regular {
        return false;
    }
    zx.graph.add_edge(left, right, WireKind::Regular);
    zx.graph.remove_node(node);
    true
}

fn normalize_phase(phase: f64) -> f64 {
    let mut value = phase % (2.0 * PI);
    if value < 0.0 {
        value += 2.0 * PI;
    }
    value
}

#[allow(dead_code)] // remaining SPEC rules — wired in follow-up
fn pi_copy(_zx: &mut ZXGraph) -> bool {
    false
}
fn bialgebra(_zx: &mut ZXGraph) -> bool {
    false
}
fn euler_decomposition(_zx: &mut ZXGraph) -> bool {
    false
}
fn color_change(_zx: &mut ZXGraph) -> bool {
    false
}
fn state_copy(_zx: &mut ZXGraph) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gate::GateRef;
    use crate::graph::{circuit_to_zx, zx_to_circuit};

    #[test]
    fn spider_fusion_merges_same_color_neighbors() {
        let gates = vec![
            GateRef::rotation("Rz", 0, 0.5),
            GateRef::rotation("Rz", 0, 0.3),
        ];
        let mut zx = circuit_to_zx(&gates);
        let before = zx.graph.node_count();
        let rewrites = simplify(&mut zx);
        let recovered = zx_to_circuit(&zx);
        assert!(rewrites > 0);
        assert!(zx.graph.node_count() < before);
        // The two Rz must fuse into a single Rz with the summed angle — not be
        // dropped (the old extractor produced an empty circuit here).
        assert_eq!(recovered, vec![GateRef::rotation("Rz", 0, 0.8)]);
    }

    #[test]
    fn opposite_rotations_cancel_to_identity() {
        // Rz(π)·Rz(π) fuses to phase 0, leaving a bare wire. Extraction yields
        // an empty circuit, which callers read as "identity" only because the
        // diagram is a fully-walked chain, not because extraction declined.
        let gates = vec![
            GateRef::rotation("Rz", 0, PI),
            GateRef::rotation("Rz", 0, PI),
        ];
        let mut zx = circuit_to_zx(&gates);
        assert!(simplify(&mut zx) > 0);
        assert!(zx_to_circuit(&zx).is_empty());
    }

    #[test]
    fn worklist_reaches_fixpoint() {
        let gates = vec![
            GateRef::rotation("Rz", 0, 0.25),
            GateRef::rotation("Rz", 0, 0.25),
            GateRef::rotation("Rz", 0, 0.25),
            GateRef::rotation("Rz", 0, 0.25),
        ];
        let mut zx = circuit_to_zx(&gates);
        simplify(&mut zx);
        let again = simplify(&mut zx);
        assert_eq!(again, 0);
    }
}
