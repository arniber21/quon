//! Phase polynomial extraction, merging, and re-synthesis (issue #96, ADR-0013).
//!
//! Pure-Rust (MLIR-free) implementation of phase-polynomial T-count
//! optimization for `{CNOT, T, T†}` circuit blocks. Non-Clifford gates
//! (T, T†) contribute phases that are linear Boolean functions of the
//! computational basis bits. CNOTs transform these linear functions,
//! enabling **non-adjacent** T-merging that the peephole gate-cancellation
//! pass cannot see.
//!
//! ## Phase polynomial
//!
//! A circuit over `{CNOT, T, T†}` (possibly with interleaved Clifford gates
//! that delimit blocks) implements a unitary
//!
//! ```text
//! U|x⟩ = ω^{f(x)} |A x⟩
//! ```
//!
//! where `A` is a GF(2)-linear map (from CNOTs), `ω = e^{iπ/4}`, and
//! `f(x) = Σ_l a_l · l(x)` is the *phase polynomial* — a sum of linear
//! Boolean functions `l` weighted by integer coefficients `a_l` (mod 8,
//! since `T⁸ = I` up to global phase).
//!
//! ## T-count
//!
//! A coefficient `c` (mod 8) contributes:
//!
//! | `c mod 8` | Gate(s) | T-count |
//! |-----------|---------|---------|
//! | 0         | —       | 0       |
//! | 1         | T       | 1       |
//! | 2         | S       | 0       |
//! | 3         | S, T    | 1       |
//! | 4         | Z       | 0       |
//! | 5         | Z, T    | 1       |
//! | 6         | S†      | 0       |
//! | 7         | T†      | 1       |
//!
//! Even coefficients are Clifford (0 T); odd coefficients need exactly 1 T
//! or T†. Merging two T gates on the *same parity* (even if separated by
//! CNOTs) can halve the T-count.

use std::collections::{BTreeMap, HashSet};

// ---------------------------------------------------------------------------
// PhasePolynomial
// ---------------------------------------------------------------------------

/// A parity bitvector — a linear Boolean function over qubit inputs.
///
/// Backed by a dynamic bitset (`Vec<u64>`) so it supports arbitrary qubit
/// counts (well beyond the 128-qubit limit of a fixed `u128`). Trailing
/// zero words are trimmed, giving a canonical representation: two equal
/// parities compare, order, and hash identically regardless of how many
/// words were allocated.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Parity {
    /// Little-endian words: bit `i` lives in `bits[i / 64]` at position `i % 64`.
    bits: Vec<u64>,
}

impl Parity {
    /// Number of `u64` words needed to address `n` qubits (`ceil(n / 64)`).
    const fn width(n: usize) -> usize {
        (n + 63) / 64
    }

    /// The zero parity (no bits set).
    pub fn zero() -> Self {
        Self { bits: Vec::new() }
    }

    /// The `i`-th basis vector over `n` qubits: bit `i` set, all others clear.
    pub fn basis(n: usize, i: usize) -> Self {
        debug_assert!(i < n, "basis index {i} out of range for {n} qubits");
        let mut bits = vec![0u64; Self::width(n)];
        bits[i / 64] |= 1u64 << (i % 64);
        Self::normalized(bits)
    }

    /// Construct a parity from the low 128 bits of `value`, sized for `n` qubits.
    /// Bits beyond `n` are masked off by the fixed width allocation. Primarily
    /// used to build expected parities in tests.
    pub fn from_u128(n: usize, value: u128) -> Self {
        let mut bits = vec![0u64; Self::width(n)];
        if !bits.is_empty() {
            bits[0] = value as u64;
        }
        if bits.len() >= 2 {
            bits[1] = (value >> 64) as u64;
        }
        Self::normalized(bits)
    }

    /// XOR `other` into `self` in place, extending `self` if needed and
    /// re-trimming trailing zero words to keep the representation canonical.
    pub fn xor_in_place(&mut self, other: &Self) {
        if self.bits.len() < other.bits.len() {
            self.bits.resize(other.bits.len(), 0);
        }
        for (w, &o) in self.bits.iter_mut().zip(other.bits.iter()) {
            *w ^= o;
        }
        while let Some(&0) = self.bits.last() {
            self.bits.pop();
        }
    }

    /// Trim trailing zero words for a canonical representation.
    fn normalized(mut bits: Vec<u64>) -> Self {
        while let Some(&0) = bits.last() {
            bits.pop();
        }
        Self { bits }
    }
}

