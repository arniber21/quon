---
title: Application demos
description: Application-domain Quon demos — optimization (QAOA/MaxCut, VQE), simulation (Ising), and a TSP sketch — and what each leaves to classical Python.
---

Quon is a circuit language, not an optimization framework. The application
demos under [`samples/applications/`](https://github.com/arniber21/quon/tree/main/samples/applications)
(issue #191) make that boundary explicit: each one is a `.qn` circuit — the
*quantum* half of a hybrid algorithm — paired with a classical outer loop in
Python. The `.qn` lowers and typechecks; the Python does the parts Quon has no
business doing (parameter optimization, cost evaluation, route decoding).

This page summarizes the demos. Each is also documented in
[`samples/applications/README.md`](https://github.com/arniber21/quon/blob/main/samples/applications/README.md)
and verified by a seeded checker under
[`test/verify/`](https://github.com/arniber21/quon/tree/main/test/verify).

## What is Quon vs. what is classical

Across every demo the split is the same:

- **Quon (the `.qn` circuit).** Prepares a parameterized quantum state — a
  QAOA cost+mixer ansatz, a Trotterized Ising evolution, or a VQE hardware
  ansatz — and proves a compile-time depth bound via the `Circuit<Q, N, D, F>`
  type. The variational angles are baked in as literals.
- **Classical Python (the checker / outer loop).** Picks the angles (a
  statevector sweep / classical optimizer), evaluates the objective from
  measurement data, and does any problem-specific decoding (e.g. decoding a TSP
  bitstring to a city tour, then 2-opt). Quon has none of this.

## The demos

### MaxCut QAOA

MaxCut on the 6-vertex triangular prism (3-regular, MaxCut = 7) via one QAOA
layer (`Rzz(gamma)` cost edges + `Rx(beta)` mixer on a Hadamard state).
[`maxcut_prism6.qn`](https://github.com/arniber21/quon/blob/main/samples/applications/maxcut_prism6.qn)
proves depth 11; the checker
[`maxcut_prism6.py`](https://github.com/arniber21/quon/blob/main/test/verify/maxcut_prism6.py)
runs it on Aer and asserts the expected cut ≥ 0.8 × MaxCut and that the
most-probable bitstring is an optimal cut.

### QAOA depth scaling

The same C5 graph at p=1 and p=2 to show the depth/quality tradeoff directly.
[`maxcut_c5_p1.qn`](https://github.com/arniber21/quon/blob/main/samples/applications/maxcut_c5_p1.qn)
(depth 7) reaches expected cut ~3.75; the p=2 companion
[`maxcut_c5_p2.qn`](https://github.com/arniber21/quon/blob/main/samples/applications/maxcut_c5_p2.qn)
(depth 13) closes the gap to the optimum (~4.0). The checker
[`maxcut_depth.py`](https://github.com/arniber21/quon/blob/main/test/verify/maxcut_depth.py)
compiles **both** and asserts p=2 ≥ p=1.

### Ising on a ring

The transverse-field Ising model on a ring (periodic boundary), extending the
open-chain [`ising.qn`](https://github.com/arniber21/quon/blob/main/test/verify/ising.qn)
fixture with the closing bond. Trotter parameters (J, h, t, n_steps) are Quon
`Float`/`Int` params, partial-evaluated into rotation angles.
[`ising_ring.qn`](https://github.com/arniber21/quon/blob/main/samples/applications/ising_ring.qn)
+ [`ising_ring.py`](https://github.com/arniber21/quon/blob/main/test/verify/ising_ring.py),
which checks the t = 0 identity (all-zeros) oracle.

### VQE ansatz

A hardware-efficient Ry/CNOT ansatz for a 2-qubit model Hamiltonian (ground
energy −1.400). The whole VQE outer loop — energy evaluation, the optimizer,
Pauli grouping — is classical; Quon only lowers the ansatz.
[`vqe_ansatz.qn`](https://github.com/arniber21/quon/blob/main/samples/applications/vqe_ansatz.qn)
+ [`vqe_ansatz.py`](https://github.com/arniber21/quon/blob/main/test/verify/vqe_ansatz.py),
which extracts the statevector from the compiled circuit, computes ⟨H⟩ exactly,
and checks it equals the ground energy (plus a seeded Aer consistency check).
A SKETCH of VQE structure, not a chemistry-accuracy claim.

### Toy TSP sketch

A schematic: a small TSP-*shaped* cost Hamiltonian (weighted `Rzz` couplings +
`Rz` penalty fields) on 4 qubits in one QAOA layer — the same circuit shape a
TSP-to-Ising reformulation emits. Tour decoding, constraint enforcement, and
2-opt are classical and live outside Quon.
[`tsp_sketch.qn`](https://github.com/arniber21/quon/blob/main/samples/applications/tsp_sketch.qn)
+ [`tsp_sketch.py`](https://github.com/arniber21/quon/blob/main/test/verify/tsp_sketch.py)
(structural: compiles + shaped, parseable QASM — not a TSP solver).

## Reproducing

Build the compiler, then run any checker:

```sh
cargo build --release -p quonc
QUONC=target/release/quonc python test/verify/maxcut_prism6.py
```

Every `ci: smoke` catalog entry is also compiled with `quonc` in CI (the
[`samples_catalog`](https://github.com/arniber21/quon/blob/main/quonc/tests/samples_catalog.rs)
test); the Aer checkers above are seeded for reproducibility.
