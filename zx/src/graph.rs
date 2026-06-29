// ZX-graph data structure — see issue #20, SPEC.md §7.2

use petgraph::Direction;
use petgraph::stable_graph::{NodeIndex, StableGraph};
use petgraph::visit::EdgeRef;

use crate::gate::GateRef;

/// Phases within this tolerance of zero are treated as identity.
pub(crate) const PHASE_EPS: f64 = 1e-9;

#[derive(Debug, Clone, PartialEq)]
pub enum SpiderColor {
    Z,
    X,
}

#[derive(Debug, Clone)]
pub struct Spider {
    pub color: SpiderColor,
    pub phase: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WireKind {
    Regular,
    Hadamard,
}

/// A ZX-diagram. Input/output boundary nodes have phase 0.
pub struct ZXGraph {
    pub graph: StableGraph<Spider, WireKind>,
    pub inputs: Vec<NodeIndex>,
    pub outputs: Vec<NodeIndex>,
}

impl Default for ZXGraph {
    fn default() -> Self {
        Self::new()
    }
}

impl ZXGraph {
    pub fn new() -> Self {
        Self {
            graph: StableGraph::new(),
            inputs: vec![],
            outputs: vec![],
        }
    }

    fn add_boundary(&mut self) -> NodeIndex {
        self.graph.add_node(Spider {
            color: SpiderColor::Z,
            phase: 0.0,
        })
    }

    fn connect(&mut self, from: NodeIndex, to: NodeIndex, kind: WireKind) {
        self.graph.add_edge(from, to, kind);
    }

    /// True when `node` is an input or output boundary. Boundaries carry no gate
    /// and must never be fused or deleted, otherwise `inputs`/`outputs` dangle.
    pub(crate) fn is_boundary(&self, node: NodeIndex) -> bool {
        self.inputs.contains(&node) || self.outputs.contains(&node)
    }

