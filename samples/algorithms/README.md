# Algorithm samples

Narrative walkthroughs of named quantum algorithms and canonical circuit
constructions — building on `learning/`'s primitives, but each still a
complete, runnable program.

## Status

No dedicated pack — contributions are welcome directly against this
category. Prefer a new algorithm/construction over duplicating one that
already has a canonical home in `test/verify/` (link it from your README
section instead of forking the `.qn`).

## Seeds

| Catalog id | Path | Concept |
| --- | --- | --- |
| `algorithms/ghz-state` | [`ghz_state.qn`](./ghz_state.qn) | n-qubit GHZ entanglement (generalizes `learning/hello-bell`'s Bell pair) |
| `algorithms/deutsch-jozsa` | [`deutsch_jozsa.qn`](./deutsch_jozsa.qn) | Deutsch-Jozsa: constant vs balanced oracle in a single query (textbook #186) |
| `algorithms/simon` | [`simon.qn`](./simon.qn) | Simon's algorithm: hidden string recovery with GF(2) classical post-processing (textbook #186) |
| `algorithms/phase-estimation` | [`phase_estimation.qn`](./phase_estimation.qn) | QPE: estimate eigenvalue phase of Rz(2*pi) with a single counting qubit (textbook #186) |
| `algorithms/stdlib-qft-roundtrip` | [`stdlib_qft_roundtrip.qn`](./stdlib_qft_roundtrip.qn) | QFT then its adjoint at n=3, definition from experimental `stdlib/qft.qn` (#195) |
| `algorithms/stdlib-qft-n2` | [`stdlib_qft_n2.qn`](./stdlib_qft_n2.qn) | `qft(2)` instantiation of `stdlib/qft.qn` (#195) |
| `algorithms/stdlib-bernstein-vazirani` | [`stdlib_bernstein_vazirani.qn`](./stdlib_bernstein_vazirani.qn) | BV secret 101 around `stdlib/oracles.qn` (#195) |
| `algorithms/stdlib-grover-mark` | [`stdlib_grover_mark.qn`](./stdlib_grover_mark.qn) | Hadamards plus the stdlib mark of basis state 11 (#195) |
| `algorithms/stdlib-qpe-1bit` | [`stdlib_qpe_1bit.qn`](./stdlib_qpe_1bit.qn) | 1-bit QPE from `stdlib/phase_estimation.qn` (#195) |
| `algorithms/stdlib-phase-kickback` | [`stdlib_phase_kickback.qn`](./stdlib_phase_kickback.qn) | Counting-qubit kickback block from `stdlib/phase_estimation.qn` (#195) |

For deeper algorithm fixtures already verified end-to-end on Aer, see
[`test/verify/`](../../test/verify/) (Grover, QFT, Bernstein–Vazirani, Shor,
Ising, QAOA) and the [website cookbook](../../website/src/content/docs/cookbook/).

## Adding a seed

See [`../CONTRIBUTING.md`](../CONTRIBUTING.md). Every new file here needs a
matching row in [`../catalog.yaml`](../catalog.yaml).
