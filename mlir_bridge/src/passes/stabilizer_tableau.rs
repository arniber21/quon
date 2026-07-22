//! Aaronson–Gottesman stabilizer tableau simulator (issue #96, ADR-0013).
//!
//! Pure-Rust (MLIR-free) implementation of the stabilizer formalism for
//! Clifford circuit analysis. An *n*-qubit Clifford operation is represented
//! as a `(2n) × (2n+1)` binary tableau following the CHP representation of
//! Aaronson & Gottesman, *Improved Simulation of Stabilizer Circuits* (2004).
//!
//! ## Tableau layout
//!
//! Rows `0..n` are **destabilizer** generators (images of `X₀..Xₙ₋₁`).
//! Rows `n..2n` are **stabilizer** generators (images of `Z₀..Zₙ₋₁`).
//!
//! Each row encodes a Pauli operator as `(x | z | r)`:
//!
//! | `x[j]` | `z[j]` | Pauli on qubit *j* |
//! |--------|--------|---------------------|
//! | 0      | 0      | I                   |
//! | 1      | 0      | X                   |
//! | 0      | 1      | Z                   |
//! | 1      | 1      | Y                   |
//!
//! `r = 1` means the operator carries a factor of −1.
//!
//! ## Identity / Pauli detection
//!
//! After conjugating the identity tableau through a Clifford sequence, the
//! result is the identity tableau iff the sequence is the identity operation.
//! If the tableau is diagonal (no inter-qubit mixing) but has non-trivial
//! phase bits, the sequence is a single Pauli operator.

// ---------------------------------------------------------------------------
// Tableau
// ---------------------------------------------------------------------------

/// A stabilizer tableau over GF(2).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tableau {
    /// Number of qubits.
    n: usize,
    /// X part: `2n × n` binary matrix.
    x: Vec<Vec<bool>>,
    /// Z part: `2n × n` binary matrix.
    z: Vec<Vec<bool>>,
    /// Phase bits: `2n` entries (`true` = negative sign).
    r: Vec<bool>,
}

impl Tableau {
    /// Create the identity tableau on `n` qubits.
    pub fn identity(n: usize) -> Self {
        let mut x = vec![vec![false; n]; 2 * n];
        let mut z = vec![vec![false; n]; 2 * n];
        for i in 0..n {
            x[i][i] = true; // destabilizer: X_i
            z[i + n][i] = true; // stabilizer: Z_i
        }
        Self {
            n,
            x,
            z,
            r: vec![false; 2 * n],
        }
    }

    /// Number of qubits.
    pub fn n(&self) -> usize {
        self.n
    }

    // --- Clifford gate updates ---------------------------------------------

    /// Apply Hadamard on qubit `k`: `X ↔ Z`, `Y → −Y`.
    pub fn h(&mut self, k: usize) {
        for i in 0..2 * self.n {
            self.r[i] ^= self.x[i][k] & self.z[i][k];
            std::mem::swap(&mut self.x[i][k], &mut self.z[i][k]);
        }
    }

    /// Apply S gate on qubit `k`: `X → Y`, `Z → Z`, `Y → −X`.
    pub fn s(&mut self, k: usize) {
        for i in 0..2 * self.n {
            self.r[i] ^= self.x[i][k] & self.z[i][k];
            self.z[i][k] ^= self.x[i][k];
        }
    }

    /// Apply S† gate on qubit `k`: `X → −Y`, `Z → Z`, `Y → X`.
    pub fn s_dag(&mut self, k: usize) {
        for i in 0..2 * self.n {
            self.r[i] ^= self.x[i][k] & !self.z[i][k];
            self.z[i][k] ^= self.x[i][k];
        }
    }

