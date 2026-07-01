//! Native gate decomposition — ZYZ single-qubit and KAK two-qubit (issue #24).

use crate::target::GateOp;
use crate::unitary::{
    Complex, M2, M4, gate_unitary, is_separable, rotation_unitary, two_qubit_gate_unitary,
    unitary_distance, unitary_distance2, zyz_angles, zyz_matrix,
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
    native
        .iter()
        .any(|g| g.eq_ignore_ascii_case(&n) || g.eq_ignore_ascii_case(name))
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

fn remap_ops(ops: &mut [GateOp], qubit: usize) {
    for op in ops {
        if op.qubits.len() == 1 {
            op.qubits[0] = qubit;
        }
    }
}

fn wrap_angle(theta: f64) -> f64 {
    let two_pi = 2.0 * std::f64::consts::PI;
    let mut t = theta % two_pi;
    if t < 0.0 {
        t += two_pi;
    }
    t
}

/// Synthesize Ry(θ) using only `rz` and `sx` (IBM-style), up to global phase.
fn synthesize_ry_rz_sx(out: &mut Vec<GateOp>, theta: f64, qubit: usize) {
    if theta.abs() < EPS {
        return;
    }
    // Emitted order composes as Rz(π) · Sx · Rz(θ + π) · Sx up to global phase.
    push_sx(out, qubit);
    push_rz(out, theta + std::f64::consts::PI, qubit);
    push_sx(out, qubit);
    push_rz(out, std::f64::consts::PI, qubit);
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
        push_rz(&mut out, gamma, q);
        out.push(GateOp {
            name: "ry".to_string(),
            qubits: vec![q],
            params: vec![beta],
        });
        push_rz(&mut out, alpha, q);
        return out;
    }

    if is_native(native, "rz") && is_native(native, "sx") {
        push_rz(&mut out, gamma, q);
        synthesize_ry_rz_sx(&mut out, beta, q);
        push_rz(&mut out, alpha, q);
        return out;
    }

    if is_native(native, "rz") && is_native(native, "rx") {
        push_rz(&mut out, gamma, q);
        out.push(GateOp {
            name: "rx".to_string(),
            qubits: vec![q],
            params: vec![beta],
        });
        push_rz(&mut out, alpha, q);
        return out;
    }

    out
}

