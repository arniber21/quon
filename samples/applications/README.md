# Application samples

Programs framed around a real-world use case (optimization, simulation) rather
than a named textbook algorithm -- the "why would I use this" counterpart to
`algorithms/`. This pack (issue #191, parent epic #184) adds application-domain
demos that pair a Quon circuit with a classical outer loop, and is explicit
about which half of each demo is Quon and which is classical Python.

## Status

No dedicated pack -- contributions are welcome directly against this category.
The #191 demos below were landed together; see each entry's `.qn` header and
the matching `test/verify/*.py` checker for the Quon-vs-classical split.

## Seeds

| Catalog id | Path | Use case | Quantitative check |
| --- | --- | --- | --- |
| `applications/quantum-coin-flip` | [`quantum_coin_flip.qn`](./quantum_coin_flip.qn) | Genuine (non-pseudo) random bit generation | -- |
| `applications/maxcut-qaoa` | [`maxcut_prism6.qn`](./maxcut_prism6.qn) | MaxCut on a 6-vertex 3-regular graph via p=1 QAOA | Aer: expected cut >= 0.8*MaxCut, mode is optimal |
| `applications/qaoa-depth-p1` | [`maxcut_c5_p1.qn`](./maxcut_c5_p1.qn) | QAOA depth-scaling baseline (C5, p=1) | Aer (paired with p=2 below) |
| `applications/qaoa-depth-p2` | [`maxcut_c5_p2.qn`](./maxcut_c5_p2.qn) | QAOA depth-scaling deeper ansatz (C5, p=2) | Aer: p=2 expected cut >= p=1 |
| `applications/ising-ring` | [`ising_ring.qn`](./ising_ring.qn) | Transverse-field Ising on a ring (periodic boundary) | Aer: t=0 identity -> all zeros |
| `applications/vqe-ansatz` | [`vqe_ansatz.qn`](./vqe_ansatz.qn) | VQE-shaped ansatz for a 2-qubit model Hamiltonian | statevector: <H> = ground energy -1.400 |
| `applications/tsp-sketch` | [`tsp_sketch.qn`](./tsp_sketch.qn) | Toy TSP sketch: a QAOA-shaped cost circuit | structural: compiles + shaped/parseable QASM |

## Demos (#191)

Each demo follows the same shape: a `.qn` circuit (the Quon half) plus a
classical outer loop (Python). The split is called out per demo.

### MaxCut QAOA (`maxcut_prism6.qn`)

- **Problem.** MaxCut on the triangular prism -- a 3-regular graph on 6 vertices
  with 9 edges; its MaxCut is 7 (the two triangles force two uncut edges).
- **Encoding.** The cost Hamiltonian H_C = sum (1 - Z_i Z_j)/2 is one QAOA cost
  layer of `Rzz(gamma)` over the 9 edges, followed by an `Rx(beta)` transverse
  mixer, on a Hadamard initial state. One layer = p=1.
- **Quon part.** Lowers the parametric ansatz and proves its depth bound
  (1 + (9 + 1) = 11). The 9 edges are spelled out (no Matrix/list instance data
  in the e2e path yet), exactly the pattern `test/verify/qaoa_graph.qn` uses.
- **Classical part.** The angles (gamma, beta) are variational: a classical
  optimizer sweeps them to maximize the expected cut. Quon has no optimizer, so
  the angles were found offline by a statevector sweep and baked in as literals.
- **Run / interpret.** `QUONC=target/debug/quonc python test/verify/maxcut_prism6.py`
  samples the distribution on Aer and checks the expected cut >= 5.6 and that the
  most-probable bitstring is an optimal (cut = 7) cut.

### QAOA depth scaling (`maxcut_c5_p1.qn`, `maxcut_c5_p2.qn`)

- **Problem.** MaxCut on the 5-cycle C5 (MaxCut = 4, 10 optimal bitstrings),
  comparing one QAOA layer (p=1) against two (p=2) on the **same** graph.
- **Encoding.** Identical primitives to the prism demo (Hadamard, `Rzz(gamma)`
  ring cost, `Rx(beta)` mixer); p=2 composes two layers at the run-block site.
- **Quon part.** Proves depth 7 for p=1 and 13 for p=2 -- the depth bound
  roughly doubles with an extra layer, available at compile time.
- **Classical part.** Four angles for p=2 (and two for p=1) were found offline
  and baked in; the classical optimization is external Python.
- **Run / interpret.** `python test/verify/maxcut_depth.py` compiles **both**
  files, reports each expected cut, and asserts p=2 (4.0) >= p=1 (~3.75) -- the
  empirical "more depth, more quality" tradeoff, with the depth ratio printed.

### Ising on a ring (`ising_ring.qn`)

- **Problem.** Simulate the transverse-field Ising model H = -J sum Z_i Z_j -
  h sum X_i via first-order Trotterization, on a ring (periodic boundary).
- **Encoding.** Extends `test/verify/ising.qn`'s open chain with the closing
  bond (5, 0): a `zz_ring` of six `Rzz(-2 J tau)` gates composed with an
  `x_layer` of `Rx(-2 h tau)`, `repeat`-ed `n_steps` times. The Trotter
  parameters (J, h, t, n_steps) are genuine Quon `Float`/`Int` params,
  partial-evaluated into the rotation angles via `let tau = t / float(n_steps)`.
- **Quon part.** Lowers the Trotterized evolution and proves depth n_steps * 7.
- **Classical part.** Choosing J, h, t, n_steps to control Trotter error is a
  classical modelling decision; Quon evaluates the angles but does not pick them.
- **Run / interpret.** `python test/verify/ising_ring.py` runs at t = 0, where
  tau = 0 makes every rotation the identity, so measuring |000000> must give all
  zeros -- the t=0 boundary oracle, applied to the new ring topology.

### VQE ansatz (`vqe_ansatz.qn`)

- **Problem.** Approximate the ground state of a 2-qubit model Hamiltonian
  H = 0.5 Z0 + 0.3 Z1 + 0.6 X0 X1 - 0.4 Z0 Z1 (ground energy E0 = -1.400; the
  X0 X1 term makes the ground state a genuine superposition).
- **Encoding.** A hardware-efficient ansatz: two Ry layers bracketing a CNOT
  entangler, with the optimized angles baked in.
- **Quon part.** Lowers the ansatz and proves its depth bound.
- **Classical part.** The entire VQE outer loop is classical Python: evaluating
  <H> from measurement data, the parameter optimizer, and Pauli-term grouping.
- **Run / interpret.** `python test/verify/vqe_ansatz.py` extracts the trial
  state's statevector from the compiled circuit, computes <H> exactly, and
  asserts it equals E0 = -1.400, plus a seeded Aer cross-check that the
  measurement distribution matches the state. This is a SKETCH of VQE
  structure, not a claim of chemical accuracy.

### Toy TSP sketch (`tsp_sketch.qn`)

- **Problem.** The Travelling Salesman Problem. A full encoding is n^2 qubits
  plus heavy penalties; this is a **schematic**, not a real TSP solver.
- **Encoding.** A small TSP-*shaped* cost Hamiltonian on 4 qubits -- weighted
  `Rzz` pair couplings (inter-"city" distances) plus `Rz` local
  constraint-penalty fields -- in one QAOA cost+mixer layer. Same machinery as
  MaxCut/Ising; the point is the circuit shape is identical.
- **Quon part.** Lowers the cost+mixer layer and proves its depth bound.
- **Classical part.** Everything TSP-specific is classical Python and outside
  Quon: decoding a bitstring to a city ordering, enforcing the "visit each city
  once" constraint, and the 2-opt / Lin-Kernighan refinement loop.
- **Run / interpret.** `python test/verify/tsp_sketch.py` verifies the circuit
  compiles and emits a properly-shaped, parseable OpenQASM workload (gate counts
  + Qiskit ingestion). It makes NO claim about solving TSP.

## Adding a seed

See [`../CONTRIBUTING.md`](../CONTRIBUTING.md). Every new file here needs a
matching row in [`../catalog.yaml`](../catalog.yaml). A demo that claims a
numerical result must also wire a seeded checker under [`test/verify/`](../../test/verify/),
following the ising.py / qaoa.py pattern.
