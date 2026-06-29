//! Opaque gate reference for circuit ↔ ZX translation.

#[derive(Debug, Clone, PartialEq)]
pub struct GateRef {
    pub name: String,
    pub qubits: Vec<usize>,
    pub angle: Option<f64>,
}

impl GateRef {
    pub fn new(name: impl Into<String>, qubits: Vec<usize>) -> Self {
        Self {
            name: name.into(),
            qubits,
            angle: None,
        }
    }

    pub fn rotation(name: impl Into<String>, qubit: usize, angle: f64) -> Self {
        Self {
            name: name.into(),
            qubits: vec![qubit],
            angle: Some(angle),
        }
    }
}
