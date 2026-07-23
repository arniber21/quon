---
title: Neutral-atom architecture model
description: The hardware model, target schema, and scheduling approach behind Quon's neutral-atom backend.
---

Reconfigurable neutral-atom arrays are a different kind of quantum machine than
the fixed-coupling devices most compilers target. There are no SWAP gates:
connectivity is rewritten at runtime by physically moving atoms. That single
fact reshapes the entire compilation problem — placement, routing, scheduling,
and cost are all about *movement* rather than gate insertion. This page
documents the abstract hardware model Quon's `quon_na` backend approximates, the
target JSON schema every pass is built against, and how the compiler schedules
movement and entanglement on top of it.

This is a web-oriented condensation of the normative reference in the repository.
Every modeled mechanism is attributed to a specific paper, and every numeric
constant is either pinned to a cited source or explicitly labeled an
illustrative placeholder. For the full field-by-field schema, the resource-report
formats, and the complete citation list, read the [source document in the
repo](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/architecture_model.md).

## What is being modeled

The backend targets an abstract **reconfigurable neutral-atom array**: atoms
held in optical traps on a 2D plane, where some traps are static (a spatial
light modulator, SLM) and some are mobile (a crossed 2D acousto-optic
deflector, AOD). Entangling gates are mediated by the Rydberg blockade between
atoms brought within interaction range; connectivity is reconfigured at runtime
by physically moving atoms rather than by inserting SWAP gates. This is the
family called **DPQA** (dynamically field-programmable qubit arrays) in
[OLSQ-DPQA] and [Enola], and **FPQA/RAA** in [Atomique].

Two variants are modeled, in two compiler stages:

1. **Flat reconfigurable array** — a single plane, one SLM grid plus one or more
   AOD grids, a global Rydberg laser illuminating the whole plane. "Which pairs
   may interact" is a *distance and scheduling* notion, not a spatial one. The
   model of [OLSQ-DPQA], [Enola], and [Atomique].
2. **Zoned architecture** — physically separate regions with different
   capabilities: a *storage zone* (dense static traps, shielded, long
   coherence), an *entanglement zone* (paired traps under a zone-restricted
   Rydberg beam), and optionally a *readout zone* (mid-circuit measurement
   without disturbing other atoms). The model of [AbstractModel], [RAP], and the
   experimental architecture of [Bluvstein24].

These are two genuinely different hardware models from two different lines of
literature. The backend builds both deliberately — the flat AOD movement planner
first, the zoned joint placement-routing scheduler second — and results from one
stage must never be quoted as evidence about the other.

## What is explicitly not modeled

Drawing the boundary of the model is as important as drawing its contents.
Several things a reader might expect are deliberately absent:

- **No full QEC decoder.** The QEC layer is a logical-op and resource-accounting
  abstraction: code blocks expand to atom counts and syndrome rounds are
  schedulable operations, but no syndrome decoding, error propagation, or
  logical-failure-rate simulation is performed.
- **No vendor-specific hardware claims.** The model is grounded entirely in
  published techniques and their stated parameters. The target descriptor is a
  *generic* reconfigurable neutral-atom machine; it does not describe any
  specific commercial device.
- **No pulse-level physics.** Gates are discrete scheduled actions with
  durations and fidelities; no Hamiltonian, pulse-shape, or blockade-strength
  simulation.
- **No atom-loss or heating simulation.** [Atomique] models movement-induced
  heating and probabilistic atom loss; this backend's v0 cost model does not.
  Movement cost is time-based; heating-aware cost is a possible later extension.
- **No continuous trajectories.** Movement is modeled at the granularity of
  rearrangement steps between discrete placements, as in [RAP].
  Collision-freedom within a step is guaranteed by the AOD ordering constraints,
  not by path simulation.

## Fixed, reconfigurable, and zoned arrays

Three architecture families appear in the neutral-atom compilation literature.
The distinction matters because compiler problems and cost models differ per
family:

