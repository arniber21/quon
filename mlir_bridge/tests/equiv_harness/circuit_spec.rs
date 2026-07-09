//! Random circuit IR for the equivalence harness (issue #118).

use std::f64::consts::PI;

use proptest::prelude::*;

/// Named gate kinds supported by the harness simulator and lowerer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(clippy::upper_case_acronyms)] // Gate names match MLIR / OpenQASM spellings.
pub enum GateKind {
    H,
    X,
    Y,
    Z,
    S,
    Sdag,
    T,
    Tdag,
    CNOT,
    CZ,
    SWAP,
    Rx,
    Ry,
    Rz,
}

impl GateKind {
    pub fn arity(self) -> usize {
        match self {
            Self::CNOT | Self::CZ | Self::SWAP => 2,
            _ => 1,
        }
    }

    pub fn is_rotation(self) -> bool {
        matches!(self, Self::Rx | Self::Ry | Self::Rz)
    }

    /// Gate name as written into `quantum.circ.gate` / rotation ops.
    pub fn mlir_name(self) -> &'static str {
        match self {
            Self::H => "H",
            Self::X => "X",
            Self::Y => "Y",
            Self::Z => "Z",
            Self::S => "S",
            Self::Sdag => "S†",
            Self::T => "T",
            Self::Tdag => "T†",
            Self::CNOT => "CNOT",
            Self::CZ => "CZ",
            Self::SWAP => "SWAP",
            Self::Rx => "Rx",
            Self::Ry => "Ry",
            Self::Rz => "Rz",
        }
    }

    /// True when the uncomputation pass knows an inverse for this gate.
    pub fn has_known_inverse(self) -> bool {
        !self.is_rotation()
    }

    pub fn is_clifford(self) -> bool {
        matches!(
            self,
            Self::H
                | Self::X
                | Self::Y
                | Self::Z
                | Self::S
                | Self::Sdag
                | Self::CNOT
                | Self::CZ
                | Self::SWAP
        )
    }
}

/// One gate application on logical qubit indices `< width`.
#[derive(Clone, Debug, PartialEq)]
pub struct GateInst {
    pub kind: GateKind,
    pub qubits: Vec<u8>,
    pub angle: Option<f64>,
}

impl GateInst {
    pub fn new(kind: GateKind, qubits: impl Into<Vec<u8>>) -> Self {
        Self {
            kind,
            qubits: qubits.into(),
            angle: None,
        }
    }

    pub fn rotation(kind: GateKind, qubit: u8, angle: f64) -> Self {
        Self {
            kind,
            qubits: vec![qubit],
            angle: Some(angle),
        }
    }
}

/// A small circuit: width `1..=4`, depth (gate count) `0..=24`.
#[derive(Clone, Debug, PartialEq)]
pub struct CircuitSpec {
    pub width: u8,
    pub gates: Vec<GateInst>,
}

impl CircuitSpec {
    pub fn new(width: u8, gates: Vec<GateInst>) -> Self {
        debug_assert!((1..=4).contains(&width));
        Self { width, gates }
    }

    pub fn depth(&self) -> usize {
        self.gates.len()
    }

    pub fn all_clifford(&self) -> bool {
        self.gates.iter().all(|g| g.kind.is_clifford())
    }
}

fn discrete_angle() -> impl Strategy<Value = f64> {
    prop_oneof![
        Just(0.0),
        Just(PI / 4.0),
        Just(PI / 2.0),
        Just(PI),
        Just(3.0 * PI / 2.0),
        Just(2.0 * PI),
        Just(-PI / 4.0),
        Just(0.3),
        Just(0.5),
        (0.0f64..std::f64::consts::TAU),
    ]
}

fn one_qubit_kind() -> impl Strategy<Value = GateKind> {
    prop_oneof![
        Just(GateKind::H),
        Just(GateKind::X),
        Just(GateKind::Y),
        Just(GateKind::Z),
        Just(GateKind::S),
        Just(GateKind::Sdag),
        Just(GateKind::T),
        Just(GateKind::Tdag),
        Just(GateKind::Rx),
        Just(GateKind::Ry),
        Just(GateKind::Rz),
    ]
}

fn two_qubit_kind() -> impl Strategy<Value = GateKind> {
    prop_oneof![
        3 => Just(GateKind::CNOT),
        1 => Just(GateKind::CZ),
        1 => Just(GateKind::SWAP),
    ]
}

fn one_qubit_gate(width: u8) -> impl Strategy<Value = GateInst> {
    (one_qubit_kind(), 0u8..width, discrete_angle()).prop_map(|(kind, q, angle)| {
        if kind.is_rotation() {
            GateInst::rotation(kind, q, angle)
        } else {
            GateInst::new(kind, vec![q])
        }
    })
}

