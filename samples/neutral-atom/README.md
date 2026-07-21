# Neutral-atom samples

Narrative walkthroughs of the neutral-atom (NA) compilation path: zoned vs.
flat movement planning, the routing-agnostic vs. routing-aware zoned
placer, and NA/QEC hybrid scheduling.

**Schedule model only.** Every walkthrough below reports *analytic* schedule
metrics from `quonc`'s NA pipeline (rearrangement steps, trap transfers,
estimated cycles, an analytic physical error budget). None of it is sampled
on real neutral-atom hardware, none of it is a production QEC threshold
claim, and none of it is fused with Sinter/Stim sampled data (ADR-0020). For
QEC evidence backed by Sinter sampling, see
[`examples/na_qec/`](../../examples/na_qec/) (linked below) — that pack has
its own non-claims about thresholds.

## Status

Owned by pack **#192** (NA pedagogy, parent #184). This category **links,
not forks**, its `.qn` sources — one canonical program per story, per the
"link, don't fork" rule in [`../CONTRIBUTING.md`](../CONTRIBUTING.md) — and
carries the narrative here and in [Headline comparisons](#headline-comparisons)
instead of duplicating the program bodies:

- **zoned vs. flat backend** (`--na-backend`) — [`test/na/bell.qn`](../../test/na/bell.qn),
  with [`test/na/qft_small.qn`](../../test/na/qft_small.qn) as the "why flat
  is a footnote, not the default" counter-example.
- **routing-agnostic vs. routing-aware placer** (`--na-placer`, zoned only)
  — [`test/na/qaoa_graph.qn`](../../test/na/qaoa_graph.qn) (dense graph) and
  [`test/na/ising.qn`](../../test/na/ising.qn) (nearest-neighbor chain).
- **dynamic circuit** (genuine mid-circuit measurement scheduling) — the
  already-linked [`examples/na_qec/repetition_d3_memory.qn`](../../examples/na_qec/repetition_d3_memory.qn),
  cross-linked from the
  [teleportation cookbook page](../../website/src/content/docs/cookbook/teleportation.mdx).

All four backend/placer seeds link straight into `test/na/`'s existing NA
smoke programs — see [Seeds](#seeds) below for the exact paths. The
dynamic-circuit story reuses `neutral-atom/repetition-d3-memory`, which
already links (not copies) [`examples/na_qec/repetition_d3_memory.qn`](../../examples/na_qec/repetition_d3_memory.qn)
per ADR-0025, rather than adding a second toy `.qn` that would fork
`test/na/syndrome_round_toy.qn` without actually demonstrating anything the
toy's schedule doesn't already show (see [§3](#3-dynamic-circuit--mid-circuit-measurement) for why).

## Headline comparisons

### 1. `--na-backend zoned|flat`

```sh
quonc --target targets/neutral_atom/generic_rna_v0.json --na-backend zoned \
      --emit-na-schedule - test/na/bell.qn
quonc --target targets/neutral_atom/generic_rna_v0.json --na-backend flat \
      --emit-na-schedule - test/na/bell.qn
```

On `bell.qn` (2 qubits, 1 `CNOT`), both backends succeed:

| Backend | Estimated cycles | Rearrangement steps | Trap transfers | Bottleneck |
| --- | ---: | ---: | ---: | --- |
| `zoned` (default) | 9 | 1 | 4 | rearrangement |
| `flat` | 6 | 0 | 0 | rydberg |

(Issue #298: `H @0`'s Z-Y-Z decomposition into a local `rz` + a global `ry`
raster now contributes real schedule layers — previously it was silently
dropped during extraction, so these headline cycle counts undercounted the
actual program. The `ry` raster is a Hahn-echo-refocused composite pulse,
not a bare raster: every *other* trapped atom (here, qubit 1) gets a local
`Rz(pi)`/`Rz(-pi)` echo pair around the raster's second half, which provably
nets to identity for it — a bare raster would otherwise have also rotated
it, since every atom is bound into the trap array from schedule start (see
`quon_na::pipeline::push_global_ry_with_refocus`). Rearrangement/transfer/
bottleneck are unaffected: none of this needs a site placement.)

`zoned` moves qubit 0 into a dedicated entanglement zone before the Rydberg
pulse; `flat` entangles the two atoms in place on the row-major storage
grid, since a single pair already fits inside the target's Rydberg range.
Try the same `--na-backend flat` flag on [`test/na/qft_small.qn`](../../test/na/qft_small.qn)
(4 qubits, every pair coupled through the controlled-rotation ladder) and
it fails closed instead:

```text
error: AOD movement planning failed: entangling geometry violation (R1–R3) at cycle 0:
R2: non-partners AtomId(2)–AtomId(3) distance 5 ≤ rydberg_range_um 7.5; the flat AOD
planner checks all occupied atoms, idle ones included (B11, fail-closed) — a placement
grid denser than the target's Rydberg limits cannot entangle legally; use the zoned
backend (--na-backend zoned) for such targets
```

That failure is the point: on `generic_rna_v0`, the flat backend's
row-major grid only stays legal for the smallest programs, which is why
**`zoned` is the headline default and `flat` is a secondary path**, not the
other way around. The flat-only `--na-placement row-major|degree|clustering`
knobs (footnote, not primary) only matter once a program is small/sparse
enough for `flat` to succeed in the first place.

### 2. `--na-placer routing-agnostic|routing-aware` (zoned)

```sh
quonc --target targets/neutral_atom/generic_rna_v0.json --na-backend zoned \
      --na-placer routing-agnostic --emit-resource-report - \
      --resource-report-format markdown test/na/qaoa_graph.qn
quonc --target targets/neutral_atom/generic_rna_v0.json --na-backend zoned \
      --na-placer routing-aware --emit-resource-report - \
      --resource-report-format markdown test/na/qaoa_graph.qn
```

| Sample | Placer | Estimated cycles | Rearrangement steps | Trap transfers | Total time (µs) |
| --- | --- | ---: | ---: | ---: | ---: |
| `qaoa_graph.qn` (dense, 3-regular graph) | `routing-agnostic` (default) | 80 | 8 | 22 | 1686 |
| `qaoa_graph.qn` | `routing-aware` | 83 | 9 | 24 | 1742 |
| `ising.qn` (nearest-neighbor chain) | `routing-agnostic` (default) | 48 | 9 | 20 | 2217 |
| `ising.qn` | `routing-aware` | 48 | 9 | 20 | 2217 |

(Issue #298: both programs apply per-qubit `H`/`Rx` rotations
(`qaoa_graph.qn`'s `hadamard_all`/`mixer_4`; `ising.qn`'s `x_layer`) that
extraction used to silently drop; they now contribute real schedule layers.
Rearrangement/transfer counts are unaffected. `Rx` has no first-class
`LocalGateKind` yet — issue #298's scope is `h`/`rz`/`ry`/`u3` — so it
decomposes through the `u3(theta, phi, lambda)` escape hatch even though
`rx` is nominally in `generic_rna_v0.json`'s `native_gates`; `u3` is a plain
`LocalGate`, not a raster, so it costs no extra protection. `H`'s global `ry`
component does: every *other* trapped atom needs a local
`Rz(pi)`/`Rz(-pi)` echo pair around the raster's second half so it provably
doesn't also rotate (a raw raster physically hits every trapped atom, not
just the one it was decomposed for — see
`quon_na::pipeline::push_global_ry_with_refocus`). `qaoa_graph.qn`'s 4
independent `H`s (3 bystanders apiece) is why its cycle counts move more
than `bell.qn`'s single `H` above. The echo-refocus sequence costs O(N)
actions per rotation (O(N²) total for N independent rotations) — see
[`docs/neutral_atom/globalry_scaling.md`](../../docs/neutral_atom/globalry_scaling.md)
for the full scaling analysis, measured benchmark, and the architectural
changes needed to remove the ceiling. `ising.qn` has no bare `H` or other
non-diagonal 1-qubit gate — its `Rzz`-sandwich `Rz`s are diagonal and need
no `ry`/echo at all — so it is untouched by the echo fix and keeps its
original #298 numbers.)

Both zoned placer modes are ZAC-style descendants (Sec. VI-B of the RAP
paper, arXiv:2505.22715): `routing-agnostic` (the default) places atoms by
nearest-legal-distance only, and `routing-aware` is the RAP-style search
that additionally minimizes projected routing cost (Eq. (1)), guided by the
paper's Eq. (3)-(4) heuristic (issue #297) — **`zoned` itself is not
"RAP"**; RAP names one of its two placer *modes*, not the backend. See
[`quon_na::zoned`](../../quon_na/src/zoned.rs) for the module docs this
walkthrough is drawn from.

This is the honest result, not a cherry-picked one: at this small size,
`routing-aware` is not a strict win over `routing-agnostic` on either
graph — on the chain the two modes now produce *identical* metrics across
the board, including total time (2217 µs both). That is not a silent
fallback to the agnostic planner: both runs report `na_placer: routing_aware`
in their schedule metadata and take the aware code path, and (verifiable via
`--emit-na-stats`) the aware search genuinely completes every layer rather
than exhausting its budget — it's a real search that, on this small,
low-contention circuit, converges on the same joint-optimal placement
distance-minimizing greedy already finds. On the denser MaxCut graph,
`routing-aware` still uses one more rearrangement step and 2 more trap
transfers than agnostic, but 1742 µs total time is now *lower* than the
1764 µs a mismatched (pre-#297) cost model used to report for the same step
count — the guided search finds a shorter-travel placement within that
group structure. This aligns with the #111/#297 story: the comparison is
about *when* aware placement pays off (denser, larger interaction graphs —
see the RAP Table I reproduction on the 42-qubit `ising_n42` fixture in
`docs/neutral_atom/rap_table_i_methodology.md`, where aware now beats
agnostic decisively), not that it always wins on every small fixture. Both
are analytic-schedule-only comparisons.

### 3. Dynamic circuit / mid-circuit measurement

```sh
quonc --target targets/neutral_atom/generic_rna_v0.json \
      --emit-na-schedule - --verify-na examples/na_qec/repetition_d3_memory.qn
```

`repetition_d3_memory.qn` (already linked above as
`neutral-atom/repetition-d3-memory`) runs two Kelly-style syndrome-extraction
rounds followed by a logical measurement. Its NA schedule genuinely
interleaves measurement with further entangling layers — this is not a
terminal-measurement-only schedule:

| Metric | Value |
| --- | ---: |
| `rydberg_stages` | 6 |
| `measurement_rounds` | 3 |
| `reset_rounds` | 2 |
| `estimated_cycles` | 37 |

The schedule's layer order (see `quonc/tests/na_showcase.rs`'s
`repetition_d3_memory_schedule_is_genuinely_mid_circuit` test) is
`Entangle2* -> Measure -> Reset -> Wait -> Entangle2* -> Measure -> Reset ->
Wait -> Entangle2* -> Measure`: each syndrome round's ancilla measurement
and reset genuinely sits *between* two rounds of entangling layers, not
after all of them. That is the real ordering constraint this pack can
honestly claim on the NA schedule path today.

What this sample does **not** demonstrate: branching a correction on a
measured outcome. Feed-forward *correction* lowering (conditioning a later
gate on an earlier mid-circuit measurement result) is still limited on the
NA path — this schedule measures and resets every round, but does not
schedule a Pauli-frame correction gate conditioned on the outcome. True
feed-forward control flow on the NA path is future work, not claimed here.
(`test/na/syndrome_round_toy.qn` — the toy CI fixture this story used to
fork into `samples/` — does not actually help demonstrate this: its
program measures three qubits only *after* all entangling layers, and its
NA schedule reports `measurement_rounds: 0` because none of those terminal
measurements are lowered into a scheduled action at all. Forking it into a
second `samples/neutral-atom/*.qn` copy would have restated that same
non-claim under a "dynamic circuit" label it doesn't earn — which is why
this pack points the dynamic-circuit story at the QEC memory circuit's real
mid-circuit schedule instead.)

## Seeds

| Catalog id | Path | Notes |
| --- | --- | --- |
| `neutral-atom/bell-pair` | [`test/na/bell.qn`](../../test/na/bell.qn) | Headline zoned-vs-flat backend story on the smallest possible program. |
| `neutral-atom/qaoa-maxcut` | [`test/na/qaoa_graph.qn`](../../test/na/qaoa_graph.qn) | Headline routing-agnostic-vs-aware placer story on a dense (3-regular) interaction graph. |
| `neutral-atom/qft-small` | [`test/na/qft_small.qn`](../../test/na/qft_small.qn) | Long-range controlled-rotation fan-in; the "why flat fails here" counter-example to `bell.qn`. |
| `neutral-atom/ising-trotter` | [`test/na/ising.qn`](../../test/na/ising.qn) | Nearest-neighbor-only interaction graph; second routing-agnostic-vs-aware data point, contrasting locality against `qaoa_graph.qn`. |
| `neutral-atom/repetition-d3-memory` | [`examples/na_qec/repetition_d3_memory.qn`](../../examples/na_qec/repetition_d3_memory.qn) | Distance-3 repetition-code memory on the NA/QEC hybrid path (ADR-0016); also the dynamic-circuit / genuine mid-circuit-measurement story (§3). Linked, not copied — see [`examples/na_qec/`](../../examples/na_qec/) for QEC evidence and its own non-claims about thresholds. |

## Adding a seed

See [`../CONTRIBUTING.md`](../CONTRIBUTING.md). One canonical `.qn` per
story: link (don't fork) anything that already has a canonical home in
[`test/na/`](../../test/na/) or [`examples/na_qec/`](../../examples/na_qec/);
only add an owned, narratively-commented copy under `samples/neutral-atom/`
when the story's circuit is genuinely new pedagogy that doesn't already
exist as a `test/na/` or `examples/na_qec/` program.
