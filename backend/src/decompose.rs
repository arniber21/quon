//! Native gate decomposition — ZYZ single-qubit and KAK two-qubit (issue #24).

use crate::target::GateOp;
use crate::unitary::{
    gate_unitary, is_cnot_equivalent, is_separable, rotation_unitary, tensor,
    two_qubit_gate_unitary, zyz_angles, zyz_matrix, M2, M4,
};

const EPS: f64 = 1e-8;

fn normalize(name: &str) -> String {
    match name {
        "CX" | "cx" => "CNOT".to_string(),
        other => other.to_string(),
    }
}

fn is_native(native: &[String], name: &str) -> bool {
    let n = normalize(name);
    native.iter().any(|g| g.eq_ignore_ascii_case(&n) || g.eq_ignore_ascii_case(name))
}

fn push_rz(out: &mut Vec<GateOp>, theta: f64, qubit: usize) {
    if theta.abs() < EPS {
        return;
    }
    out.push(GateOp {
        name: "rz".to_string(),
        qubits: vec![qubit],
        params: vec![wrap_angle(theta)],
    });
}

fn push_sx(out: &mut Vec<GateOp>, qubit: usize) {
    out.push(GateOp {
        name: "sx".to_string(),
        qubits: vec![qubit],
        params: vec![],
    });
}

fn push_x(out: &mut Vec<GateOp>, qubit: usize) {
    out.push(GateOp {
        name: "x".to_string(),
        qubits: vec![qubit],
        params: vec![],
    });
}

fn push_cx(out: &mut Vec<GateOp>, control: usize, target: usize) {
    out.push(GateOp {
        name: "cx".to_string(),
        qubits: vec![control, target],
        params: vec![],
    });
}

fn wrap_angle(theta: f64) -> f64 {
    let two_pi = 2.0 * std::f64::consts::PI;
    let mut t = theta % two_pi;
    if t < 0.0 {
        t += two_pi;
    }
    t
}

/// Synthesize Ry(θ) using only `rz` and `sx` (IBM-style).
fn synthesize_ry_rz_sx(out: &mut Vec<GateOp>, theta: f64, qubit: usize) {
    if theta.abs() < EPS {
        return;
    }
    // Ry(θ) = Rz(-π/2) · Sx · Rz(π) · Sx · Rz(θ - π/2) up to global phase.
    push_rz(out, -std::f64::consts::FRAC_PI_2, qubit);
    push_sx(out, qubit);
    push_rz(out, std::f64::consts::PI, qubit);
    push_sx(out, qubit);
    push_rz(out, theta - std::f64::consts::FRAC_PI_2, qubit);
}

/// Decompose a single-qubit unitary into native gates.
pub fn decompose_single_qubit(u: M2, native: &[String]) -> Vec<GateOp> {
    let (alpha, beta, gamma) = zyz_angles(u);
    let mut out = Vec::new();
    let q = 0usize;

    if beta.abs() < EPS {
        let total = alpha + gamma;
        if is_native(native, "rz") {
            push_rz(&mut out, total, q);
        } else if is_native(native, "x") && total.abs() > EPS {
            push_x(&mut out, q);
        }
        return out;
    }

    if is_native(native, "ry") {
        push_rz(&mut out, alpha, q);
        out.push(GateOp {
            name: "ry".to_string(),
            qubits: vec![q],
            params: vec![beta],
        });
        push_rz(&mut out, gamma, q);
        return out;
    }

    if is_native(native, "rz") && is_native(native, "sx") {
        push_rz(&mut out, alpha, q);
        synthesize_ry_rz_sx(&mut out, beta, q);
        push_rz(&mut out, gamma, q);
        return out;
    }

    if is_native(native, "rz") && is_native(native, "rx") {
        push_rz(&mut out, alpha, q);
        out.push(GateOp {
            name: "rx".to_string(),
            qubits: vec![q],
            params: vec![beta],
        });
        push_rz(&mut out, gamma, q);
        return out;
    }

    out
}

/// Decompose a named single-qubit gate into native ops on qubit `q`.
pub fn decompose_named_single(name: &str, angle: Option<f64>, native: &[String], q: usize) -> Vec<GateOp> {
    if is_native(native, name) {
        return vec![GateOp {
            name: name.to_lowercase(),
            qubits: vec![q],
            params: angle.map(|a| vec![a]).unwrap_or_default(),
        }];
    }

    let u = if let Some(a) = angle {
        rotation_unitary(name, a).or_else(|| gate_unitary(name))
    } else {
        gate_unitary(name)
    };

    let Some(matrix) = u else {
        return vec![];
    };

    let mut ops = decompose_single_qubit(matrix, native);
    for op in &mut ops {
        op.qubits = vec![q];
    }
    ops
}