/// A phase polynomial over `n` qubits.
///
/// Maps each linear Boolean function (represented as a [`Parity`] bitvector
/// of qubit parities) to an integer coefficient in units of π/4 (mod 8).
#[derive(Clone, Debug)]
pub struct PhasePolynomial {
    /// Number of qubits.
    n: usize,
    /// Parity → coefficient (mod 8, range 0..8).
    terms: BTreeMap<Parity, u8>,
}

impl PhasePolynomial {
    /// Create an empty phase polynomial on `n` qubits.
    pub fn new(n: usize) -> Self {
        Self {
            n,
            terms: BTreeMap::new(),
        }
    }

    /// Number of qubits.
    pub fn n(&self) -> usize {
        self.n
    }

    /// Add `coeff` (in π/4 units) to the term with parity `p`.
    /// Coefficient is normalized mod 8; zero terms are removed.
    pub fn add_term(&mut self, p: Parity, coeff: i8) {
        let new_coeff = (self.coeff(&p) as i8 + coeff).rem_euclid(8) as u8;
        if new_coeff == 0 {
            self.terms.remove(&p);
        } else {
            self.terms.insert(p, new_coeff);
        }
    }

    /// Get the coefficient (mod 8) for parity `p`, or 0 if absent.
    pub fn coeff(&self, p: &Parity) -> u8 {
        self.terms.get(p).copied().unwrap_or(0)
    }

    /// Number of distinct non-zero phase terms.
    pub fn num_terms(&self) -> usize {
        self.terms.len()
    }

    /// T-count: number of terms with odd coefficient.
    pub fn t_count(&self) -> usize {
        self.terms.values().filter(|c| **c % 2 == 1).count()
    }

    /// Iterate over (parity, coefficient) pairs.
    pub fn terms(&self) -> impl Iterator<Item = (&Parity, u8)> + '_ {
        self.terms.iter().map(|(p, &c)| (p, c))
    }
}

/// Returns `true` if `name` is a CNOT+T block gate (CNOT, T, or T†).
pub fn is_block_gate(name: &str) -> bool {
    let canonical = quon_core::gates::canonical_id(name).unwrap_or(name);
    matches!(canonical, "CNOT" | "T" | "T_dag")
}

/// Returns `true` if `name` is T or T† (non-Clifford, contributes to T-count).
pub fn is_t_gate(name: &str) -> bool {
    let canonical = quon_core::gates::canonical_id(name).unwrap_or(name);
    matches!(canonical, "T" | "T_dag")
}

/// Count T/T† gates in a sequence.
pub fn count_t_gates(gates: &[(String, Vec<usize>)]) -> usize {
    gates.iter().filter(|(name, _)| is_t_gate(name)).count()
}

// ---------------------------------------------------------------------------
// Extraction
// ---------------------------------------------------------------------------

/// Extract the phase polynomial from a `{CNOT, T, T†}` circuit block.
///
/// Tracks parities through CNOTs: each T/T† on qubit `q` contributes a phase
/// term on the *current* parity of `q`, which may be a non-trivial linear
/// function of the input bits if CNOTs have acted on `q`.
///
/// Returns the phase polynomial.
pub fn extract(n: usize, gates: &[(String, Vec<usize>)]) -> PhasePolynomial {
    let mut poly = PhasePolynomial::new(n);
    // p[i] = current parity of qubit i, as a bitvector over input bits.
    let mut parity: Vec<Parity> = (0..n).map(|i| Parity::basis(n, i)).collect();

    for (name, qubits) in gates {
        let canonical = quon_core::gates::canonical_id(name).unwrap_or(name);
        match canonical {
            "CNOT" => {
                let c = qubits[0];
                let t = qubits[1];
                let pc = parity[c].clone();
                parity[t].xor_in_place(&pc);
            }
            "T" => {
                let q = qubits[0];
                poly.add_term(parity[q].clone(), 1);
            }
            "T_dag" => {
                let q = qubits[0];
                poly.add_term(parity[q].clone(), -1);
            }
            _ => {} // skip non-block gates
        }
    }
    poly
}

// ---------------------------------------------------------------------------
// Re-synthesis
// ---------------------------------------------------------------------------

/// Map a coefficient (mod 8) to the minimal T-count and the gate names to
/// emit on a single qubit.
///
/// Clifford gates (S, S†, Z) are emitted *before* the T/T† so the T gate
/// sits at the point where the parity is available.
fn coeff_to_gates(c: u8) -> Vec<&'static str> {
    match c {
        0 => vec![],
        1 => vec!["T"],
        2 => vec!["S"],
        3 => vec!["S", "T"],
        4 => vec!["Z"],
        5 => vec!["Z", "T"],
        6 => vec!["S_dag"],
        7 => vec!["T_dag"],
        _ => vec![],
    }
}