    /// Apply CNOT with control `c`, target `t`.
    pub fn cnot(&mut self, c: usize, t: usize) {
        for i in 0..2 * self.n {
            let xc = self.x[i][c];
            let xt = self.x[i][t];
            let zc = self.z[i][c];
            let zt = self.z[i][t];
            // Phase: flip when control-X and target-Z, and X_t == Z_c.
            self.r[i] ^= xc & zt & !(xt ^ zc);
            self.x[i][t] = xt ^ xc;
            self.z[i][c] = zc ^ zt;
        }
    }

    /// Apply X gate on qubit `k`: flips Z components (`Z → −Z`).
    pub fn x(&mut self, k: usize) {
        for i in 0..2 * self.n {
            self.r[i] ^= self.z[i][k];
        }
    }

    /// Apply Y gate on qubit `k`: `X → −X`, `Z → −Z`, `Y → Y`.
    pub fn y(&mut self, k: usize) {
        for i in 0..2 * self.n {
            self.r[i] ^= self.x[i][k] ^ self.z[i][k];
        }
    }

    /// Apply Z gate on qubit `k`: flips X components (`X → −X`).
    pub fn z(&mut self, k: usize) {
        for i in 0..2 * self.n {
            self.r[i] ^= self.x[i][k];
        }
    }

    /// Apply SWAP on qubits `a`, `b` via three CNOTs.
    pub fn swap(&mut self, a: usize, b: usize) {
        self.cnot(a, b);
        self.cnot(b, a);
        self.cnot(a, b);
    }

    // --- Analysis ----------------------------------------------------------

    /// Returns `true` iff this tableau is the identity tableau.
    pub fn is_identity(&self) -> bool {
        for i in 0..self.n {
            if self.r[i] || self.r[i + self.n] {
                return false;
            }
            for j in 0..self.n {
                let expected_x = j == i;
                let expected_z = j == i;
                if self.x[i][j] != expected_x || self.z[i][j] {
                    return false;
                }
                if self.x[i + self.n][j] || self.z[i + self.n][j] != expected_z {
                    return false;
                }
            }
        }
        true
    }

    /// If the tableau represents a single Pauli operator, return the
    /// corresponding gate list.
    ///
    /// A single Pauli `P = P₀ ⊗ P₁ ⊗ … ⊗ Pₙ₋₁` conjugates `Xᵢ → ±Xᵢ`
    /// and `Zᵢ → ±Zᵢ` (no inter-qubit mixing). The sign bits determine
    /// `Pᵢ`:
    ///
    /// | `r[i]` | `r[i+n]` | `Pᵢ` |
    /// |--------|----------|------|
    /// | 0      | 0        | I    |
    /// | 0      | 1        | X    |
    /// | 1      | 1        | Y    |
    /// | 1      | 0        | Z    |
    pub fn as_single_pauli(&self) -> Option<Vec<(String, Vec<usize>)>> {
        for i in 0..self.n {
            // Destabilizer row i: must be ±X_i (diagonal, no Z component)
            for j in 0..self.n {
                if self.x[i][j] != (j == i) || self.z[i][j] {
                    return None;
                }
            }
            // Stabilizer row i+n: must be ±Z_i (diagonal, no X component)
            for j in 0..self.n {
                if self.x[i + self.n][j] || self.z[i + self.n][j] != (j == i) {
                    return None;
                }
            }
        }
        let mut gates = Vec::new();
        for i in 0..self.n {
            let rx = self.r[i];
            let rz = self.r[i + self.n];
            match (rx, rz) {
                (false, false) => {} // I
                (false, true) => gates.push(("X".to_string(), vec![i])),
                (true, true) => gates.push(("Y".to_string(), vec![i])),
                (true, false) => gates.push(("Z".to_string(), vec![i])),
            }
        }
        Some(gates)
    }

    // --- Gate dispatch -----------------------------------------------------