    /// Undirected neighbours of `node` paired with the connecting wire kind.
    pub(crate) fn neighbors(&self, node: NodeIndex) -> Vec<(NodeIndex, WireKind)> {
        self.graph
            .edges_directed(node, Direction::Incoming)
            .map(|edge| (edge.source(), edge.weight().clone()))
            .chain(
                self.graph
                    .edges_directed(node, Direction::Outgoing)
                    .map(|edge| (edge.target(), edge.weight().clone())),
            )
            .collect()
    }
}

/// Translate a gate list to a ZX-diagram.
pub fn circuit_to_zx(gates: &[GateRef]) -> ZXGraph {
    let mut zx = ZXGraph::new();
    if gates.is_empty() {
        return zx;
    }
    let qubit_count = gates
        .iter()
        .flat_map(|gate| gate.qubits.iter().copied())
        .max()
        .map(|index| index + 1)
        .unwrap_or(1);
    let mut wires: Vec<NodeIndex> = (0..qubit_count).map(|_| zx.add_boundary()).collect();
    zx.inputs = wires.clone();

    for gate in gates {
        match gate.name.as_str() {
            "H" => {
                let q = gate.qubits[0];
                let mid = zx.graph.add_node(Spider {
                    color: SpiderColor::X,
                    phase: 0.0,
                });
                zx.connect(wires[q], mid, WireKind::Hadamard);
                wires[q] = mid;
            }
            "Z" | "Rz" => {
                let q = gate.qubits[0];
                let phase = gate.angle.unwrap_or(0.0);
                let spider = zx.graph.add_node(Spider {
                    color: SpiderColor::Z,
                    phase,
                });
                zx.connect(wires[q], spider, WireKind::Regular);
                wires[q] = spider;
            }
            "X" | "Rx" => {
                let q = gate.qubits[0];
                let phase = gate.angle.unwrap_or(0.0);
                let spider = zx.graph.add_node(Spider {
                    color: SpiderColor::X,
                    phase,
                });
                zx.connect(wires[q], spider, WireKind::Regular);
                wires[q] = spider;
            }
            "CNOT" | "CX" => {
                let control = gate.qubits[0];
                let target = gate.qubits[1];
                let z = zx.graph.add_node(Spider {
                    color: SpiderColor::Z,
                    phase: 0.0,
                });
                let x = zx.graph.add_node(Spider {
                    color: SpiderColor::X,
                    phase: 0.0,
                });
                zx.connect(wires[control], z, WireKind::Regular);
                zx.connect(z, x, WireKind::Regular);
                zx.connect(wires[target], x, WireKind::Regular);
                wires[control] = z;
                wires[target] = x;
            }
            _ => {}
        }
    }

    // Terminate each wire in a dedicated phase-0 boundary so every gate spider
    // stays interior and is free to fuse without invalidating `outputs`.
    zx.outputs = wires
        .into_iter()
        .map(|wire| {
            let boundary = zx.add_boundary();
            zx.connect(wire, boundary, WireKind::Regular);
            boundary
        })
        .collect();
    zx
}

/// Translate a ZX-diagram back to a gate list by walking each wire from its
/// input boundary to its output boundary.
///
/// This extractor is deliberately **all-or-nothing**: it only reconstructs
/// diagrams that are independent single-qubit chains of Regular-edge Z/X
/// spiders (rotations and Paulis). If it meets a branch (degree > 2, e.g. the
/// shared spiders of a `CNOT`), a Hadamard edge, or a dangling boundary, it
/// returns an empty vector to signal "cannot faithfully extract" rather than
/// emitting a circuit that differs from the diagram. Callers must treat the
/// empty result as "no rewrite" — never as a verified identity — so an
/// incomplete extraction can never silently corrupt a circuit.
pub fn zx_to_circuit(zx: &ZXGraph) -> Vec<GateRef> {
    let mut gates = Vec::new();
    for (qubit, &input) in zx.inputs.iter().enumerate() {
        let Some(&output) = zx.outputs.get(qubit) else {
            return Vec::new();
        };
        let mut previous: Option<NodeIndex> = None;
        let mut current = input;
        while current != output {
            let forward: Vec<(NodeIndex, WireKind)> = zx
                .neighbors(current)
                .into_iter()
                .filter(|(neighbor, _)| Some(*neighbor) != previous)
                .collect();
            // A faithful single-qubit chain has exactly one way forward and
            // never uses a Hadamard edge (which would denote an `H` gate this
            // encoding cannot round-trip).
            let [(next, WireKind::Regular)] = forward.as_slice() else {
                return Vec::new();
            };
            let next = *next;
            if next != output {
                let Some(spider) = zx.graph.node_weight(next) else {
                    return Vec::new();
                };
                if spider.phase.abs() > PHASE_EPS {
                    let axis = match spider.color {
                        SpiderColor::Z => "Rz",
                        SpiderColor::X => "Rx",
                    };
                    gates.push(GateRef::rotation(axis, qubit, spider.phase));
                }
            }
            previous = Some(current);
            current = next;
        }
    }
    gates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rotation_chain_round_trips_faithfully() {
        // Distinct axes must survive extraction in order — the old extractor
        // kept only the final spider and silently dropped the Rz.
        let gates = vec![
            GateRef::rotation("Rz", 0, 0.5),
            GateRef::rotation("Rx", 0, 0.3),
        ];
        let zx = circuit_to_zx(&gates);
        let recovered = zx_to_circuit(&zx);
        assert_eq!(recovered, gates);
    }

    #[test]
    fn extraction_declines_on_multi_qubit_entangling_diagrams() {
        // The greedy chain walk cannot faithfully extract a CNOT, so it must
        // decline (empty) rather than emit a wrong, shorter circuit.
        let gates = vec![GateRef::new("H", vec![0]), GateRef::new("CNOT", vec![0, 1])];
        let zx = circuit_to_zx(&gates);
        assert!(zx_to_circuit(&zx).is_empty());
    }

    #[test]
    fn independent_single_qubit_wires_extract_each_chain() {
        let gates = vec![
            GateRef::rotation("Rz", 0, 0.5),
            GateRef::rotation("Rx", 1, 0.25),
        ];
        let zx = circuit_to_zx(&gates);
        assert_eq!(zx_to_circuit(&zx), gates);
    }
}
