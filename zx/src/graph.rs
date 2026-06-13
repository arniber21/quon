// ZX-graph data structure — see issue #20, SPEC.md §7.2
// Built on petgraph::StableGraph so node indices remain valid across rewriting.

use petgraph::stable_graph::StableGraph;

#[derive(Debug, Clone, PartialEq)]
pub enum SpiderColor { Z, X }

#[derive(Debug, Clone)]
pub struct Spider {
    pub color: SpiderColor,
    pub phase: f64, // phase angle in [0, 2π)
}

#[derive(Debug, Clone, PartialEq)]
pub enum WireKind {
    Regular,
    Hadamard,
}

/// A ZX-diagram. Input/output boundary nodes have phase 0.
pub struct ZXGraph {
    pub graph: StableGraph<Spider, WireKind>,
    pub inputs: Vec<petgraph::stable_graph::NodeIndex>,
    pub outputs: Vec<petgraph::stable_graph::NodeIndex>,
}

impl ZXGraph {
    pub fn new() -> Self {
        Self {
            graph: StableGraph::new(),
            inputs: vec![],
            outputs: vec![],
        }
    }
}

/// Translate a gate list to a ZX-diagram.
pub fn circuit_to_zx(_gates: &[crate::rewrite::GateRef]) -> ZXGraph {
    todo!("circuit → ZX translation — see issue #20")
}

/// Translate a ZX-diagram back to a gate list via Euler decomposition.
pub fn zx_to_circuit(_zx: &ZXGraph) -> Vec<crate::rewrite::GateRef> {
    todo!("ZX → circuit back-translation — see issue #20")
}
