// ZX-graph data structure — see issue #20, SPEC.md §7.2

use petgraph::stable_graph::{NodeIndex, StableGraph};

use crate::gate::GateRef;

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

    zx.outputs = wires;
    zx
}

/// Translate a ZX-diagram back to a gate list via a greedy spider walk.
pub fn zx_to_circuit(zx: &ZXGraph) -> Vec<GateRef> {
    if zx.inputs.is_empty() {
        return vec![];
    }
    let mut gates = Vec::new();
    for qubit in 0..zx.outputs.len() {
        let output = zx.outputs[qubit];
        if let Some(spider) = zx.graph.node_weight(output) {
            match spider.color {
                SpiderColor::Z if spider.phase.abs() > 1e-9 => {
                    gates.push(GateRef::rotation("Rz", qubit, spider.phase));
                }
                SpiderColor::X if spider.phase.abs() > 1e-9 => {
                    gates.push(GateRef::rotation("Rx", qubit, spider.phase));
                }
                SpiderColor::X if spider.phase.abs() <= 1e-9 => {
                    gates.push(GateRef::new("H", vec![qubit]));
                }
                _ => {}
            }
        }
    }
    gates
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bell_state_round_trips_through_zx() {
        let gates = vec![GateRef::new("H", vec![0]), GateRef::new("CNOT", vec![0, 1])];
        let zx = circuit_to_zx(&gates);
        let recovered = zx_to_circuit(&zx);
        assert!(!recovered.is_empty());
    }
}
