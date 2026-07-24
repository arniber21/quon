# Creative / games samples

Playful Quon programs that engage a learner while teaching exactly one real
Quon concept each. Each is a small `.qn` circuit paired with a seeded Aer
checker that doubles as the "game" referee: it runs the circuit many times and
asserts the playable claim (a fair die, a mind-reading trick, a drawn fringe).
This pack (issue #200, parent epic #184) is the Qiskit-inspired counterpart to
the `learning/` track: same concepts, game framing.

The split is the same as the `applications/` demos — Quon prepares the state
and measures; the Python checker plays the game (distribution check, ASCII
render). No claim of quantum supremacy is made or needed: the point is that
each concept is *visible* in the statistics a beginner can run.

## Status

Dedicated pack — [#200](https://github.com/arniber21/quon/issues/200). The
three samples below were landed together; see each entry's `.qn` header and the
matching `test/verify/*.py` checker for the concept and the playable claim.

## Seeds

| Catalog id | Path | Game | Concept | Quantitative check |
| --- | --- | --- | --- | --- |
| `creative/quantum-dice` | [`quantum_dice.qn`](./quantum_dice.qn) | Roll a fair 8-sided die | Superposition + parallel composition (`for` depth-1 layer) + Born rule | Aer: all 8 faces, each ~1/8 |
| `creative/entangled-twins` | [`entangled_twins.qn`](./entangled_twins.qn) | Predict Bob's bits from Alice's | Entanglement as a *determined relation* (correlation vs anti-correlation) | Aer: b0==b1 and b2!=b3, every shot |
| `creative/interference-barcode` | [`interference_barcode.qn`](./interference_barcode.qn) | Draw a barcode with phase | Interference (T phase + superposition -> non-uniform Born distribution) | Aer: histogram monotone in Hamming weight; ASCII render |

## Concepts and cross-links

- **Superposition & the Born rule.** `quantum_dice.qn` is the many-qubit
  cousin of [`samples/applications/quantum_coin_flip.qn`](../applications/quantum_coin_flip.qn)
  and [`samples/learning/states_measurement.qn`](../learning/states_measurement.qn).
  The `for q in qubits(n) { H q }` layer is the same depth-1 parallel layer
  taught in [`samples/learning/gates_composition.qn`](../learning/gates_composition.qn).
- **Entanglement as a relation.** `entangled_twins.qn` extends
  [`samples/learning/hello_bell.qn`](../learning/hello_bell.qn) (one correlated
  pair) and [`samples/learning/entanglement.qn`](../learning/entanglement.qn)
  (correlated vs separable) by pairing a |Φ⁺⟩ (correlated) with a |Ψ⁺⟩
  (anti-correlated) Bell state: the relation is set by preparation, not by
  signaling.
- **Interference.** `interference_barcode.qn` uses the `H |> T |> H` block that
  introduces the non-Clifford `T` gate (see the
  [Clifford classification](../../website/src/content/docs/language/clifford.md)
  language page) to bend a flat superposition into a structured fringe — the
  same physics as the dice, with one phase added.

## Adding a seed

See [`../CONTRIBUTING.md`](../CONTRIBUTING.md). Every new file here needs a
matching row in [`../catalog.yaml`](../catalog.yaml) and a seeded checker under
[`../../test/verify/`](../../test/verify/). Keep one concept per sample and
keep the game framing honest — the checker must verify a claim the prose makes.