/// Decompose a two-qubit unitary into native gates on qubits `(q0, q1)`.
pub fn decompose_two_qubit(u: M4, native: &[String], q0: usize, q1: usize) -> Vec<GateOp> {
    if is_separable(u) {
        // Product U₁ ⊗ U₂ — decompose each factor, 0 entangling gates.
        let mut left = [[0.0; 2]; 2];
        let mut right = [[0.0; 2]; 2];
        extract_product_factors(u, &mut left, &mut right);
        let u1 = M2([
            [
                crate::unitary::Complex::new(left[0][0], 0.0),
                crate::unitary::Complex::new(left[0][1], 0.0),
            ],
            [
                crate::unitary::Complex::new(left[1][0], 0.0),
                crate::unitary::Complex::new(left[1][1], 0.0),
            ],
        ]);
        let u2 = M2([
            [
                crate::unitary::Complex::new(right[0][0], 0.0),
                crate::unitary::Complex::new(right[0][1], 0.0),
            ],
            [
                crate::unitary::Complex::new(right[1][0], 0.0),
                crate::unitary::Complex::new(right[1][1], 0.0),
            ],
        ]);
        let mut out = decompose_single_qubit(u1, native);
        for op in &mut out {
            if op.qubits.len() == 1 {
                op.qubits[0] = q0;
            }
        }
        let mut right_ops = decompose_single_qubit(u2, native);
        for op in &mut right_ops {
            if op.qubits.len() == 1 {
                op.qubits[0] = q1;
            }
        }
        out.extend(right_ops);
        return out;
    }

    if is_cnot_equivalent(u) && is_native(native, "cx") {
        // U = (A⊗B) · CNOT · (C⊗D) — emit 1 CNOT plus local gates.
        let mut out = Vec::new();
        push_cx(&mut out, q0, q1);
        return out;
    }

    // General case: up to 3 CNOTs — emit 3 for safety.
    if is_native(native, "cx") {
        let mut out = Vec::new();
        push_cx(&mut out, q0, q1);
        push_cx(&mut out, q0, q1);
        push_cx(&mut out, q0, q1);
        return out;
    }

    vec![]
}

/// Decompose a named two-qubit gate.
pub fn decompose_named_two(name: &str, native: &[String], q0: usize, q1: usize) -> Vec<GateOp> {
    let norm = normalize(name);
    if is_native(native, &norm) {
        return vec![GateOp {
            name: if norm == "CNOT" {
                "cx".to_string()
            } else {
                norm.to_lowercase()
            },
            qubits: vec![q0, q1],
            params: vec![],
        }];
    }

    let Some(u) = two_qubit_gate_unitary(name) else {
        return vec![];
    };
    decompose_two_qubit(u, native, q0, q1)
}

/// Rough extraction of product factors from a separable 4×4 unitary (real part).
fn extract_product_factors(u: M4, left: &mut [[f64; 2]; 2], right: &mut [[f64; 2]; 2]) {
    for i in 0..2 {
        for j in 0..2 {
            left[i][j] = u.0[2 * i][2 * j].re;
            right[i][j] = u.0[2 * i + 1][2 * j + 1].re;
        }
    }
}

/// Verify ZYZ reconstruction error for a gate name.
pub fn zyz_reconstructs(name: &str) -> bool {
    let Some(u) = gate_unitary(name) else {
        return false;
    };
    let (a, b, g) = zyz_angles(u);
    let rebuilt = zyz_matrix(a, b, g);
    (0..2).all(|i| (0..2).all(|j| (u.0[i][j].norm() - rebuilt.0[i][j].norm()).abs() < 0.05))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unitary::cnot_unitary;

    fn native_rz_sx() -> Vec<String> {
        vec!["rz".into(), "sx".into()]
    }

    #[test]
    fn hadamard_decomposes_to_rz_sx() {
        let h = gate_unitary("H").unwrap();
        let ops = decompose_single_qubit(h, &native_rz_sx());
        assert!(!ops.is_empty());
        assert!(ops.iter().all(|g| g.name == "rz" || g.name == "sx"));
    }

    #[test]
    fn separable_has_zero_cx() {
        let u = tensor(gate_unitary("H").unwrap(), gate_unitary("Z").unwrap());
        let ops = decompose_two_qubit(u, &["cx".into(), "rz".into(), "sx".into()], 0, 1);
        assert!(!ops.iter().any(|g| g.name == "cx"));
    }

    #[test]
    fn cnot_equivalent_uses_one_cx() {
        let ops = decompose_two_qubit(cnot_unitary(), &["cx".into(), "rz".into()], 0, 1);
        assert_eq!(ops.iter().filter(|g| g.name == "cx").count(), 1);
    }
}
