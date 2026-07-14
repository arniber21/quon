# Clifford+T optimization: Reed–Muller T-count + canonical tableaux (#96)

Story 25 / SPEC §7.2 described Clifford+T optimization as “Todd algorithm structure” for Universal circuits and Aaronson–Gottesman tableaux for Clifford circuits. After design grilling for #96 we are building a **fault-tolerant T-count compiler** (not an AC-minimal peephole), shipping **both** branches in #96, and deliberately diverging from TODD as the Universal kernel.

**Universal path:** normalize exact `Rz(k·π/4)` into discrete Clifford+T; eliminate internal Hadamards via gadgetization; rewrite to `Clifford_in · (CNOT+T) · Clifford_out`; optimize the middle with **Amy–Mosca Reed–Muller / phase-polynomial** methods (exact on small CI instances, best-effort decoder heuristics in general); resynthesize. Declining a stage leaves the whole `quantum.circ.func` unchanged. Primary objective is **T-count**; depth and non-T gate count may increase, and the func `depth` attribute is recomputed (unlike peephole Flux lemmas that assume depth-non-increasing).

**Clifford path:** Aaronson–Gottesman tableau simulation with **canonical Clifford resynthesis** (CHP generators `{CNOT,H,S}` inside the kernel; registry Cliffords on emit). On Universal funcs, also canonize maximal Clifford layers (initial/final blocks), not only `clifford=true` funcs. If Universal opt removes all `T`/`T†`, re-infer and may flip `clifford` to true.

**Ancillas / borrow:** gadget ancillas are emitted only as `quantum.circ.borrow`. The optimizer may leave **open reversible** borrow bodies; `compiler_uncomputation` closes them in the fixpoint (structure over pass-author memory). Post-fixpoint IR must still satisfy borrow/linearity checks.

**Pipeline:** circ fixpoint order is `gate_cancellation` → `rotation_merging` → **`clifford_t_opt`** → `compiler_uncomputation` → `zx_simplification`. ZX interaction after faithful multi-qubit extraction (#75) is deferred; do not special-case ZX inside #96.

**Packaging:** Melior-free kernel in a new **`clifford_t`** crate (modules for tableau, phase polynomial / RM, Hadamard gadgetization, synthesis). `mlir_bridge` extracts/rebuilds only. Validation: equiv harness + props, Aer/statevector on acceptance fixtures, Flux only on structural safety envelopes—not RM optimality proofs.

## Considered Options

- **TODD (Heyfron–Campbell) or Amy T-par as the Universal kernel** — rejected for #96 in favor of Reed–Muller / Amy–Mosca; SPEC “Todd” wording should be updated to match.
- **Ancilla-free or pass-private scratch wires** — rejected; ancillas must be `borrow`-shaped.
- **Self-contained borrow uncompute inside `clifford_t_opt`** — rejected; fixpoint `compiler_uncomputation` owns closing open reversible borrows.
- **Depth-non-increasing Universal rewrites** — rejected; incompatible with FT T-count reduction.

## Consequences

- SPEC §7.1–7.2 and §13 were updated to match this ADR (RM + AG canon; fixpoint order; no TODD-as-kernel claim).
- `quon_core::optimization` depth-non-increasing Flux lemmas remain for peepholes only; they do not constrain `clifford_t_opt`.
- #75 may later force a ZX vs T-count policy; tracked as a known revisit, not part of this ADR’s pass logic.
