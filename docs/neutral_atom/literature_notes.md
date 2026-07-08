# Neutral-atom backend: literature notes

Companion to [architecture_model.md](./architecture_model.md), which is the
normative model/schema document. This file records, per source: what the paper
does, what this backend takes from it, and what it deliberately does *not*
take. Citation keys match the architecture model's References section.

The sources fall into three groups that must not be conflated (see
architecture_model.md §4): the **flat reconfigurable-array compiler line**
([OLSQ-DPQA], [Enola], [Atomique]), the **zoned-architecture compiler line**
([AbstractModel], [RAP], [QMAP-docs]), and the **QEC overhead sources**
([Bravyi24], [BMD07], [Kelly15], [Gottesman97], [PK22], [Xu24], [Fowler12],
[Bluvstein24]).

---

## Flat reconfigurable-array line

### [OLSQ-DPQA] — Tan, Bluvstein, Lukin, Cong, Quantum 8, 1281 (2024), arXiv:2306.03487

**What it is.** The foundational compiler paper for DPQA (dynamically
field-programmable qubit arrays): formalizes the hardware model — static SLM
traps plus a 2D AOD whose traps sit at row×column intersections, a global
Rydberg laser — and compiles circuits to it with an SMT solver (Z3),
minimizing the number of Rydberg stages, provably optimally for small
circuits. A hybrid "iterative peeling" greedy/SMT mode reaches ~90 qubits.

