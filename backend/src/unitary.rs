//! Complex 2×2 and 4×4 unitary matrices for gate decomposition (issue #24).

use std::f64::consts::PI;

/// A complex number with real and imaginary parts.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Complex {
    pub re: f64,
    pub im: f64,
}

impl Complex {
    pub const fn new(re: f64, im: f64) -> Self {
        Self { re, im }
    }

    pub fn norm(self) -> f64 {
        (self.re * self.re + self.im * self.im).sqrt()
    }

    pub fn arg(self) -> f64 {
        self.im.atan2(self.re)
    }

    pub fn conj(self) -> Self {
        Self::new(self.re, -self.im)
    }

    fn exp(self) -> Self {
        let r = self.re.exp();
        Self::new(r * self.im.cos(), r * self.im.sin())
    }

    fn from_polar(theta: f64) -> Self {
        Self::new(theta.cos(), theta.sin())
    }
}

impl std::ops::Add for Complex {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self::new(self.re + rhs.re, self.im + rhs.im)
    }
}

impl std::ops::Sub for Complex {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self::new(self.re - rhs.re, self.im - rhs.im)
    }
}

impl std::ops::Mul for Complex {
    type Output = Self;
    fn mul(self, rhs: Self) -> Self {
        Self::new(
            self.re * rhs.re - self.im * rhs.im,
            self.re * rhs.im + self.im * rhs.re,
        )
    }
}

impl std::ops::Div for Complex {
    type Output = Self;
    fn div(self, rhs: Self) -> Self {
        let denom = rhs.re * rhs.re + rhs.im * rhs.im;
        Self::new(
            (self.re * rhs.re + self.im * rhs.im) / denom,
            (self.im * rhs.re - self.re * rhs.im) / denom,
        )
    }
}

impl std::ops::Neg for Complex {
    type Output = Self;
    fn neg(self) -> Self {
        Self::new(-self.re, -self.im)
    }
}

impl std::ops::Mul<f64> for Complex {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self::new(self.re * rhs, self.im * rhs)
    }
}

/// A single-qubit unitary as a 2×2 complex matrix.
#[derive(Clone, Copy, Debug)]
pub struct M2(pub [[Complex; 2]; 2]);

/// A two-qubit unitary as a 4×4 complex matrix (|q0 q1⟩ row/column ordering).
#[derive(Clone, Copy, Debug)]
pub struct M4(pub [[Complex; 4]; 4]);

const I: Complex = Complex::new(0.0, 1.0);
const ONE: Complex = Complex::new(1.0, 0.0);
const ZERO: Complex = Complex::new(0.0, 0.0);

fn c(re: f64, im: f64) -> Complex {
    Complex::new(re, im)
}

fn kron(a: M2, b: M2) -> M4 {
    let mut out = [[ZERO; 4]; 4];
    for i in 0..2 {
        for j in 0..2 {
            for k in 0..2 {
                for l in 0..2 {
                    let row = 2 * i + k;
                    let col = 2 * j + l;
                    out[row][col] = a.0[i][j] * b.0[k][l];
                }
            }
        }
    }
    M4(out)
}

fn rz(theta: f64) -> M2 {
    let half = Complex::new(0.0, -theta / 2.0).exp();
    M2([[half, ZERO], [ZERO, half.conj()]])
}

fn ry(theta: f64) -> M2 {
    let cos = (theta / 2.0).cos();
    let sin = (theta / 2.0).sin();
    M2([[c(cos, 0.0), c(-sin, 0.0)], [c(sin, 0.0), c(cos, 0.0)]])
}

fn rx(theta: f64) -> M2 {
    let cos = (theta / 2.0).cos();
    let sin = (theta / 2.0).sin();
    M2([[c(cos, 0.0), c(0.0, -sin)], [c(0.0, -sin), c(cos, 0.0)]])
}

fn normalize_name(name: &str) -> &str {
    match name {
        "CX" | "cx" => "CNOT",
        other => other,
    }
}