fn two_qubit_gate(width: u8) -> impl Strategy<Value = GateInst> {
    let w = width as usize;
    (two_qubit_kind(), 0usize..w, 0usize..w).prop_filter_map(
        "distinct qubits",
        move |(kind, a, b)| {
            if a == b {
                None
            } else {
                Some(GateInst::new(kind, vec![a as u8, b as u8]))
            }
        },
    )
}

fn gate_for_width(width: u8) -> BoxedStrategy<GateInst> {
    if width < 2 {
        one_qubit_gate(width).boxed()
    } else {
        prop_oneof![
            4 => one_qubit_gate(width),
            1 => two_qubit_gate(width),
        ]
        .boxed()
    }
}

/// General random circuits for cancellation / clifford_t / SABRE.
pub fn circuit_strategy() -> impl Strategy<Value = CircuitSpec> {
    (1u8..=4).prop_flat_map(|width| {
        prop::collection::vec(gate_for_width(width), 0usize..=24)
            .prop_map(move |gates| CircuitSpec::new(width, gates))
    })
}

/// Circuits biased toward adjacent same-axis rotations (merge opportunities).
pub fn rotation_biased_strategy() -> impl Strategy<Value = CircuitSpec> {
    (1u8..=4).prop_flat_map(|width| {
        let axis = prop_oneof![Just(GateKind::Rz), Just(GateKind::Rx), Just(GateKind::Ry)];
        (
            axis,
            prop::collection::vec(discrete_angle(), 1usize..=8),
            prop::collection::vec(gate_for_width(width), 0usize..=8),
            0u8..width,
        )
            .prop_map(move |(axis, angles, extra, q)| {
                let mut gates: Vec<GateInst> = angles
                    .into_iter()
                    .map(|a| GateInst::rotation(axis, q, a))
                    .collect();
                // Sprinkle interrupters / other gates after the merge chain.
                gates.extend(extra);
                CircuitSpec::new(width, gates)
            })
    })
}

/// Width-1 circuits for ZX simplification (pass declines n≠1).
pub fn width1_strategy() -> impl Strategy<Value = CircuitSpec> {
    prop::collection::vec(gate_for_width(1), 0usize..=24)
        .prop_map(|gates| CircuitSpec::new(1, gates))
}

/// Width-1 rotation chains that ZX / merge should be able to fuse.
pub fn width1_rotation_chain_strategy() -> impl Strategy<Value = CircuitSpec> {
    (
        prop_oneof![Just(GateKind::Rz), Just(GateKind::Rx), Just(GateKind::Ry)],
        prop::collection::vec(discrete_angle(), 2usize..=6),
    )
        .prop_map(|(axis, angles)| {
            let gates = angles
                .into_iter()
                .map(|a| GateInst::rotation(axis, 0, a))
                .collect();
            CircuitSpec::new(1, gates)
        })
}

/// Reversible (known-inverse) gates only — for uncomputation borrow bodies.
fn reversible_one_qubit(width: u8) -> impl Strategy<Value = GateInst> {
    prop_oneof![
        Just(GateKind::H),
        Just(GateKind::X),
        Just(GateKind::Y),
        Just(GateKind::Z),
        Just(GateKind::S),
        Just(GateKind::Sdag),
        Just(GateKind::T),
        Just(GateKind::Tdag),
    ]
    .prop_flat_map(move |kind| {
        (Just(kind), 0u8..width).prop_map(|(kind, q)| GateInst::new(kind, vec![q]))
    })
}

pub fn reversible_gate_for_width(width: u8) -> BoxedStrategy<GateInst> {
    if width < 2 {
        reversible_one_qubit(width).boxed()
    } else {
        prop_oneof![
            3 => reversible_one_qubit(width),
            1 => two_qubit_gate(width),
        ]
        .boxed()
    }
}

pub fn reversible_circuit_strategy() -> impl Strategy<Value = CircuitSpec> {
    (1u8..=4).prop_flat_map(|width| {
        prop::collection::vec(reversible_gate_for_width(width), 1usize..=12)
            .prop_map(move |gates| CircuitSpec::new(width, gates))
    })
}

/// Multi-qubit circuits for SABRE (need ≥2 qubits for routing interest).
pub fn sabre_circuit_strategy() -> impl Strategy<Value = CircuitSpec> {
    (2u8..=4).prop_flat_map(|width| {
        prop::collection::vec(gate_for_width(width), 1usize..=16)
            .prop_map(move |gates| CircuitSpec::new(width, gates))
    })
}
