# Real Clifford+T optimization: phase polynomials + stabilizer tableaux (#96)

Implementation of the two algorithms specified in ADR-0013, shipped in #96.

## Phase polynomial (Universal / `clifford = false`)

The non-Clifford content of a `{CNOT, T, T†}` circuit block is extracted as a
sum of linear Boolean phase terms — a *phase polynomial* over GF(2). Each T/T†
gate contributes `±1` (in π/4 units) to the linear function equal to the
*current parity* of its qubit, which may be a non-trivial XOR of input bits
after CNOTs have acted. Terms with the same parity are merged (coefficients
summed mod 8); even coefficients become Clifford gates (S, S†, Z, —), reducing
T-count without any adjacency requirement.

**Re-synthesis** walks the original CNOT network, maintaining parity tracking,
and emits the merged coefficient's gates at the first occurrence of each parity.
Subsequent T/T† on the same parity are elided. CNOTs are preserved verbatim;
gate_cancellation in the fixpoint catches any redundant CNOT pairs that emerge.

H, S, and other gates outside `{CNOT, T, T†}` act as block delimiters — the
circuit is split into maximal CNOT+T blocks, each optimized independently. This
correctly handles `T·H·T` (two single-T blocks, no merging) while still
enabling non-adjacent reduction like `T·CNOT·T → S·CNOT` (T-count 2→0).

## Stabilizer tableau (Clifford / `clifford = true`)

An n-qubit Clifford operation is simulated via its conjugation action on Pauli
generators, represented as a `(2n) × (2n+1)` binary tableau (CHP
representation, Aaronson & Gottesman 2004). Supported gates: H, S, S†, CNOT,
X, Y, Z, SWAP.

After conjugating the identity tableau through the entire gate sequence:

- **Identity** → all gates removed (no-op). This catches non-adjacent
  identities like `S·S·S·S = I` or `H·S⁴·H = I` that the peephole
  gate_cancellation pass cannot see (S is not self-inverse; the H gates are
  separated by four S gates).
- **Single Pauli** → the sequence is replaced by the Pauli as a minimal gate
  list (e.g., `S·S → Z`).
- **Non-trivial Clifford** → no change.

## Packaging

Per ADR-0013, the algorithms are MLIR-free (pure Rust) so they are testable
without a Context. They live in `mlir_bridge/src/passes/` as
`stabilizer_tableau.rs` and `phase_polynomial.rs`. The MLIR glue
(extract/rebuild) is in `clifford_t_opt.rs`, which walks `quantum.circ.func`
bodies, extracts a flat gate list, dispatches to the appropriate algorithm,
and rebuilds the block if the kernel reports a reduction.

## Pipeline

The circ fixpoint order is now `gate_cancellation` → `rotation_merging` →
**`clifford_t_opt`** → `compiler_uncomputation` → `zx_simplification`
(ADR-0013). The fixpoint ensures that T-count reductions enabling further
peephole cancellations (e.g., adjacent CNOT pairs from re-synthesis) are
caught in subsequent rounds.

## Limitations / future work

- **Hadamard gadgetization** (eliminating internal H gates via T-gadgets) is
  not implemented; circuits with internal H gates are split at H boundaries.
- **CNOT network optimization** is not part of the phase polynomial pass;
  redundant CNOTs are caught by gate_cancellation in the fixpoint.
- **Reed-Muller exact synthesis** (Amy-Mosca) for optimal T-count on small
  CI instances is not implemented; the current re-synthesis uses first-fit
  parity merging.
- The `clifford_t` crate proposed in ADR-0013 was not created; the algorithms
  live in `mlir_bridge/src/passes/` as specified by issue #96's architecture.