/// Returns the unitary matrix for a standard gate name (up to global phase).
pub fn gate_unitary(name: &str) -> Option<M2> {
    let s = 1.0 / 2.0_f64.sqrt();
    match normalize_name(name) {
        "I" | "id" => Some(M2([[ONE, ZERO], [ZERO, ONE]])),
        "X" | "x" => Some(M2([[ZERO, ONE], [ONE, ZERO]])),
        "Y" | "y" => Some(M2([[ZERO, -I], [I, ZERO]])),
        "Z" | "z" => Some(M2([[ONE, ZERO], [ZERO, -ONE]])),
        "H" | "h" => Some(M2([[c(s, 0.0), c(s, 0.0)], [c(s, 0.0), c(-s, 0.0)]])),
        "S" | "s" => Some(M2([[ONE, ZERO], [ZERO, I]])),
        "Sdag" | "S†" | "sdg" => Some(M2([[ONE, ZERO], [ZERO, -I]])),
        "T" | "t" => Some(M2([[ONE, ZERO], [ZERO, c(0.0, PI / 4.0).exp()]])),
        "Tdag" | "T†" | "tdg" => Some(M2([[ONE, ZERO], [ZERO, c(0.0, -PI / 4.0).exp()]])),
        "sx" => Some(M2([
            [c(0.5, 0.5), c(0.5, -0.5)],
            [c(0.5, -0.5), c(0.5, 0.5)],
        ])),
        _ => None,
    }
}

/// Returns the unitary for a rotation gate with the given angle (radians).
pub fn rotation_unitary(name: &str, angle: f64) -> Option<M2> {
    match name {
        "Rz" | "rz" => Some(rz(angle)),
        "Ry" | "ry" => Some(ry(angle)),
        "Rx" | "rx" => Some(rx(angle)),
        _ => None,
    }
}

pub fn cnot_unitary() -> M4 {
    M4([
        [ONE, ZERO, ZERO, ZERO],
        [ZERO, ONE, ZERO, ZERO],
        [ZERO, ZERO, ZERO, ONE],
        [ZERO, ZERO, ONE, ZERO],
    ])
}

/// Returns the 4×4 unitary for a standard two-qubit gate.
pub fn two_qubit_gate_unitary(name: &str) -> Option<M4> {
    match normalize_name(name) {
        "CNOT" => Some(cnot_unitary()),
        "SWAP" | "swap" => Some(M4([
            [ONE, ZERO, ZERO, ZERO],
            [ZERO, ZERO, ONE, ZERO],
            [ZERO, ONE, ZERO, ZERO],
            [ZERO, ZERO, ZERO, ONE],
        ])),
        "CZ" | "cz" => Some(M4([
            [ONE, ZERO, ZERO, ZERO],
            [ZERO, ONE, ZERO, ZERO],
            [ZERO, ZERO, ONE, ZERO],
            [ZERO, ZERO, ZERO, c(-1.0, 0.0)],
        ])),
        _ => None,
    }
}

/// Kronecker product of two single-qubit unitaries.
pub fn tensor(a: M2, b: M2) -> M4 {
    kron(a, b)
}

/// Matrix multiply for 2×2.
#[allow(clippy::needless_range_loop)]
pub fn mul2(a: M2, b: M2) -> M2 {
    let mut out = [[ZERO; 2]; 2];
    for i in 0..2 {
        for j in 0..2 {
            for k in 0..2 {
                out[i][j] = out[i][j] + a.0[i][k] * b.0[k][j];
            }
        }
    }
    M2(out)
}

pub fn scale2(a: M2, phase: Complex) -> M2 {
    let mut out = a.0;
    for row in &mut out {
        for value in row {
            *value = phase * *value;
        }
    }
    M2(out)
}

/// Matrix multiply for 4×4.
#[allow(clippy::needless_range_loop)]
pub fn mul4(a: M4, b: M4) -> M4 {
    let mut out = [[ZERO; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            for k in 0..4 {
                out[i][j] = out[i][j] + a.0[i][k] * b.0[k][j];
            }
        }
    }
    M4(out)
}

/// ZYZ Euler angles (α, β, γ) such that U ≈ Rz(α) · Ry(β) · Rz(γ) up to global phase.
pub fn zyz_angles(u: M2) -> (f64, f64, f64) {
    let eps = 1e-10;
    let det = u.0[0][0] * u.0[1][1] - u.0[0][1] * u.0[1][0];
    let su2 = scale2(u, Complex::from_polar(-det.arg() / 2.0));
    let sin_half = su2.0[1][0].norm();
    let beta = 2.0 * sin_half.atan2(su2.0[0][0].norm());
    if sin_half > eps {
        let sum = su2.0[1][1].arg() - su2.0[0][0].arg();
        let diff = su2.0[1][0].arg() - (-su2.0[0][1]).arg();
        ((sum + diff) / 2.0, beta, (sum - diff) / 2.0)
    } else {
        (su2.0[1][1].arg() - su2.0[0][0].arg(), 0.0, 0.0)
    }
}

