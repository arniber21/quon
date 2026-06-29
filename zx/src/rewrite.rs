// ZX-calculus rewrite rules — issue #20, SPEC.md §7.2

use std::collections::VecDeque;
use std::f64::consts::PI;

use petgraph::Direction;
use petgraph::stable_graph::NodeIndex;
use petgraph::visit::EdgeRef;

use crate::graph::{Spider, SpiderColor, WireKind, ZXGraph};

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

fn neighbors(zx: &ZXGraph, node: NodeIndex) -> Vec<(NodeIndex, WireKind)> {
    zx.graph
        .edges_directed(node, Direction::Incoming)
        .map(|edge| (edge.source(), edge.weight().clone()))
        .chain(
            zx.graph
                .edges_directed(node, Direction::Outgoing)
                .map(|edge| (edge.target(), edge.weight().clone())),
        )
        .collect()
}

fn degree(zx: &ZXGraph, node: NodeIndex) -> usize {
    zx.graph.edges_directed(node, Direction::Incoming).count()
        + zx.graph.edges_directed(node, Direction::Outgoing).count()
}

fn spider_fusion(zx: &mut ZXGraph, node: NodeIndex) -> bool {
    let Some(spider) = zx.graph.node_weight(node).cloned() else {
        return false;
    };
    let node_neighbors = neighbors(zx, node);
    if node_neighbors.len() != 1 {
        return false;
    }
    let (other, WireKind::Regular) = &node_neighbors[0] else {
        return false;
    };
    let other = *other;
    let Some(other_spider) = zx.graph.node_weight(other).cloned() else {
        return false;
    };
    if spider.color != other_spider.color {
        return false;
    }
    let merged_phase = normalize_phase(spider.phase + other_spider.phase);
    zx.graph.node_weight_mut(node).expect("node").phase = merged_phase;
    let other_node_neighbors = neighbors(zx, other);
    for (neighbor, kind) in other_node_neighbors {
        if neighbor == node {
            continue;
        }
        zx.graph.add_edge(node, neighbor, kind);
    }
    zx.graph.remove_node(other);
    true
}

fn identity_removal(zx: &mut ZXGraph, node: NodeIndex) -> bool {
    let Some(spider) = zx.graph.node_weight(node).cloned() else {
        return false;
    };
    if spider.phase.abs() > 1e-9 || degree(zx, node) != 2 {
        return false;
    }
    let neighbors = neighbors(zx, node);
    if neighbors.len() != 2 {
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
        assert!(recovered.len() <= 1);
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
