---
title: Creative & games samples
description: Playful Quon programs — a quantum die, an entangled-twins magic trick, and an interference barcode — each teaching one concept while a seeded Aer checker referees the game.
---

Quon is a typed circuit language, but the concepts it teaches are also, at
heart, fun: superposition is a fair coin, entanglement is a mind-reading trick,
and interference draws patterns. The creative / games pack under
[`samples/creative/`](https://github.com/arniber21/quon/tree/main/samples/creative)
(issue #200) leans into that. Each sample is a small `.qn` circuit paired with a
seeded Aer checker that doubles as the game referee — it runs the circuit many
times and asserts the playable claim. No quantum-supremacy rhetoric: the point
is that each concept is *visible* in statistics a beginner can run.

As with the [application demos](./applications/), the split is explicit: Quon
prepares the state and measures; classical Python plays the game (distribution
check, ASCII render).

## The samples

### Quantum dice — a fair 2ⁿ-sided die

A Hadamard on each of `n` qubits, placed in a single depth-1 parallel layer
(`for q in qubits(n) { H q }`), measures to a uniformly random `n`-bit string —
a face of a `2ⁿ`-sided die. The randomness is genuine (the Born rule with equal
amplitudes), not a pseudo-random seed.
[`quantum_dice.qn`](https://github.com/arniber21/quon/blob/main/samples/creative/quantum_dice.qn)
+ [`quantum_dice.py`](https://github.com/arniber21/quon/blob/main/test/verify/quantum_dice.py),
which asserts all 8 faces appear, each near 1/8 of the shots.

**Concept:** superposition + parallel composition + the Born rule. The
many-qubit cousin of the single-qubit
[coin flip](https://github.com/arniber21/quon/blob/main/samples/applications/quantum_coin_flip.qn).

### Entangled twins — predict Bob's bits

Two Bell pairs with *opposite* correlations in one program: pair (q0, q1) is
|Φ⁺⟩ — the bits always **agree**; pair (q2, q3) is |Ψ⁺⟩ — the bits always
**disagree**. Alice holds q0 and q2 and announces Bob's bits in full, correctly,
every shot. The lesson: entanglement is not "the bits are the same" but "the
bits are in a *determined relation*," and whether that relation is same or
opposite is set entirely by how the pair was prepared (an `X` on the CNOT target
flips the correlation).
[`entangled_twins.qn`](https://github.com/arniber21/quon/blob/main/samples/creative/entangled_twins.qn)
+ [`entangled_twins.py`](https://github.com/arniber21/quon/blob/main/test/verify/entangled_twins.py),
which asserts b0==b1 and b2!=b3 on every shot.

**Concept:** entanglement, as a correlation locked in at preparation time.

### Interference barcode — draw with phase

Each qubit gets `H |> T |> H`: a Hadamard makes |+⟩, the `T` gate adds a π/4
phase to |1⟩, and a second Hadamard interferes the paths back together. The
phase is not 0 or π, so the paths no longer cancel symmetrically — P(0) ≈ 0.854,
P(1) ≈ 0.146. Across three qubits the eight outcomes line up by Hamming weight
into a descending fringe, which the checker renders as an ASCII barcode: a
pattern drawn by phase, not by a lookup table.
[`interference_barcode.qn`](https://github.com/arniber21/quon/blob/main/samples/creative/interference_barcode.qn)
+ [`interference_barcode.py`](https://github.com/arniber21/quon/blob/main/test/verify/interference_barcode.py),
which renders the histogram and asserts it is monotone in Hamming weight.

**Concept:** interference — superposition alone is flat (the dice); a phase
between the branches makes the randomness *structured*.

## Reproducing

Build the compiler, then run any referee:

```sh
cargo build --release -p quonc
QUONC=target/release/quonc python test/verify/quantum_dice.py
QUONC=target/release/quonc python test/verify/entangled_twins.py
QUONC=target/release/quonc python test/verify/interference_barcode.py
```

Every `ci: smoke` catalog entry is also compiled with `quonc` in CI (the
[`samples_catalog`](https://github.com/arniber21/quon/blob/main/quonc/tests/samples_catalog.rs)
test); the Aer referees above are seeded for reproducibility.