/// True when the 4×4 unitary is a product U₁ ⊗ U₂ (KAK separable).
pub fn is_separable(u: M4) -> bool {
    let mut r = [[ZERO; 4]; 4];
    for a in 0..2 {
        for b in 0..2 {
            for c in 0..2 {
                for d in 0..2 {
                    let row = 2 * a + c;
                    let col = 2 * b + d;
                    r[row][col] = u.0[2 * a + b][2 * c + d];
                }
            }
        }
    }
    matrix_rank_4(&r) <= 1
}

#[allow(clippy::needless_range_loop)]
fn matrix_rank_4(m: &[[Complex; 4]; 4]) -> usize {
    let mut a = *m;
    let mut rank = 0usize;
    for col in 0..4 {
        let mut pivot = rank;
        while pivot < 4 && a[pivot][col].norm() < 1e-10 {
            pivot += 1;
        }
        if pivot == 4 {
            continue;
        }
        a.swap(rank, pivot);
        let scale = a[rank][col];
        if scale.norm() < 1e-30 {
            continue;
        }
        for j in col..4 {
            a[rank][j] = a[rank][j] / scale;
        }
        for i in 0..4 {
            if i == rank {
                continue;
            }
            let factor = a[i][col];
            if factor.norm() < 1e-10 {
                continue;
            }
            for j in col..4 {
                a[i][j] = a[i][j] - factor * a[rank][j];
            }
        }
        rank += 1;
    }
    rank
}

/// Frob-norm distance between two 4×4 unitaries up to global phase.
pub fn unitary_distance(a: M4, b: M4) -> f64 {
    let mut inner = ZERO;
    for i in 0..4 {
        for j in 0..4 {
            inner = inner + a.0[i][j] * b.0[i][j].conj();
        }
    }
    let phase = if inner.norm() > 1e-12 {
        inner / c(inner.norm(), 0.0)
    } else {
        ONE
    };
    let mut sum = 0.0;
    for i in 0..4 {
        for j in 0..4 {
            let diff = a.0[i][j] - phase * b.0[i][j];
            sum += diff.norm() * diff.norm();
        }
    }
    sum.sqrt()
}

/// Frob-norm distance between two 2×2 unitaries up to global phase.
pub fn unitary_distance2(a: M2, b: M2) -> f64 {
    let mut inner = ZERO;
    for i in 0..2 {
        for j in 0..2 {
            inner = inner + a.0[i][j] * b.0[i][j].conj();
        }
    }
    let phase = if inner.norm() > 1e-12 {
        inner / c(inner.norm(), 0.0)
    } else {
        ONE
    };
    let mut sum = 0.0;
    for i in 0..2 {
        for j in 0..2 {
            let diff = a.0[i][j] - phase * b.0[i][j];
            sum += diff.norm() * diff.norm();
        }
    }
    sum.sqrt()
}

/// True when `u` is locally equivalent to CNOT (one entangling gate suffices).
pub fn is_cnot_equivalent(u: M4) -> bool {
    if is_separable(u) {
        return false;
    }
    unitary_distance(u, cnot_unitary()) < 0.05
}

/// Build Rz(θ) · Ry(β) · Rz(γ) as a 2×2 matrix.
pub fn zyz_matrix(alpha: f64, beta: f64, gamma: f64) -> M2 {
    mul2(rz(alpha), mul2(ry(beta), rz(gamma)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn h_zyz_reconstructs() {
        let h = gate_unitary("H").unwrap();
        let (a, b, g) = zyz_angles(h);
        let rebuilt = zyz_matrix(a, b, g);
        assert!(unitary_distance2(h, rebuilt) < 1e-9);
    }

    #[test]
    fn zyz_reconstructs_standard_gates() {
        for name in ["X", "Y", "Z", "H", "S", "T", "sx"] {
            let u = gate_unitary(name).unwrap();
            let (a, b, g) = zyz_angles(u);
            let rebuilt = zyz_matrix(a, b, g);
            assert!(unitary_distance2(u, rebuilt) < 1e-9, "{name}");
        }
    }

    #[test]
    fn tensor_is_separable() {
        let u = tensor(gate_unitary("H").unwrap(), gate_unitary("X").unwrap());
        assert!(is_separable(u));
    }

    #[test]
    fn cnot_is_not_separable() {
        assert!(!is_separable(cnot_unitary()));
    }
}