/// Decompose a named single-qubit gate into native ops on qubit `q`.
pub fn decompose_named_single(
    name: &str,
    angle: Option<f64>,
    native: &[String],
    q: usize,
) -> Vec<GateOp> {
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
        let mut left = [[Complex::new(0.0, 0.0); 2]; 2];
        let mut right = [[Complex::new(0.0, 0.0); 2]; 2];
        extract_product_factors(u, &mut left, &mut right);
        let u1 = M2(left);
        let u2 = M2(right);
        let mut out = decompose_single_qubit(u1, native);
        remap_ops(&mut out, q0);
        let mut right_ops = decompose_single_qubit(u2, native);
        remap_ops(&mut right_ops, q1);
        out.extend(right_ops);
        return out;
    }

    if unitary_distance(u, crate::unitary::cnot_unitary()) < EPS && is_native(native, "cx") {
        let mut out = Vec::new();
        push_cx(&mut out, q0, q1);
        return out;
    }

    if let Some(cz) = two_qubit_gate_unitary("CZ")
        && unitary_distance(u, cz) < EPS
        && is_native(native, "cx")
    {
        let mut out = Vec::new();
        let mut h_before = decompose_named_single("H", None, native, q1);
        out.append(&mut h_before);
        push_cx(&mut out, q0, q1);
        let mut h_after = decompose_named_single("H", None, native, q1);
        out.append(&mut h_after);
        return out;
    }

    if let Some(swap) = two_qubit_gate_unitary("SWAP")
        && unitary_distance(u, swap) < EPS
        && is_native(native, "cx")
    {
        let mut out = Vec::new();
        push_cx(&mut out, q0, q1);
        push_cx(&mut out, q1, q0);
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

/// Extract product factors from a separable 4×4 unitary.
fn extract_product_factors(u: M4, left: &mut [[Complex; 2]; 2], right: &mut [[Complex; 2]; 2]) {
    // First non-negligible entry, in row-major order. The enumerate indices feed
    // only arithmetic (block/offset decomposition), never array access, so Flux
    // discharges the search without bounds reasoning.
    let pivot = u.0.iter().enumerate().find_map(|(row, u_row)| {
        u_row.iter().enumerate().find_map(|(col, &val)| {
            (val.norm() > EPS).then_some((row / 2, row % 2, col / 2, col % 2, val))
        })
    });
    let Some((a0, c0, b0, d0, scale)) = pivot else {
        return;
    };
    // `left[i][j] = u[c0 + 2i][d0 + 2j]`: the strided 2×2 sub-grid with row/col
    // offsets `c0`/`d0` and stride 2. `skip(..).step_by(2)` walks exactly those
    // rows/cols, so the fill stays iterator-based and provably in bounds.
    for (left_row, u_row) in left.iter_mut().zip(u.0.iter().skip(c0).step_by(2)) {
        for (cell, &val) in left_row.iter_mut().zip(u_row.iter().skip(d0).step_by(2)) {
            *cell = val;
        }
    }
    // `right[i][j] = u[2*a0 + i][2*b0 + j] / scale`: the contiguous 2×2 block at
    // block-row `a0`, block-col `b0`. `skip(2*block).take(2)` selects it.
    for (right_row, u_row) in right.iter_mut().zip(u.0.iter().skip(2 * a0).take(2)) {
        for (cell, &val) in right_row.iter_mut().zip(u_row.iter().skip(2 * b0).take(2)) {
            *cell = val / scale;
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
    unitary_distance2(u, rebuilt) < 1e-9
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::unitary::{cnot_unitary, mul2, mul4, tensor};

    fn native_rz_sx() -> Vec<String> {
        vec!["rz".into(), "sx".into()]
    }

    fn gate_op_unitary(op: &GateOp) -> M2 {
        if let Some(theta) = op.params.first() {
            rotation_unitary(&op.name, *theta).unwrap()
        } else {
            gate_unitary(&op.name).unwrap()
        }
    }

    fn compose_single_ops(ops: &[GateOp]) -> M2 {
        ops.iter().fold(gate_unitary("I").unwrap(), |acc, op| {
            mul2(gate_op_unitary(op), acc)
        })
    }

    fn expand_two_qubit_op(op: &GateOp) -> M4 {
        if op.qubits.len() == 1 {
            let single = gate_op_unitary(op);
            if op.qubits[0] == 0 {
                tensor(single, gate_unitary("I").unwrap())
            } else {
                tensor(gate_unitary("I").unwrap(), single)
            }
        } else if op.name == "cx" && op.qubits == [0, 1] {
            cnot_unitary()
        } else if op.name == "cx" && op.qubits == [1, 0] {
            M4([
                [
                    Complex::new(1.0, 0.0),
                    Complex::new(0.0, 0.0),
                    Complex::new(0.0, 0.0),
                    Complex::new(0.0, 0.0),
                ],
                [
                    Complex::new(0.0, 0.0),
                    Complex::new(0.0, 0.0),
                    Complex::new(0.0, 0.0),
                    Complex::new(1.0, 0.0),
                ],
                [
                    Complex::new(0.0, 0.0),
                    Complex::new(0.0, 0.0),
                    Complex::new(1.0, 0.0),
                    Complex::new(0.0, 0.0),
                ],
                [
                    Complex::new(0.0, 0.0),
                    Complex::new(1.0, 0.0),
                    Complex::new(0.0, 0.0),
                    Complex::new(0.0, 0.0),
                ],
            ])
        } else {
            two_qubit_gate_unitary(&op.name).unwrap()
        }
    }

    fn compose_two_ops(ops: &[GateOp]) -> M4 {
        ops.iter().fold(
            tensor(gate_unitary("I").unwrap(), gate_unitary("I").unwrap()),
            |acc, op| mul4(expand_two_qubit_op(op), acc),
        )
    }

    #[test]
    fn hadamard_decomposes_to_rz_sx() {
        let h = gate_unitary("H").unwrap();
        let ops = decompose_single_qubit(h, &native_rz_sx());
        assert!(!ops.is_empty());
        assert!(ops.iter().all(|g| g.name == "rz" || g.name == "sx"));
        assert!(unitary_distance2(h, compose_single_ops(&ops)) < 1e-9);
    }

    #[test]
    fn standard_single_qubit_gates_decompose_equivalently() {
        for name in ["X", "Y", "Z", "H", "S", "T", "sx"] {
            let u = gate_unitary(name).unwrap();
            let ops = decompose_single_qubit(u, &native_rz_sx());
            assert!(
                unitary_distance2(u, compose_single_ops(&ops)) < 1e-9,
                "{name}"
            );
        }
    }

    #[test]
    fn separable_has_zero_cx() {
        let u = tensor(gate_unitary("H").unwrap(), gate_unitary("Z").unwrap());
        let ops = decompose_two_qubit(u, &["cx".into(), "rz".into(), "sx".into()], 0, 1);
        assert!(!ops.iter().any(|g| g.name == "cx"));
        assert!(unitary_distance(u, compose_two_ops(&ops)) < 1e-8);
    }

    #[test]
    fn cnot_equivalent_uses_one_cx() {
        let ops = decompose_two_qubit(cnot_unitary(), &["cx".into(), "rz".into()], 0, 1);
        assert_eq!(ops.iter().filter(|g| g.name == "cx").count(), 1);
        assert!(unitary_distance(cnot_unitary(), compose_two_ops(&ops)) < 1e-8);
    }

    #[test]
    fn cz_decomposes_exactly_with_cx_and_local_h() {
        let cz = two_qubit_gate_unitary("CZ").unwrap();
        let ops = decompose_two_qubit(cz, &["cx".into(), "rz".into(), "sx".into()], 0, 1);
        assert_eq!(ops.iter().filter(|g| g.name == "cx").count(), 1);
        assert!(unitary_distance(cz, compose_two_ops(&ops)) < 1e-8);
    }

    #[test]
    fn swap_decomposes_exactly_to_three_cx() {
        let swap = two_qubit_gate_unitary("SWAP").unwrap();
        let ops = decompose_two_qubit(swap, &["cx".into(), "rz".into(), "sx".into()], 0, 1);
        assert_eq!(ops.iter().filter(|g| g.name == "cx").count(), 3);
        assert!(unitary_distance(swap, compose_two_ops(&ops)) < 1e-8);
    }
}