| Family | Traps | Connectivity | Compiler problem |
| --- | --- | --- | --- |
| **Fixed array** | Static only | Fixed local neighborhood (within an interaction radius) | Qubit mapping + SWAP insertion, as on any fixed-coupling device. Baselines "FAA" in [Atomique]. |
| **Reconfigurable array (DPQA/FPQA)** | Static SLM + mobile AOD | Any pair can be brought within range by movement | Joint placement, movement scheduling, gate layering under AOD constraints ([OLSQ-DPQA], [Enola], [Atomique]). |
| **Zoned architecture** | Static traps in operation-specific zones + AOD transport between zones | Entangling only inside the entanglement zone; storage shielded; readout isolated | Zone-aware placement and routing; shuttle scheduling storage↔entanglement↔readout ([AbstractModel], [RAP], [Bluvstein24]). |

Quon's existing gate-model backend covers the first family generically. The
`quon_na` backend covers the second and third via
`TargetKind::NeutralAtomReconfigurable`.

## The movement model

This backend does **not** model free-grid Manhattan movement. Atoms in an AOD
are trapped at the intersections of a set of AOD rows and columns; the only
movement controls are the Y coordinate of each row and the X coordinate of each
column ([OLSQ-DPQA]: "we cannot move AOD traps individually"). A movement plan
that moves one atom independently of its row and column mates is not realizable
on this hardware and is rejected by the movement-legality verifier. The enforced
constraints, each pinned to a source:

- **M1 — Coupled motion.** All atoms in an activated AOD row (column) move
  together when that row (column) moves.
- **M2 — Order preservation (no crossing).** Within one AOD, a row cannot move
  past another row, nor a column past another column — crossing rows would
  violate minimum separation and cause heating/atom loss.
- **M3 — No merging.** Two rows (columns) of the same AOD cannot occupy the
  same coordinate.
- **M4 — Static traps are static.** SLM-trapped atoms do not move; a position
  change requires a trap transfer into an AOD, a costed, fidelity-bearing action.
- **M5 — Occupancy.** One trap holds at most one atom; two atoms may never
  occupy the same site at the same time.

M2 and M3 bind *within one AOD only*; rows/columns of different AODs may cross.
In the zoned model the same constraints reappear as the three rearrangement
constraints of [RAP]: *non-crossing*, *preservation* (atoms starting in the same
AOD row stay in it), and *ghost spots* (activating rows and columns affects all
their grid intersections, so loading a subset needs offset moves — this is why
zone transfer is not free even when distances are short).

**Movement timing.** One rearrangement step covering maximum distance $d$ takes

```
t_move(d) = sqrt(d / a),  a = 2750 m/s²   (e.g. d = 110 µm → t = 200 µs)
```

