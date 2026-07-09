//! Phase-invariant statevector simulator using `backend::unitary` (issue #118).

use backend::unitary::{Complex, M2, M4, gate_unitary, rotation_unitary, two_qubit_gate_unitary};

use super::circuit_spec::{CircuitSpec, GateInst, GateKind};

/// Default numerical tolerance for phase-invariant L2 distance.
pub const TOL: f64 = 1e-8;

const ZERO: Complex = Complex::new(0.0, 0.0);
const ONE: Complex = Complex::new(1.0, 0.0);

/// Amplitudes for an `n`-qubit state (`amps.len() == 2^n`).
#[derive(Clone, Debug)]
pub struct State {
    pub n: u8,
    pub amps: Vec<Complex>,
}

impl State {
    pub fn zeros(n: u8) -> Self {
        let dim = 1usize << n;
        let mut amps = vec![ZERO; dim];
        amps[0] = ONE;
        Self { n, amps }
    }

    pub fn computational_basis(n: u8, bits: usize) -> Self {
        let dim = 1usize << n;
        assert!(bits < dim);
        let mut amps = vec![ZERO; dim];
        amps[bits] = ONE;
        Self { n, amps }
    }
}

/// `|0…0⟩` on `n` qubits.
pub fn basis_state(n: u8) -> State {
    State::zeros(n)
}

fn apply_m2(state: &mut State, target: u8, u: M2) {
    let n = state.n as usize;
    let t = target as usize;
    assert!(t < n);
    let dim = state.amps.len();
    let bit = 1usize << t;
    let mut out = state.amps.clone();
    for i in 0..dim {
        if i & bit != 0 {
            continue;
        }
        let j = i | bit;
        let a0 = state.amps[i];
        let a1 = state.amps[j];
        // |0⟩ component at i, |1⟩ at j — apply U on the target qubit.
        out[i] = u.0[0][0] * a0 + u.0[0][1] * a1;
        out[j] = u.0[1][0] * a0 + u.0[1][1] * a1;
    }
    state.amps = out;
}

fn apply_m4(state: &mut State, q0: u8, q1: u8, u: M4) {
    let n = state.n as usize;
    let a = q0 as usize;
    let b = q1 as usize;
    assert!(a < n && b < n && a != b);
    let dim = state.amps.len();
    let bit0 = 1usize << a;
    let bit1 = 1usize << b;
    let mut out = state.amps.clone();
    for i in 0..dim {
        // Only process the representative where both target bits are 0.
        if i & bit0 != 0 || i & bit1 != 0 {
            continue;
        }
        let i00 = i;
        let i01 = i | bit1;
        let i10 = i | bit0;
        let i11 = i | bit0 | bit1;
        let v = [
            state.amps[i00],
            state.amps[i01],
            state.amps[i10],
            state.amps[i11],
        ];
        // M4 uses |q0 q1⟩ row/column ordering (q0 is the high bit in the 2-bit index).
        let mut w = [ZERO; 4];
        for (row, w_row) in w.iter_mut().enumerate() {
            for (col, &v_col) in v.iter().enumerate() {
                *w_row = *w_row + u.0[row][col] * v_col;
            }
        }
        out[i00] = w[0];
        out[i01] = w[1];
        out[i10] = w[2];
        out[i11] = w[3];
    }
    state.amps = out;
}

fn unitary_for(gate: &GateInst) -> Result<(Option<M2>, Option<M4>), String> {
    match gate.kind {
        GateKind::Rx | GateKind::Ry | GateKind::Rz => {
            let angle = gate
                .angle
                .ok_or_else(|| "rotation missing angle".to_string())?;
            let u = rotation_unitary(gate.kind.mlir_name(), angle)
                .ok_or_else(|| format!("unknown rotation {}", gate.kind.mlir_name()))?;
            Ok((Some(u), None))
        }
        GateKind::CNOT | GateKind::CZ | GateKind::SWAP => {
            let u = two_qubit_gate_unitary(gate.kind.mlir_name())
                .ok_or_else(|| format!("unknown 2Q {}", gate.kind.mlir_name()))?;
            Ok((None, Some(u)))
        }
        other => {
            let u = gate_unitary(other.mlir_name())
                .ok_or_else(|| format!("unknown 1Q {}", other.mlir_name()))?;
            Ok((Some(u), None))
        }
    }
}

/// Apply a single gate in-place.
pub fn apply_gate(state: &mut State, gate: &GateInst) -> Result<(), String> {
    let (m2, m4) = unitary_for(gate)?;
    match gate.kind.arity() {
        1 => {
            let u = m2.ok_or("expected 1Q unitary")?;
            let q = *gate.qubits.first().ok_or("missing qubit")?;
            apply_m2(state, q, u);
        }
        2 => {
            let u = m4.ok_or("expected 2Q unitary")?;
            let q0 = gate.qubits[0];
            let q1 = gate.qubits[1];
            apply_m4(state, q0, q1, u);
        }
        _ => return Err("unsupported arity".into()),
    }
    Ok(())
}

/// Apply every gate in `spec` to `state`.
pub fn apply_circuit(state: &mut State, spec: &CircuitSpec) -> Result<(), String> {
    assert_eq!(state.n, spec.width);
    for gate in &spec.gates {
        apply_gate(state, gate)?;
    }
    Ok(())
}

/// Phase-invariant L2 distance: `min_θ ‖ψ − e^{iθ} φ‖₂`.
pub fn phase_invariant_distance(a: &State, b: &State) -> f64 {
    assert_eq!(a.n, b.n);
    assert_eq!(a.amps.len(), b.amps.len());
    let mut inner = ZERO;
    for (va, vb) in a.amps.iter().zip(b.amps.iter()) {
        inner = inner + (*va) * vb.conj();
    }
    let phase = if inner.norm() > 1e-12 {
        inner / Complex::new(inner.norm(), 0.0)
    } else {
        ONE
    };
    let mut sum = 0.0;
    for (va, vb) in a.amps.iter().zip(b.amps.iter()) {
        let diff = *va - phase * *vb;
        sum += diff.norm() * diff.norm();
    }
    sum.sqrt()
}

/// True when `a` and `b` agree up to global phase within `tol`.
pub fn statevectors_equiv(a: &State, b: &State, tol: f64) -> bool {
    phase_invariant_distance(a, b) <= tol
}

/// Compare two circuits by applying both to several computational-basis inputs.
pub fn circuits_equiv(pre: &CircuitSpec, post: &CircuitSpec, tol: f64) -> Result<(), String> {
    if pre.width != post.width {
        return Err(format!("width mismatch: {} vs {}", pre.width, post.width));
    }
    let n = pre.width;
    let dim = 1usize << n;
    // Always |0…0⟩, plus up to 7 other basis states (cap for CI).
    let mut inputs: Vec<usize> = vec![0];
    let step = (dim / 8).max(1);
    let mut bits = step;
    while bits < dim && inputs.len() < 8 {
        inputs.push(bits);
        bits += step;
    }
    for bits in inputs {
        let mut a = State::computational_basis(n, bits);
        let mut b = State::computational_basis(n, bits);
        apply_circuit(&mut a, pre)?;
        apply_circuit(&mut b, post)?;
        let d = phase_invariant_distance(&a, &b);
        if d > tol {
            return Err(format!(
                "statevector mismatch on |{bits:0width$b}⟩: dist={d} (tol={tol})\npre={pre:?}\npost={post:?}",
                width = n as usize
            ));
        }
    }
    Ok(())
}
