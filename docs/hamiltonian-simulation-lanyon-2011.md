# Hamiltonian-simulation reproduction note: Lanyon *et al.* (2011)

This note selects one small, digital Hamiltonian-simulation result for a Quon
circuit reproduction. It follows the repository's literature-note convention:
record what the primary paper does, what to reproduce, and what must **not** be
claimed from a smaller, portable circuit.

## Recommendation

**Reproduce the two-spin digital Ising evolution in** B. P. Lanyon *et al.*,
[*Universal digital quantum simulation with trapped ions*](https://arxiv.org/abs/1109.1512),
*Science* **334**, 57–61 (2011),
[doi:10.1126/science.1208001](https://doi.org/10.1126/science.1208001).

This is a primary experimental paper, not a review or tutorial. It states the
product-formula algorithm, implements a noncommuting two-spin Ising model with
up to 100 gates on as many as six qubits, and reports the `J = 2B` two-spin
case. Its apparatus has native collective ion gates, but its mathematical
algorithm has a direct gate-model expansion into `H`, `Rz`, and `CX`; this
makes it a good portable Quon target.

**What the paper does.** For a time-independent local Hamiltonian
`H = sum_k H_k`, it gives the Trotter product

$$
e^{-iHt/\hbar}=\lim_{n\to\infty}\left(\prod_k e^{-iH_k t/(n\hbar)}\right)^n.
$$

It then digitally simulates the dimensionless two-spin Ising Hamiltonian

$$
\widetilde H_{\mathrm{Ising}}
 = B(Z_0+Z_1)+JX_0X_1,
\qquad U(\tau)=e^{-i\widetilde H\tau},
$$

where `tau = Et/hbar` is its dimensionless evolution phase. The paper calls
`B(Z_0+Z_1)` the uniform `z`-field and `J X_0X_1` the orthogonal spin-spin
interaction; they do not commute. It implements the two factors
stroboscopically with its `O2` and `O4` trapped-ion operations and observes
convergence toward exact dynamics as the digital resolution increases.

**Primary-source basis.** The formula and the locality/Trotter discussion are
in the algorithm discussion preceding the operation set; the Ising Hamiltonian,
noncommutation, `O2`/`O4` step, and `J = 2B` experiment immediately follow it
in the linked arXiv manuscript. The article also reports a 91(1)% process
fidelity at its finest resolution for that two-spin time-independent case.
That experimental fidelity is evidence about its calcium-ion apparatus, not a
fidelity prediction for a Quon-compiled circuit.

## Direct circuit compilation

Set `hbar = 1`, choose `n` first-order steps, and define

$$
\beta = \frac{B\tau}{n},\qquad
\gamma = \frac{J\tau}{n}.
$$

One explicit first-order step is

$$
S_1(\tau/n)=e^{-i\beta(Z_0+Z_1)}e^{-i\gamma X_0X_1},
\qquad U(\tau)\approx S_1(\tau/n)^n.
$$

The commuting terms within each displayed factor give the following gate
recipe (with the standard convention `Rz(a) = exp(-i a Z/2)`):

1. Apply `Rz(2 beta)` to qubits `q0` and `q1`.
2. Implement `exp(-i gamma X0 X1)` as
   `H(q0); H(q1); CX(q0, q1); Rz(2 gamma, q1); CX(q0, q1); H(q0); H(q1)`.
3. Repeat the entire step exactly `n` times. Measure in the computational
   basis for `Z` populations; add final `H` on measured qubits when measuring
   `X` observables.

The second line is a gate identity, not an approximation:

$$
 e^{-i\gamma X_0X_1}
 = (H\otimes H)\,\operatorname{CX}\,[I\otimes R_z(2\gamma)]\,
   \operatorname{CX}\,(H\otimes H).
$$

Thus the only algorithmic approximation in this circuit is the finite-step
product formula (apart from any target/backend noise).

## Concrete reproduction instance

Use the paper's relative coupling `J = 2B` with a deliberately small,
inspectable parameter point:

| Item | Value |
| --- | --- |
| qubits | `q0`, `q1` |
| initial state | `|00>` (also run `|11>` if initialization syntax permits) |
| `B`, `J` | `1/2`, `1` |
| total phase `tau` | `pi/2` |
| Trotter steps `n` | `4` |
| field rotations per step | `Rz(pi/8)` on each qubit |
| interaction rotation per step | the `XX` block above with central `Rz(pi/4)` |

This consumes, before any optimizer cancellation, four steps of two field
rotations and one `XX` block: **16 `H`, 8 `CX`, and 12 `Rz` gates**. It
exercises parameter arithmetic, repeated composition, rotations on both
qubits, a directional two-qubit gate, basis changes, and measurement-basis
changes. It is nontrivial while still small enough to compare state vectors or
observable trajectories against a classical `4 x 4` matrix exponential.

## Optional three-qubit pedagogical extension

The paper also demonstrates three-spin programmable interactions, but the
following open-chain circuit is a **pedagogical extension**, not a claim that
it recreates one of the paper's calibrated three-ion sequences:

$$
\widetilde H_3=B(Z_0+Z_1+Z_2)+J(X_0X_1+X_1X_2).
$$

For the same `B`, `J`, `tau`, and `n`, each step is three field `Rz(pi/8)`
gates followed by the above `XX` block on `(q0,q1)` and then `(q1,q2)`.
The two `XX` exponentials commute algebraically, although their decomposed
blocks share `q1` and should be emitted serially unless Quon's scheduler proves
a safe rewrite. This variant exercises a chain interaction graph, reuse of a
middle qubit, and 16 `CX` gates over four steps. It is useful after the
2-qubit reproduction passes, not instead of it.

## Fidelity and equivalence boundaries

- **Exact reproduction:** the selected Hamiltonian, its `J = 2B` relation, and
  finite-step first-order digital evolution are faithful to the paper. The
  `XX` synthesis above exactly implements each interaction factor.
- **Not exact continuous evolution:** with `n = 4`,
  `S_1(tau/n)^n != exp(-i H tau)` because the field and interaction do not
  commute. Increase `n` and compare to the exact two-qubit matrix exponential;
  for fixed total phase, first-order product-formula error decreases with the
  step size.
- **Not an exact hardware reproduction:** Lanyon *et al.* use collective
  trapped-ion `O2`/`O4` operations, their pulse calibration, and their noise
  model. `H`/`Rz`/`CX` is a portable logical decomposition, so do not compare
  its gate count, wall-clock duration, or measured fidelity to the paper's
  reported 91(1)% result.
- **Conventions must remain explicit:** reversing the product-factor order
  changes a finite-`n` circuit but not its `n -> infinity` limit; changing the
  sign convention for `Rz` changes the simulated Hamiltonian. Keep the
  `Rz(a) = exp(-i a Z/2)` convention above in the Quon example and in its
  reference calculation.

## Exact circuit features to exercise

The initial Quon artifact should implement the **two-qubit, four-step recipe**
exactly as listed: symbolic or numeric `pi` angles, loop/repetition (or four
explicit repetitions), `H`, parameterized `Rz`, `CX`, and both `Z`- and
`X`-basis measurements. Its acceptance comparison is the exact two-spin
matrix exponential at `B=1/2`, `J=1`, `tau=pi/2`, alongside the expected
finite-step Trotter discrepancy. Only then add the three-qubit open chain.