cited to [RAP] and [Enola]; [OLSQ-DPQA] states the same law as
$t = T_0\sqrt{D/D_0}$ with $T_0 = 200\,\mu s$, $D_0 = 110\,\mu m$ ("to maintain
constant heating"). Each trap transfer adds 15 µs. A known literature divergence:
[Atomique] instead charges a fixed 300 µs per movement stage regardless of
distance. The backend implements the √-law because it is what the reproduced
paper's metric uses; the divergence is documented so nobody "fixes" one to match
the other.

## The interaction model

For the flat array, the Rydberg laser is global, which makes interaction a
scheduling problem more than a geometry one:

- **R1 — Range.** Two atoms can perform an entangling gate iff their distance
  is $\le r_b$ (the Rydberg blockade radius) while the Rydberg laser is on.
- **R2 — Compulsion.** The flat-array laser illuminates the whole plane: every
  pair within $r_b$ when the laser fires undergoes a gate, wanted or not. Parking
  two non-partner atoms within $r_b$ at a Rydberg stage is illegal.
- **R3 — Isolation.** Non-interacting atoms must be separated by $> 2.5\,r_b$;
  among any three atoms at most one pairwise distance may be below this when the
  laser is on. This is the source of the `min_rydberg_spacing_um` field.
- **R4 — Stage-based error.** Because the laser is global, every illuminated
  idle atom accrues error each Rydberg stage. Minimizing the number of Rydberg
  stages — not the gate count — is the correct flat-array objective.

In the zoned model, R1–R3 hold *inside the entanglement zone*: traps there come
in pairs placed so that pair members interact and distinct pairs do not. The
beam covers only that zone, so storage-zone atoms are shielded and R4's idle
penalty applies only to atoms left isolated in the entanglement zone.

## Scheduling

Compilation proceeds in three moves: extract an interaction graph from the
circuit, color its entangling layers, then plan movement and compact the
schedule.

**Edge coloring.** Each 2-qubit gate layer is a graph whose edges are the gates
and whose vertices are the atoms. Scheduling a layer means partitioning its
edges into Rydberg stages where no two edges sharing a vertex fire together — an
edge coloring. The backend uses the [Enola] Misra–Gries bound, which guarantees
at most $S_{opt} + 1$ stages, so the number of Rydberg stages stays within one
of optimal.

**ASAP scheduling and compaction.** The flat movement planner greedily builds
maximal sets of AOD-compatible parallel moves (maximal independent sets in a
move-conflict graph, after [Enola] and [Atomique]). An exclusive-cycle ASAP
serializer lays independent layers out sequentially for a merge-free baseline,
then a greedy pass recovers legal entangle-only parallelism. This compaction pass
is engineering glue — not a paper reproduction, and not Enola-optimal ASAP;
[Enola] is cited only for the critical-path lower bound.

**The zoned scheduler** reproduces the joint placement-routing formulation of
[RAP]: placements are chosen per 2Q layer by an A* search whose node cost is a
*routing* cost — implied movements grouped greedily into AOD-compatible parallel
groups, $\text{cost}(p) = \sum_G \sqrt{d_{max}(G)}$ over groups $G$, extended with
reuse and one-layer look-ahead. Placement quality is measured in rearrangement
steps and durations, not raw travel distance — that is what "routing-aware"
means versus the sequential distance-minimizing placement of the ZAC baseline it
improves on. The regression anchor pins to [RAP] Table I (e.g. the 42-qubit
`ising` benchmark: 22 rearrangement steps routing-agnostic vs 9 routing-aware).

## The target JSON schema

The `NeutralAtomTarget` payload is loaded with `quonc --target <path>`. Field
names are normative; the checked-in sample is
`targets/neutral_atom/generic_rna_v0.json`. Units: lengths in µm, times in µs,
fidelities as probabilities in $[0, 1]$.

The top level ties the pieces together:

| Field | Type | Meaning |
| --- | --- | --- |
| `id` | string | Target identifier |
| `kind` | `"neutral_atom_reconfigurable"` | `TargetKind` discriminant |
| `grid` | object | Bounding box: `width_um`, `height_um` |
| `zones` | array of Zone | Zone list — occupancy, capacity, operation legality |
| `movement` | object | AOD movement model |
| `interaction` | object | Rydberg parameters |
| `native_gates` | array of string | Gate names executable natively, e.g. `["cz", "rz_local", "ry_global", "measure_z"]` |
| `timing` | object | Operation durations |
| `fidelity` | object | Operation fidelities + coherence time |
| `error_model` | object, optional | Explicit physical error probabilities for QEC (never derived as `1 − fidelity`) |
| `cost_model` | object | Linear cost weights |

A **Zone** declares a region's capability. A flat-array target is expressed as a
single `entanglement` zone covering the whole grid, so the zone constraints
degenerate to the flat model; a zoned target declares at least one `storage` and
one `entanglement` zone.

| Field | Type | Meaning |
| --- | --- | --- |
| `zone_id` | integer | Unique zone identifier |
| `kind` | `"storage"` \| `"entanglement"` \| `"readout"` | Zone capability |
| `rows`, `cols` | integer | Static-trap grid extent (for `entanglement`: trap *pairs*) |
| `origin_um` | [number, number] | Lower-left corner of the trap grid |
| `site_pitch_um` | [number, number] | Trap spacing in x and y |
| `pair_gap_um` | number, `entanglement` only | Distance between the two traps of a pair |

The **Movement** object selects the AOD row/column-coupled semantics (the only
legal value; `"free_manhattan"` is deliberately not a variant) and caps the
simultaneously usable rows/columns and independent AOD units, which is where M2
and M3 bind.

| Field | Type | Meaning |
| --- | --- | --- |
| `model` | `"aod_row_column_coupled"` | Enables M1–M5 in the movement-legality verifier |
| `aod_rows`, `aod_cols` | integer | Max simultaneously usable AOD rows/columns |
| `num_aods` | integer | Number of independent AOD units |
| `min_row_col_separation_um` | number | Minimum spacing of same-AOD rows/columns |
| `speed_model` | `{ "kind": "sqrt", "acceleration_m_s2": 2750 }` | Move duration $t = \sqrt{d/a}$ |
| `trap_transfer_us` | number | Duration of one pick-up or drop-off |

The **Interaction** object pins the Rydberg geometry that gates R1–R3:

| Field | Type | Meaning |
| --- | --- | --- |
| `rydberg_range_um` | number | $r_b$: max distance for an entangling pair |
| `min_rydberg_spacing_um` | number | Isolation distance for non-partners, $= 2.5\,r_b$ per R3 |
| `max_parallel_entangling_pairs` | integer | Cap on simultaneous 2Q gates per Rydberg stage |

**Timing** and **fidelity** are flat objects of operation durations and
per-action fidelities — `cz_us`, `single_qubit_us`, `measurement_us`, `reset_us`;
`cz`, `single_qubit`, `atom_transfer`, `coherence_time_us`. The optional
**`error_model`** is a *sibling* of `fidelity`, not a replacement: it carries
explicit physical error probabilities (per Rydberg stage, per measurement round,
per rearrangement step, per transfer, per µs of idle) for QEC resource
accounting. Per ADR-0017, you must never convert rates as `1 − fidelity.*`.

### Cost model

The v0 cost model is a simple linear functional over a compiled schedule,
reported by the resource estimator and minimized greedily by the schedulers:

```
cost(schedule) = w_stage · n_rydberg_stages
              + w_move  · Σ_steps t_move(d_max(step))
              + w_xfer  · n_trap_transfers
              + w_idle  · Σ_atoms t_idle(atom)
```

Each term is grounded separately: Rydberg stages expose all illuminated atoms to
error (the flat-array objective); $\sum\sqrt{d_{max}}$ per group is the [RAP]
placement cost; transfers are fidelity-bearing actions the reuse optimization
exists to save; idle time is a linear decoherence proxy. The *shape* of the cost
(which terms exist) is cited; the *weights* are illustrative placeholders, not
published values.

## QEC overlay

The QEC abstraction layer expands each logical qubit's code block into atoms by
an exact per-`CodeFamily` formula. $N$ below counts atoms per logical qubit,
including syndrome/check ancillas unless stated otherwise.

| `CodeFamily` | Formula (atoms per logical qubit) | Parameters | Primary citation |
| --- | --- | --- | --- |
| `SurfaceCodeLike` | $2d^2 - 1$ (incl. checks) | distance $d$ | [Bravyi24] §1; [BMD07] |
| `RepetitionCodeToy` | $2d - 1$ (incl. measure atoms) | distance $d$ | [Kelly15] Fig. 1b |
| `HighRateQldpcLike` | $\lceil 1/r \rceil$ ($r$ = net rate, incl. checks) | rate $r$ | [Bravyi24] §1, Table 1 |
| `AbstractBlockCode` | $\lceil n/k \rceil$ (user convention) | $n, k$ | [Gottesman97] §2.3 |

A few subtleties are worth recording. The rotated surface code's $2d^2 - 1$ is
exact ([Bravyi24]); the common shorthand $N \approx 2d^2$ is this rounded. For
qLDPC, physical-per-logical overhead is the *inverse* of the rate, $n/k = 1/r$,
not $k/r$ — an easy bug, called out in the issue. Constant-rate qLDPC families
exist asymptotically, with concrete low-overhead points on reconfigurable
arrays, which is the reason this family is worth modeling on this backend at all.

**The hybrid QEC path.** Code blocks are scheduling units: whole blocks move
between zones and logical 2Q gates are physical-parallel transversal
interleavings — the operational picture motivating this layer, demonstrated in
[Bluvstein24]. The hybrid QEC schedule path (ADR-0016) adds `memory_rounds` —
syndrome cycles that are schedulable operations — to the resource report. The
**atom-indexed interaction graph** (ADR-0029) lets the scheduler treat each
physical atom as a first-class node, so transversal logical gates over code
blocks decompose into atom-level entangling layers that the same movement and
edge-coloring machinery handles. Sizing helpers live in `quon_qec`, with
`quon_na` consuming them for schedules.

Two artifacts come out of QEC evaluation and must never be fused (ADR-0020): a
compiler **analytic** `ResourceReport` (schedule metrics, QEC metadata, an
`error_budget` of `rate × count`, and an Enola-Eq.-(1) fidelity estimate) and a
**sampled** Sinter CSV of logical failure rates from Monte Carlo. Neither is a
threshold claim.

## References

- **[OLSQ-DPQA]** D. B. Tan, D. Bluvstein, M. D. Lukin, J. Cong, "Compiling Quantum Circuits for Dynamically Field-Programmable Neutral Atoms Array Processors", *Quantum* 8, 1281 (2024). arXiv:2306.03487.
- **[Enola]** D. B. Tan, W.-H. Lin, J. Cong, "Compilation for Dynamically Field-Programmable Qubit Arrays with Efficient and Provably Near-Optimal Scheduling", ASPDAC 2025. arXiv:2405.15095.
- **[Atomique]** H. Wang et al., "Atomique: A Quantum Compiler for Reconfigurable Neutral Atom Arrays", ISCA 2024. arXiv:2311.15123.
- **[RAP]** Y. Stade, W.-H. Lin, J. Cong, R. Wille, "Routing-Aware Placement for Zoned Neutral Atom-based Quantum Computing", ICCAD 2025. arXiv:2505.22715. *(The paper reproduced by the zoned scheduler.)*
- **[AbstractModel]** Y. Stade, L. Schmid, L. Burgholzer, R. Wille, "An Abstract Model and Efficient Routing for Logical Entangling Gates on Zoned Neutral Atom Architectures", IEEE QCE 2024. arXiv:2405.08068.
- **[Bluvstein24]** D. Bluvstein et al., "Logical quantum processor based on reconfigurable atom arrays", *Nature* 626, 58 (2024). arXiv:2312.03982.
- **[Bravyi24]** S. Bravyi et al., "High-threshold and low-overhead fault-tolerant quantum memory", *Nature* 627, 778 (2024). arXiv:2308.07915.
- **[BMD07]** H. Bombin, M. A. Martin-Delgado, "Optimal resources for topological two-dimensional stabilizer codes", *Phys. Rev. A* 76, 012305 (2007). arXiv:quant-ph/0703272.
- **[Kelly15]** J. Kelly et al., "State preservation by repetitive error detection in a superconducting quantum circuit", *Nature* 519, 66 (2015). arXiv:1411.7403.
- **[Gottesman97]** D. Gottesman, "Stabilizer Codes and Quantum Error Correction", PhD thesis, Caltech (1997). arXiv:quant-ph/9705052.

For the normative full document — the complete mechanism-to-source attribution
table, the resource-report field reference, the numeric-constant provenance table,
and every `error_budget` multiplier — see
[architecture_model.md](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/architecture_model.md)
in the repository.

Continue to the [compiler pipeline reference](../reference/compiler/) for how
the neutral-atom stages fit into the overall `quonc` flow.