/// Re-synthesize a `{CNOT, T, T†}` circuit from the merged phase polynomial.
///
/// Walks the original CNOT network, maintaining parity tracking. At the
/// *first* occurrence of each parity (where a T/T† gate was placed), emits
/// the merged coefficient's gates. Subsequent T/T† gates on the same parity
/// are elided (already accounted for). CNOTs are preserved verbatim.
pub fn synthesize(
    poly: &PhasePolynomial,
    original: &[(String, Vec<usize>)],
    n: usize,
) -> Vec<(String, Vec<usize>)> {
    let mut result = Vec::new();
    let mut parity: Vec<Parity> = (0..n).map(|i| Parity::basis(n, i)).collect();
    let mut emitted: HashSet<Parity> = HashSet::new();

    for (name, qubits) in original {
        let canonical = quon_core::gates::canonical_id(name).unwrap_or(name);
        match canonical {
            "CNOT" => {
                result.push((name.clone(), qubits.clone()));
                let c = qubits[0];
                let t = qubits[1];
                let pc = parity[c].clone();
                parity[t].xor_in_place(&pc);
            }
            "T" | "T_dag" => {
                let q = qubits[0];
                let p = &parity[q];
                if !emitted.contains(p) {
                    emitted.insert(p.clone());
                    let coeff = poly.coeff(p);
                    for gate_name in coeff_to_gates(coeff) {
                        result.push((gate_name.to_string(), vec![q]));
                    }
                }
            }
            _ => {
                result.push((name.clone(), qubits.clone()));
            }
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Block splitting
// ---------------------------------------------------------------------------

/// A segment of the circuit: either a `{CNOT, T, T†}` block or a single
/// non-block gate.
#[derive(Debug)]
enum Segment {
    /// A maximal run of CNOT+T gates.
    Block(Vec<(String, Vec<usize>)>),
    /// A single gate that is not part of any CNOT+T block.
    Other(String, Vec<usize>),
}

/// Split a gate sequence into maximal `{CNOT, T, T†}` blocks and other gates.
fn split_into_blocks(gates: &[(String, Vec<usize>)]) -> Vec<Segment> {
    let mut segments = Vec::new();
    let mut current_block: Vec<(String, Vec<usize>)> = Vec::new();

    for (name, qubits) in gates {
        if is_block_gate(name) {
            current_block.push((name.clone(), qubits.clone()));
        } else {
            if !current_block.is_empty() {
                segments.push(Segment::Block(std::mem::take(&mut current_block)));
            }
            segments.push(Segment::Other(name.clone(), qubits.clone()));
        }
    }
    if !current_block.is_empty() {
        segments.push(Segment::Block(current_block));
    }
    segments
}

// ---------------------------------------------------------------------------
// Optimization
// ---------------------------------------------------------------------------

/// Optimize a gate sequence for T-count using phase polynomial merging.
///
/// Splits the circuit into maximal `{CNOT, T, T†}` blocks, applies phase
/// polynomial extraction + merging to each block, and re-synthesizes if
/// the T-count decreased. Non-block gates (H, S, etc.) are preserved and
/// act as block delimiters.
///
/// Returns `Some(new_gates)` if any T-count reduction was achieved, or
/// `None` if no improvement was possible.
pub fn optimize_t_count(
    gates: &[(String, Vec<usize>)],
    n: usize,
) -> Option<Vec<(String, Vec<usize>)>> {
    // Validate operands before any indexing into parity state: every qubit
    // operand must be within the declared width `n`, and CNOT/T/T† gates must
    // carry their expected arity. Malformed inputs decline optimization
    // (return `None`) rather than risk a panic deep in extraction/synthesis.
    for (name, qubits) in gates {
        let canonical = quon_core::gates::canonical_id(name).unwrap_or(name);
        let expected_arity = match canonical {
            "CNOT" => Some(2),
            "T" | "T_dag" => Some(1),
            _ => None,
        };
        if let Some(arity) = expected_arity
            && qubits.len() != arity
        {
            return None;
        }
        for &q in qubits {
            if q >= n {
                return None;
            }
        }
    }
    let segments = split_into_blocks(gates);
    let mut result = Vec::new();
    let mut changed = false;

    for seg in segments {
        match seg {
            Segment::Block(block_gates) => {
                let original_t = count_t_gates(&block_gates);
                let poly = extract(n, &block_gates);
                let new_t = poly.t_count();
                if new_t < original_t {
                    changed = true;
                    result.extend(synthesize(&poly, &block_gates, n));
                } else {
                    result.extend(block_gates);
                }
            }
            Segment::Other(name, qubits) => {
                result.push((name, qubits));
            }
        }
    }

    if changed { Some(result) } else { None }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a `Parity` from a small bit pattern (low 128 bits) for `n` qubits.
    fn p(n: usize, v: u128) -> Parity {
        Parity::from_u128(n, v)
    }

    // --- PhasePolynomial ---------------------------------------------------

    #[test]
    fn empty_poly_has_zero_t_count() {
        let poly = PhasePolynomial::new(2);
        assert_eq!(poly.t_count(), 0);
        assert_eq!(poly.num_terms(), 0);
    }

    #[test]
    fn add_and_merge_terms() {
        let mut poly = PhasePolynomial::new(2);
        poly.add_term(p(2, 0b01), 1); // T on parity {0}
        poly.add_term(p(2, 0b01), 1); // another T on same parity → coeff 2 (S)
        assert_eq!(poly.coeff(&p(2, 0b01)), 2);
        assert_eq!(poly.t_count(), 0); // even → 0 T gates
    }

    #[test]
    fn add_canceling_terms() {
        let mut poly = PhasePolynomial::new(1);
        poly.add_term(p(1, 1), 1); // T
        poly.add_term(p(1, 1), -1); // T†
        assert_eq!(poly.num_terms(), 0); // cancelled
    }

    #[test]
    fn coeff_mod_8() {
        let mut poly = PhasePolynomial::new(1);
        poly.add_term(p(1, 1), 1);
        poly.add_term(p(1, 1), 1);
        poly.add_term(p(1, 1), 1);
        assert_eq!(poly.coeff(&p(1, 1)), 3); // T³
        assert_eq!(poly.t_count(), 1); // odd → 1 T gate
    }

    // --- Extraction --------------------------------------------------------

    #[test]
    fn extract_single_t() {
        let gates = vec![("T".to_string(), vec![0usize])];
        let poly = extract(1, &gates);
        assert_eq!(poly.num_terms(), 1);
        assert_eq!(poly.coeff(&p(1, 1)), 1);
        assert_eq!(poly.t_count(), 1);
    }

    #[test]
    fn extract_t_t_same_parity() {
        // T(0), CNOT(0,1), T(0) — both T on same parity {0}
        let gates = vec![
            ("T".to_string(), vec![0usize]),
            ("CNOT".to_string(), vec![0, 1]),
            ("T".to_string(), vec![0]),
        ];
        let poly = extract(2, &gates);
        assert_eq!(poly.num_terms(), 1); // merged
        assert_eq!(poly.coeff(&p(2, 1)), 2); // T² = S
        assert_eq!(poly.t_count(), 0); // S is Clifford
    }

    #[test]
    fn extract_t_t_different_parity() {
        // T(1), CNOT(0,1), T(1) — second T has parity {0,1}
        let gates = vec![
            ("T".to_string(), vec![1usize]),
            ("CNOT".to_string(), vec![0, 1]),
            ("T".to_string(), vec![1]),
        ];
        let poly = extract(2, &gates);
        assert_eq!(poly.num_terms(), 2); // not merged
        assert_eq!(poly.coeff(&p(2, 0b10)), 1); // parity {1}
        assert_eq!(poly.coeff(&p(2, 0b11)), 1); // parity {0,1}
        assert_eq!(poly.t_count(), 2);
    }

    #[test]
    fn extract_t_dag_cancels_t() {
        let gates = vec![
            ("T".to_string(), vec![0usize]),
            ("T_dag".to_string(), vec![0]),
        ];
        let poly = extract(1, &gates);
        assert_eq!(poly.num_terms(), 0); // cancelled
        assert_eq!(poly.t_count(), 0);
    }

    #[test]
    fn extract_non_adjacent_cnot_cancellation() {
        // T(0), CNOT(0,1), CNOT(0,1), T(0) — CNOTs cancel, both T on {0}
        let gates = vec![
            ("T".to_string(), vec![0usize]),
            ("CNOT".to_string(), vec![0, 1]),
            ("CNOT".to_string(), vec![0, 1]),
            ("T".to_string(), vec![0]),
        ];
        let poly = extract(2, &gates);
        assert_eq!(poly.num_terms(), 1);
        assert_eq!(poly.coeff(&p(2, 1)), 2); // T² = S
        assert_eq!(poly.t_count(), 0);
    }

    // --- Re-synthesis ------------------------------------------------------

    #[test]
    fn synthesize_merges_t_pair_to_s() {
        let original = vec![
            ("T".to_string(), vec![0usize]),
            ("CNOT".to_string(), vec![0, 1]),
            ("T".to_string(), vec![0]),
        ];
        let poly = extract(2, &original);
        let result = synthesize(&poly, &original, 2);
        // Should emit S(0), CNOT(0,1) — T-count 0
        assert_eq!(count_t_gates(&result), 0);
        assert!(result.iter().any(|(name, _)| name == "S"));
        assert!(result.iter().any(|(name, _)| name == "CNOT"));
    }

    #[test]
    fn synthesize_preserves_different_parities() {
        let original = vec![
            ("T".to_string(), vec![1usize]),
            ("CNOT".to_string(), vec![0, 1]),
            ("T".to_string(), vec![1]),
        ];
        let poly = extract(2, &original);
        let result = synthesize(&poly, &original, 2);
        // Two different parities — no merging
        assert_eq!(count_t_gates(&result), 2);
    }

    #[test]
    fn synthesize_three_t_becomes_one() {
        // T(0), CNOT(0,1), T(0), CNOT(0,1), T(0) — all on parity {0}
        let original = vec![
            ("T".to_string(), vec![0usize]),
            ("CNOT".to_string(), vec![0, 1]),
            ("T".to_string(), vec![0]),
            ("CNOT".to_string(), vec![0, 1]),
            ("T".to_string(), vec![0]),
        ];
        let poly = extract(2, &original);
        assert_eq!(poly.coeff(&p(2, 1)), 3); // T³
        let result = synthesize(&poly, &original, 2);
        assert_eq!(count_t_gates(&result), 1); // reduced from 3 to 1
    }

    // --- Block splitting ---------------------------------------------------

    #[test]
    fn split_single_block() {
        let gates = vec![
            ("T".to_string(), vec![0usize]),
            ("CNOT".to_string(), vec![0, 1]),
        ];
        let segs = split_into_blocks(&gates);
        assert_eq!(segs.len(), 1);
    }

    #[test]
    fn split_with_h_delimiter() {
        let gates = vec![
            ("T".to_string(), vec![0usize]),
            ("H".to_string(), vec![0]),
            ("T".to_string(), vec![0]),
        ];
        let segs = split_into_blocks(&gates);
        assert_eq!(segs.len(), 3); // block, other, block
    }

    // --- optimize_t_count --------------------------------------------------

    #[test]
    fn optimize_reduces_non_adjacent_t() {
        // T(0), CNOT(0,1), T(0) → T-count 2 → 0
        let gates = vec![
            ("T".to_string(), vec![0usize]),
            ("CNOT".to_string(), vec![0, 1]),
            ("T".to_string(), vec![0]),
        ];
        let result = optimize_t_count(&gates, 2);
        assert!(result.is_some());
        let optimized = result.unwrap();
        assert_eq!(count_t_gates(&optimized), 0);
    }

    #[test]
    fn optimize_does_not_reduce_t_h_t() {
        // T(0), H(0), T(0) — H splits blocks, no merging possible
        let gates = vec![
            ("T".to_string(), vec![0usize]),
            ("H".to_string(), vec![0]),
            ("T".to_string(), vec![0]),
        ];
        let result = optimize_t_count(&gates, 1);
        assert!(result.is_none()); // no improvement
    }

    #[test]
    fn optimize_reduces_three_t_to_one() {
        let gates = vec![
            ("T".to_string(), vec![0usize]),
            ("CNOT".to_string(), vec![0, 1]),
            ("T".to_string(), vec![0]),
            ("CNOT".to_string(), vec![0, 1]),
            ("T".to_string(), vec![0]),
        ];
        let result = optimize_t_count(&gates, 2);
        assert!(result.is_some());
        assert_eq!(count_t_gates(&result.unwrap()), 1);
    }

    #[test]
    fn optimize_preserves_non_block_gates() {
        let gates = vec![
            ("T".to_string(), vec![0usize]),
            ("CNOT".to_string(), vec![0, 1]),
            ("T".to_string(), vec![0]),
            ("H".to_string(), vec![1]),
        ];
        let result = optimize_t_count(&gates, 2);
        assert!(result.is_some());
        let optimized = result.unwrap();
        // H gate should be preserved
        assert!(optimized.iter().any(|(name, _)| name == "H"));
        assert_eq!(count_t_gates(&optimized), 0);
    }

    #[test]
    fn optimize_empty_circuit() {
        let gates: Vec<(String, Vec<usize>)> = vec![];
        let result = optimize_t_count(&gates, 1);
        assert!(result.is_none());
    }

    #[test]
    fn optimize_no_t_gates() {
        let gates = vec![("CNOT".to_string(), vec![0usize, 1])];
        let result = optimize_t_count(&gates, 2);
        assert!(result.is_none());
    }

    // --- Wide-circuit safety (issue #392) ---------------------------------

    #[test]
    fn optimize_width_zero_empty() {
        let gates: Vec<(String, Vec<usize>)> = vec![];
        assert!(optimize_t_count(&gates, 0).is_none());
    }

    #[test]
    fn optimize_width_one_single_t() {
        // Single T on a 1-qubit circuit: no merging possible → no improvement.
        let gates = vec![("T".to_string(), vec![0usize])];
        assert!(optimize_t_count(&gates, 1).is_none());
    }

    #[test]
    fn extract_width_128_no_overflow() {
        // Bit 127 is the top bit of a u128; under the old representation
        // `1u128 << 127` worked but left no headroom. Verify extraction still
        // produces a single term at the basis parity for qubit 127.
        let gates = vec![("T".to_string(), vec![127usize])];
        let poly = extract(128, &gates);
        assert_eq!(poly.num_terms(), 1);
        assert_eq!(poly.coeff(&Parity::basis(128, 127)), 1);
        assert_eq!(poly.t_count(), 1);
    }

    #[test]
    fn optimize_width_128_single_t() {
        let gates = vec![("T".to_string(), vec![127usize])];
        assert!(optimize_t_count(&gates, 128).is_none()); // single T, no reduction
    }

    #[test]
    fn extract_width_129_no_panic() {
        // 129 qubits overflows the old u128 basis (`1u128 << 128 == 0`).
        // The dynamic Parity bitset must handle qubit 128 without panic.
        let gates = vec![("T".to_string(), vec![128usize])];
        let poly = extract(129, &gates);
        assert_eq!(poly.num_terms(), 1);
        assert_eq!(poly.coeff(&Parity::basis(129, 128)), 1);
        assert_eq!(poly.t_count(), 1);
    }

    #[test]
    fn optimize_width_129_merges_t_pair() {
        // T(128), CNOT(128,0), T(128) — both T on parity {128}; merges to S.
        // Proves optimization still reduces T-count beyond 128 qubits.
        let gates = vec![
            ("T".to_string(), vec![128usize]),
            ("CNOT".to_string(), vec![128, 0]),
            ("T".to_string(), vec![128]),
        ];
        let result = optimize_t_count(&gates, 129);
        assert!(result.is_some());
        assert_eq!(count_t_gates(&result.unwrap()), 0);
    }

    #[test]
    fn optimize_out_of_range_t_operand() {
        // T on qubit 130 in a 129-qubit circuit → decline optimization.
        let gates = vec![("T".to_string(), vec![130usize])];
        assert!(optimize_t_count(&gates, 129).is_none());
    }

    #[test]
    fn optimize_out_of_range_cnot_operand() {
        // CNOT targeting qubit 130 in a 129-qubit circuit → decline.
        let gates = vec![
            ("T".to_string(), vec![0usize]),
            ("CNOT".to_string(), vec![0, 130]),
        ];
        assert!(optimize_t_count(&gates, 129).is_none());
    }

    #[test]
    fn optimize_malformed_cnot_arity() {
        // CNOT with a single operand would index out of bounds in extraction;
        // the entry-point validation must decline instead of panicking.
        let gates = vec![("CNOT".to_string(), vec![0usize])];
        assert!(optimize_t_count(&gates, 2).is_none());
    }

    // --- coeff_to_gates ----------------------------------------------------

    #[test]
    fn coeff_to_gates_t_count() {
        for c in 0u8..8 {
            let gates = coeff_to_gates(c);
            let t_count = gates
                .iter()
                .filter(|g| **g == "T" || **g == "T_dag")
                .count();
            if c % 2 == 0 {
                assert_eq!(t_count, 0, "even coeff {c} should have 0 T gates");
            } else {
                assert_eq!(t_count, 1, "odd coeff {c} should have 1 T gate");
            }
        }
    }
}