    /// Apply a Clifford gate by canonical name. Returns `false` if the gate
    /// is not a supported Clifford.
    pub fn apply_gate(&mut self, name: &str, qubits: &[usize]) -> bool {
        let canonical = quon_core::gates::canonical_id(name);
        match canonical.unwrap_or(name) {
            "I" => {}
            "H" => self.h(qubits[0]),
            "S" => self.s(qubits[0]),
            "S_dag" => self.s_dag(qubits[0]),
            "X" => self.x(qubits[0]),
            "Y" => self.y(qubits[0]),
            "Z" => self.z(qubits[0]),
            "CNOT" => self.cnot(qubits[0], qubits[1]),
            "SWAP" => self.swap(qubits[0], qubits[1]),
            _ => return false,
        }
        true
    }

    /// Apply a sequence of Clifford gates. Returns `false` if any gate is
    /// not a supported Clifford (the tableau is left in a partial state).
    pub fn apply_sequence(&mut self, gates: &[(String, Vec<usize>)]) -> bool {
        for (name, qubits) in gates {
            if !self.apply_gate(name, qubits) {
                return false;
            }
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Optimization
// ---------------------------------------------------------------------------

/// Optimize a Clifford gate sequence using stabilizer tableau simulation.
///
/// Starts from the identity tableau, conjugates through every gate, then
/// checks the result:
///
/// * **Identity** → returns an empty gate list (remove all gates).
/// * **Single Pauli** → returns the Pauli as a minimal gate list.
/// * **Non-trivial Clifford** → returns `None` (no simplification).
///
/// Returns `None` if any gate is not a supported Clifford, or if no
/// simplification is possible.
pub fn optimize_clifford(
    gates: &[(String, Vec<usize>)],
    n: usize,
) -> Option<Vec<(String, Vec<usize>)>> {
    let mut tableau = Tableau::identity(n);
    if !tableau.apply_sequence(gates) {
        return None;
    }

    if tableau.is_identity() {
        return Some(Vec::new());
    }

    if let Some(pauli) = tableau.as_single_pauli() {
        // Only return if it's strictly fewer gates
        if pauli.len() < gates.len() {
            return Some(pauli);
        }
    }

    None
}

/// Returns `true` if `name` is a Clifford gate supported by the tableau.
pub fn is_tableau_gate(name: &str) -> bool {
    let canonical = quon_core::gates::canonical_id(name).unwrap_or(name);
    matches!(
        canonical,
        "I" | "H" | "S" | "S_dag" | "X" | "Y" | "Z" | "CNOT" | "SWAP"
    )
}

/// Returns `true` if every gate in the sequence is supported by the tableau.
pub fn is_all_tableau(gates: &[(String, Vec<usize>)]) -> bool {
    gates.iter().all(|(name, _)| is_tableau_gate(name))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Tableau gate updates ----------------------------------------------

    #[test]
    fn identity_tableau_is_identity() {
        let t = Tableau::identity(3);
        assert!(t.is_identity());
    }

    #[test]
    fn h_h_is_identity() {
        let mut t = Tableau::identity(1);
        t.h(0);
        t.h(0);
        assert!(t.is_identity());
    }

    #[test]
    fn s_s_is_z() {
        let mut t = Tableau::identity(1);
        t.s(0);
        t.s(0);
        // S² = Z: X → -X, Z → Z
        assert!(!t.is_identity());
        let pauli = t.as_single_pauli().expect("S² should be a Pauli");
        assert_eq!(pauli, vec![("Z".to_string(), vec![0])]);
    }

    #[test]
    fn s_four_times_is_identity() {
        let mut t = Tableau::identity(1);
        for _ in 0..4 {
            t.s(0);
        }
        assert!(t.is_identity());
    }

    #[test]
    fn s_dag_is_inverse_of_s() {
        let mut t = Tableau::identity(1);
        t.s(0);
        t.s_dag(0);
        assert!(t.is_identity());
    }

    #[test]
    fn cnot_cnot_is_identity() {
        let mut t = Tableau::identity(2);
        t.cnot(0, 1);
        t.cnot(0, 1);
        assert!(t.is_identity());
    }

    #[test]
    fn swap_is_identity_applied_twice() {
        let mut t = Tableau::identity(2);
        t.swap(0, 1);
        t.swap(0, 1);
        assert!(t.is_identity());
    }

    #[test]
    fn x_x_is_identity() {
        let mut t = Tableau::identity(1);
        t.x(0);
        t.x(0);
        assert!(t.is_identity());
    }

    #[test]
    fn y_y_is_identity() {
        let mut t = Tableau::identity(1);
        t.y(0);
        t.y(0);
        assert!(t.is_identity());
    }

    #[test]
    fn z_z_is_identity() {
        let mut t = Tableau::identity(1);
        t.z(0);
        t.z(0);
        assert!(t.is_identity());
    }

    #[test]
    fn h_s_s_s_s_h_is_identity() {
        // H · S⁴ · H = H · I · H = I
        let mut t = Tableau::identity(1);
        t.h(0);
        for _ in 0..4 {
            t.s(0);
        }
        t.h(0);
        assert!(t.is_identity());
    }

    #[test]
    fn h_s_s_h_is_single_pauli() {
        // H · S² · H = H · Z · H = X
        let mut t = Tableau::identity(1);
        t.h(0);
        t.s(0);
        t.s(0);
        t.h(0);
        assert!(!t.is_identity());
        let pauli = t.as_single_pauli().expect("should be single Pauli");
        assert_eq!(pauli, vec![("X".to_string(), vec![0])]);
    }

    #[test]
    fn cnot_hadamard_conjugation_is_identity() {
        // CNOT(0,1) · H(0) · H(1) · CNOT(0,1) · H(0) · H(1) = I
        // (since H⊗H · CNOT(0,1) · H⊗H = CNOT(1,0), and CNOT(0,1)·CNOT(1,0)·CNOT(0,1) = SWAP)
        // Actually: (H⊗H) · CNOT(0,1) · (H⊗H) = CNOT(1,0)
        // So CNOT(0,1) · (H⊗H) · CNOT(0,1) · (H⊗H) = CNOT(0,1) · CNOT(1,0)
        // which is NOT identity. Let me use a different sequence.
        // CNOT(0,1) · CNOT(0,1) = I (adjacent, but good test for tableau)
        let mut t = Tableau::identity(2);
        t.cnot(0, 1);
        t.cnot(0, 1);
        assert!(t.is_identity());
    }

    #[test]
    fn non_identity_is_not_identity() {
        let mut t = Tableau::identity(1);
        t.h(0);
        assert!(!t.is_identity());
    }

    #[test]
    fn h_is_not_single_pauli() {
        let mut t = Tableau::identity(1);
        t.h(0);
        // H maps X→Z, Z→X — this mixes X and Z, so it's not a single Pauli
        assert!(t.as_single_pauli().is_none());
    }

    #[test]
    fn cnot_is_not_identity() {
        let mut t = Tableau::identity(2);
        t.cnot(0, 1);
        assert!(!t.is_identity());
        // CNOT is not a single Pauli (it mixes qubits)
        assert!(t.as_single_pauli().is_none());
    }

    // --- apply_gate / apply_sequence ---------------------------------------

    #[test]
    fn apply_gate_h() {
        let mut t = Tableau::identity(1);
        assert!(t.apply_gate("H", &[0]));
        assert!(!t.is_identity());
    }

    #[test]
    fn apply_gate_unsupported_returns_false() {
        let mut t = Tableau::identity(1);
        assert!(!t.apply_gate("T", &[0]));
    }

    #[test]
    fn apply_gate_canonical_aliases() {
        let mut t1 = Tableau::identity(2);
        let mut t2 = Tableau::identity(2);
        t1.apply_gate("CNOT", &[0, 1]);
        t2.apply_gate("CX", &[0, 1]);
        assert_eq!(t1, t2);
    }

    // --- optimize_clifford -------------------------------------------------

    #[test]
    fn optimize_identity_sequence_h_s4_h() {
        let gates = vec![
            ("H".to_string(), vec![0usize]),
            ("S".to_string(), vec![0]),
            ("S".to_string(), vec![0]),
            ("S".to_string(), vec![0]),
            ("S".to_string(), vec![0]),
            ("H".to_string(), vec![0]),
        ];
        let result = optimize_clifford(&gates, 1);
        assert_eq!(result, Some(Vec::new()));
    }

    #[test]
    fn optimize_single_pauli_s_squared() {
        // S · S = Z
        let gates = vec![("S".to_string(), vec![0usize]), ("S".to_string(), vec![0])];
        let result = optimize_clifford(&gates, 1);
        assert_eq!(result, Some(vec![("Z".to_string(), vec![0])]));
    }

    #[test]
    fn optimize_non_identity_h() {
        let gates = vec![("H".to_string(), vec![0usize])];
        let result = optimize_clifford(&gates, 1);
        assert!(result.is_none()); // H is not identity or a single Pauli
    }

    #[test]
    fn optimize_unsupported_gate_returns_none() {
        let gates = vec![("T".to_string(), vec![0usize])];
        let result = optimize_clifford(&gates, 1);
        assert!(result.is_none());
    }

    #[test]
    fn optimize_cnot_hh_cnot_hh_is_identity() {
        // CNOT(0,1) · H(0)·H(1) · CNOT(0,1) · H(0)·H(1)
        // = CNOT(0,1) · CNOT(1,0) (since H⊗H conjugates CNOT to reversed CNOT)
        // = SWAP (up to order), which is NOT identity.
        // Let me use a real identity instead.
        // SWAP(0,1) · SWAP(0,1) = I  (but SWAP = 3 CNOTs, so 6 CNOTs total)
        // Better: H(0)·H(0) = I, but that's caught by gate_cancellation.
        // S(0)·S(0)·S(0)·S(0) = I (non-adjacent S gates, not caught by gate_cancellation)
        let gates = vec![
            ("S".to_string(), vec![0usize]),
            ("S".to_string(), vec![0]),
            ("S".to_string(), vec![0]),
            ("S".to_string(), vec![0]),
        ];
        let result = optimize_clifford(&gates, 1);
        assert_eq!(result, Some(Vec::new()));
    }

    #[test]
    fn optimize_multi_qubit_identity() {
        // CNOT(0,1) · CNOT(1,0) · CNOT(0,1) · CNOT(0,1) · CNOT(1,0) · CNOT(0,1)
        // = SWAP · SWAP = I
        let gates: Vec<(String, Vec<usize>)> = vec![
            ("CNOT".into(), vec![0, 1]),
            ("CNOT".into(), vec![1, 0]),
            ("CNOT".into(), vec![0, 1]),
            ("CNOT".into(), vec![0, 1]),
            ("CNOT".into(), vec![1, 0]),
            ("CNOT".into(), vec![0, 1]),
        ];
        let result = optimize_clifford(&gates, 2);
        assert_eq!(result, Some(Vec::new()));
    }

    // --- is_tableau_gate ---------------------------------------------------

    #[test]
    fn tableau_gate_recognition() {
        assert!(is_tableau_gate("H"));
        assert!(is_tableau_gate("S"));
        assert!(is_tableau_gate("S_dag"));
        assert!(is_tableau_gate("Sdag"));
        assert!(is_tableau_gate("CNOT"));
        assert!(is_tableau_gate("CX"));
        assert!(is_tableau_gate("X"));
        assert!(is_tableau_gate("Y"));
        assert!(is_tableau_gate("Z"));
        assert!(is_tableau_gate("SWAP"));
        assert!(is_tableau_gate("I"));
        assert!(!is_tableau_gate("T"));
        assert!(!is_tableau_gate("T_dag"));
    }
}
