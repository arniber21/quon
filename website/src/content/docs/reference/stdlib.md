---
title: Circuit stdlib
description: Experimental Quon circuit modules compiled with quonc --include.
---

The circuit stdlib is a set of ordinary `.qn` files under `stdlib/` for QFT,
fixed phase oracles, unweighted QAOA layers, open-chain Ising Trotter steps,
and 1-bit phase estimation. They are **experimental**: names and depth bounds
may change with no deprecation window.

Quon has no `import`. `--include` concatenates the listed files in front of
the entry source and typechecks one program with a flat scope. The design note
is [`stdlib/README.md`](https://github.com/arniber21/quon/blob/main/stdlib/README.md).

```bash
quonc --include stdlib/qft.qn --emit-qasm samples/algorithms/stdlib_qft_roundtrip.qn
```

| Module | Role |
| --- | --- |
| `stdlib/qft.qn` | Parametric `qft` and `qft_roundtrip` |
| `stdlib/oracles.qn` | Fixed Bernstein–Vazirani, Deutsch–Jozsa, and Grover phase oracles |
| `stdlib/qaoa.qn` | Unweighted cost layer, mixer, and one QAOA layer |
| `stdlib/ising.qn` | Open-chain Trotter fragments and `ising_evolve` |
| `stdlib/phase_estimation.qn` | 1-bit QPE and a CZ phase-kickback block |

Samples that compile each module are listed in `stdlib/README.md`. Oracles are
fixed circuits, not classical functions of a secret. QAOA here is unweighted
and one layer. Ising uses `repeat` on an open chain. Phase estimation is one
counting qubit.

The compiler prelude (`H`, `identity`, `adjoint`, `measure`, and the rest of
SPEC §5) is separate. Those names are intrinsics, not files in `stdlib/`.