**What this backend takes from it.**
- The *formal statement* of the AOD movement constraints (its App. A.2
  Implications 5–6 and SMT Eqs. (9)–(16)): rows/columns move as units, order
  preservation, no individual trap movement, SLM traps stationary, one atom
  per trap. These are constraints M1–M5 in architecture_model.md §5 and the
  spec for the `quantum.na` movement-legality verifier (#102, #106).
- The Rydberg interaction rules (App. A.3 Implications 8–10): range r_b,
  compulsory interaction under a global laser, and the 2.5 × r_b isolation
  rule — source of the `min_rydberg_spacing_um` schema field.
- Parameter values: r_b = 7.5 µm, minimum AOD row/column separation
  d_s = 2 µm, movement time t = T0·√(D/D0) with T0 = 200 µs, D0 = 110 µm.
- The instruction vocabulary (`init` / `rydberg` / `move` / `activate` /
  `deactivate`) informed the `quantum.na` dialect's op set and the
  `NeutralAtomAction` types (#100, #102).

**What it does not contribute.** No zones (flat plane only; zones are its
Sec. 6 future work). We do not adopt its SMT solving approach — Quon's
schedulers are heuristic (edge coloring, greedy move grouping, A*), following
[Enola] and [RAP].

### [Enola] — Tan, Lin, Cong, ASPDAC 2025, arXiv:2405.15095

**What it is.** The scalable successor to [OLSQ-DPQA] on the same hardware
model: decouples compilation into scheduling → placement → routing.
Scheduling is reduced to edge coloring of the qubit-interaction graph
(Misra–Gries gives ≤ S_opt + 1 Rydberg stages in O(n³) — its Theorem 1);
placement is simulated annealing; routing greedily extracts maximal
independent sets from a move-conflict graph. Compiles 10,000 qubits in
minutes with ~5.9× fidelity gain over [OLSQ-DPQA].

**What this backend takes from it.**
- The staged pipeline shape and the reduction *scheduling = edge coloring* —
  the direct blueprint for #105 (greedy edge-coloring entangling-layer
  scheduler) and the interaction-graph extraction of #103.
- The move-conflict formulation of AOD constraints (its Sec. 5: moves as
  (src, dst) 4-tuples; three conflict types per axis encoding coupling,
  no-merging, and order preservation) — the blueprint for the flat movement
  planner (#106), where a parallel movement round = an independent set of
  compatible moves.
- The fidelity model shape (its Eq. (1)): per-gate, per-transfer, idle-
  excitation, and 1 − t/T decoherence factors — informs the resource
  estimator's fidelity estimate (#110) and the cost-model terms (§9).
- Parameter values: transfer 15 µs at 99.9%, CZ 360 ns at 99.5%, 1Q 625 ns at
  99.97%, idle-excitation 99.75% per stage, a = 2750 m/s², T2 = 1.5 s.
- The argument that *minimizing Rydberg stages* is the correct flat-array
  objective (global laser penalizes idle atoms every stage) — adopted as the
  `w_stage` cost term.

**What it does not contribute.** No zones (its Sec. 8 defers them). We do not
reproduce its simulated-annealing placer (Quon's #104 placements are simpler
heuristics) nor claim its near-optimality bound for our scheduler unless the
implementation actually uses Misra–Gries.

### [Atomique] — Wang, Liu, Tan, Liu, Gu, Pan, Cong, Acar, Han, ISCA 2024, arXiv:2311.15123

**What it is.** A scalable heuristic compiler for the same flat FPQA/RAA
hardware, with a different decomposition: a MAX k-Cut qubit-array mapper
(which array does each qubit live in), a load-balance/aligned qubit-atom
mapper (which trap within the array), and a greedy high-parallelism router
that grows maximal legal parallel gate sets checking its Constraints 1–3
(unwanted-pair, order preservation, no overlap). Static array assignment; no
mid-circuit trap transfers; routing = movement plus gate-based SWAPs.

**What this backend takes from it.**
- The crisp verbatim statement of the three routing-time hardware constraints
  (its Sec. III-C, Figs. 9–11), used alongside [OLSQ-DPQA]'s formalization for
  M2/M3 and for constraint R2 (a move that parks non-partners within r_b is
  illegal).
- The clarification that ordering constraints bind per AOD set (different
  AODs may cross) — reflected in the `num_aods` schema field.
- Placement inspiration for #104: degree-based and interaction-clustering
  placements echo its load-balance and aligned mappings (attributed as
  inspiration, not reproduction).
- A cautionary data point recorded in architecture_model.md §5: it charges a
  *fixed* 300 µs per move (no distance dependence), diverging from the √-law
  the other papers and our cost model use.

**What it does not contribute.** Its heating/atom-loss/cooling fidelity model
(its Eqs. (1)–(2)) is explicitly out of scope for v0 (§2). Its numeric
fidelity/coherence table is deliberately ×10-optimistic relative to the 2022
experiments it scales from (stated in its Secs. IV–V-A) — do not quote those
numbers as measured values.

---

## Zoned-architecture line

### [AbstractModel] — Stade, Schmid, Burgholzer, Wille, IEEE QCE 2024, arXiv:2405.08068

**What it is.** Formalizes the zoned neutral-atom architecture (after
[Bluvstein24]): storage / entangling / readout zones, the load–move–store
shuttling step, and four shuttling constraints (non-crossing, preservation,
ghost spots, array alignment). On top of it, an efficient router (tool:
NALAC) for *logical* entangling gates: maximal independent set to pick
AOD-vs-SLM qubits, then a modified-DSatur edge coloring that also prevents
AOD crossings, then positioning by topological sort.

**What this backend takes from it.**
- The three-zone taxonomy and per-zone operation legality (architecture_model
  §7, the `Zone.kind` schema field, and #107's zone constraints:
  entangling only in entanglement zone, measurement only in readout zone).
- The shuttle semantics — load (SLM→AOD), move, store (AOD→SLM) as separate
  costed actions — matching `NeutralAtomAction` moves plus transfers.
- The ghost-spots constraint: activating AOD rows/columns affects *all* their
  intersections, so partial pickups need offset moves; this is why zone
  transfers are modeled as full rearrangement steps rather than free
  teleports.
- Timing constants for the zoned setting: load/store 20 µs, shuttling speed
  0.55 µm/µs, CZ 0.2 µs (its Sec. VI, drawn from [Bluvstein24]) — recorded
  here for comparison, though the target JSON follows [RAP]/[QMAP-docs]
  values where they differ.

**What it does not contribute.** Its logical-array (transversal-gate) routing
algorithm is not reproduced in v0 — code blocks are a resource abstraction
(#109), not a scheduled logical-gate layer.

### [RAP] — Stade, Lin, Cong, Wille, ICCAD 2025, arXiv:2505.22715 — *the reproduced paper*

**What it is.** Routing-aware placement for zoned architectures. Key insight:
placements that minimize travel distance can force the router to serialize
movements (AOD constraint violations), so placement quality must be measured
in *routing* cost. Per 2Q layer, an A* search chooses the placement; the node
cost greedily groups implied movements into AOD-compatible parallel groups
and charges cost(p) = Σ_G √(d_max(G)) (its Eq. (1)) plus reuse and look-ahead
terms (Eq. (2)), with an inadmissible-but-fast heuristic (Eqs. (3)–(5)).
Placement and routing are thereby optimized jointly, though layer by layer.
Result: rearrangement steps/time reduced 17% on average, up to −59%/−49% on
the 42-qubit `ising` benchmark, vs the distance-minimizing state of the art
(ZAC, re-implemented as the baseline in the same pipeline).

**What this backend takes from it.**
- The entire #107 formulation: this is the paper the zoned joint
  placement-routing scheduler reproduces — specifically its Secs. III-B
  (routing-aware placement definition), IV-A (search space + Eq. (1)/(2)
  costs), IV-B (A* encoding: extend by one gate / one atom), IV-C + V-C
  (heuristic, Eqs. (3)–(5)), V-A (binary-search-tree compatibility check for
  movement groups). #107 requires citing section/algorithm per implemented
  piece; this is the map.
- The reuse optimization (its Sec. III-A, inherited from ZAC): an atom used
  by consecutive 2Q layers stays in the entanglement zone — the
  "do not move atoms already in place" acceptance criterion of #106/#107.
- The rearrangement-time metric t = √(d/2750 m/s²) + 15 µs per transfer (its
  Sec. VI-B) — adopted as the schema's `speed_model` and the movement cost
  term.
- The #111 regression anchor: its Table I (Sec. VI-B). Chosen target rows:
  `ising` 42 qubits (82 2Q gates, 4 layers) — 22 rearrangement steps / 3.1 ms
  routing-agnostic vs 9 steps / 1.6 ms routing-aware; `ising` 98 qubits —
  23 → 12 steps. Single-row numbers are preferred over the table's mean row
  (the printed −17% average is the mean of per-row percentages, not the ratio
  of column means).
- A* parameter sets for reproduction runs: α=0.2, β=0.2, γ=5, δ=0.6
  (QASMBench set) and α=0.2, β=0.8, γ=5, δ=0.9 (MQT Bench set) (its Sec.
  VI-A).

**Caveats for the reproduction (#107/#111).**
- The paper contains *no* zone geometry, gate fidelities, or coherence
  numbers; those live in the companion [QMAP-repo] architecture JSON. A
  faithful reproduction pairs the paper's algorithm and timing model with
  that published architecture file.
- The QMAP repository's newer evaluation scripts use a jerk-limited timing
  model that differs from the paper's √-law except at d = 110 µm; a
  reproduction of the *paper's* Table I must use the √-law.
- The paper models storage + entanglement zones only (no readout zone);
  readout-zone constraints in #107 come from [AbstractModel], not [RAP].

### [QMAP-docs] / [QMAP-repo] — Munich Quantum Toolkit zoned compiler (docs v3.7.0)

**What it is.** The open-source implementation accompanying [RAP] and
[AbstractModel]: a routing-agnostic compiler (the ZAC-style baseline) and the
routing-aware compiler, plus a JSON architecture format (zones as SLM grids,
entanglement zones as two interleaved grids forming trap pairs, AOD limits,
operation durations/fidelities, coherence time).

**What this backend takes from it.**
- The architecture JSON as the model for our `NeutralAtomTarget` schema
  (§8): zone grids with `site_separation`/`r`/`c`/`location`, entanglement
  pairs with a pair gap, `aods` limits, `operation_duration`,
  `operation_fidelity`, `qubit_spec.T`. Our schema flattens and renames
  (units-suffixed fields, one `zones` list) but keeps a field-level
  correspondence so [RAP] reproduction configs can be translated mechanically.
- Concrete cited values for the sample target: CZ 0.36 µs @ 0.995, 1Q
  @ 0.9997, transfer 15 µs @ 0.999, T = 1.5 s, and the evaluation zone
  geometry (storage 73 × 101 @ 4 µm; entanglement 10 × 34 pairs at 12 × 10 µm
  pitch, pair gap 2 µm).
- Pipeline-stage naming (scheduling / reuse analysis / placement / routing /
  code generation), which `quon_na`'s pass structure mirrors (#103–#108).

**What it does not contribute.** We do not depend on or link against QMAP;
it is a reference implementation to validate against, and its post-paper
extensions (IDS placement, relaxed routing) are out of scope.

---

## QEC overhead sources (issue #109 formulas)

### [Bravyi24] — Bravyi, Cross, Gambetta, Maslov, Rall, Yoder, Nature 627, 778 (2024), arXiv:2308.07915

High-threshold bivariate-bicycle qLDPC memory. Contributes two of the four
overhead formulas: the cleanest printed statement of the rotated surface-code
count (its §1: n = d² data + c = n − 1 checks → `SurfaceCodeLike`
N = 2d² − 1) and the rate-based accounting for `HighRateQldpcLike` (its §1:
net rate r = k/(n + c); Table 1: BB codes take 2n physical qubits total;
[[144,12,12]] → 24 atoms per logical including checks, 12 data-only; 288 vs
~3000 surface-code qubits for the same 12 logical qubits).

### [BMD07] — Bombin, Martin-Delgado, PRA 76, 012305 (2007), arXiv:quant-ph/0703272

Origin of the rotated ([[d², 1, d]]) surface code — the reason d² data qubits
suffice. Cited as the construction behind the `SurfaceCodeLike` formula.

### [Kelly15] — Kelly et al., Nature 519, 66 (2015), arXiv:1411.7403

Canonical repetition-code experiment: linear chain of d data + (d − 1)
measure qubits in an alternating pattern (9 qubits at d = 5). The
`RepetitionCodeToy` formula N = 2d − 1 and the shape of its toy demo circuit.

### [Gottesman97] — Gottesman, PhD thesis, Caltech (1997), arXiv:quant-ph/9705052

Standard definition of the [[n, k, d]] notation (§2.3) behind
`AbstractBlockCode`'s flat n/k ratio.

### [PK22] — Panteleev, Kalachev, STOC 2022, arXiv:2111.03654

Asymptotically good qLDPC codes (constant rate, linear distance): the
existence result that makes a rate-parameterized `HighRateQldpcLike`
abstraction well-founded rather than hypothetical.

### [Xu24] — Xu et al., Nature Physics 20, 1084 (2024), arXiv:2308.08648

Constant-overhead fault-tolerant computation *on reconfigurable atom arrays*:
qLDPC memory + computation code + mediating ancillae, with syndrome
extraction via atom rearrangement. Contributes concrete neutral-atom qLDPC
overhead points (its Table I, e.g. 400 logical / 19 600 physical at
p = 10⁻³) and the observation that zones can be organized by *code role*, not
only by operation type — useful context for heterogeneous `CodeFamily`
targets, not implemented in v0.

### [Fowler12] — Fowler, Mariantoni, Martinis, Cleland, PRA 86, 032324 (2012), arXiv:1208.0928

The canonical surface-code reference — cited for the logical error-rate
scaling p_L ≅ 0.03 (p/p_th)^((d+1)/2), p_th ≈ 0.57% per step (its Eqs.
(10)–(11), Fig. 4), *not* for the qubit count: its unrotated planar patch is
(2d − 1)², and citing it for "2d²" would be wrong (recorded as a citation
trap in architecture_model.md §10.1).

### [Bluvstein24] — Bluvstein et al., Nature 626, 58 (2024), arXiv:2312.03982

The zoned logical processor experiment (storage / entangling / readout
zones; transversal CNOT by interlacing two code-block grids under one global
pulse; up to 280 atoms, 48 logical qubits). Contributes the operational
picture behind the QEC layer — code blocks as scheduling units moved whole
between zones — and is the experimental grounding for the zone taxonomy that
[AbstractModel] formalizes. Motivation only: no result from it is reproduced.

### [Bluvstein22] — Bluvstein et al., Nature 604, 451 (2022), arXiv:2112.03923

The coherent-transport experiment that most compiler-paper constants trace
back to (movement timing, transfer duration, coherence). Not read
independently for this backend; recorded because [OLSQ-DPQA], [Enola],
[Atomique], and [RAP] all cite it as the origin of their physics numbers —
when two compiler papers disagree (r_b, move-time law), the disagreement is
in their *readings* of this experiment, which is why architecture_model.md
§8.6 records divergences instead of picking silently.

---

## Cross-cutting divergences worth remembering

| Quantity | [OLSQ-DPQA] | [Enola] | [Atomique] | [RAP]/[QMAP] | Backend choice |
| --- | --- | --- | --- | --- | --- |
| Move time | T0·√(D/D0), 200 µs @ 110 µm | √(d/a), a = 2750 m/s² (same law) | fixed 300 µs/move | √(d/a), a = 2750 m/s² | √-law (reproduction requires it) |
| Rydberg range r_b | 7.5 µm | implies 6 µm | implies 2.5 µm | JSON `rydberg_range` (beam region, not radius) | 7.5 µm, cited to [OLSQ-DPQA] |
| Trap transfer | not costed (optimal mode forbids transfers) | 15 µs @ 99.9%, 4 per gate | 15 µs, loss 0.0068 | 15 µs @ 0.999 | 15 µs @ 0.999 |
| 1Q duration | — | 625 ns | 625 ns | docs example 52 µs (global raster) | 625 ns |
| Zones | none (future work) | none (future work) | none | storage + entanglement (+ readout in [AbstractModel]) | zones optional in schema; required for #107 |
