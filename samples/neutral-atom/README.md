# Neutral-atom samples

Narrative walkthroughs of the neutral-atom (NA) compilation path: zoned vs.
flat movement planning, the routing-agnostic vs. routing-aware placer, and
NA/QEC hybrid scheduling.

**Schedule model only.** Every walkthrough below reports *analytic* schedule
metrics from `quonc`'s NA pipeline (rearrangement steps, trap transfers,
estimated cycles, an analytic physical error budget). None of it is sampled
on real neutral-atom hardware, none of it is a production QEC threshold
claim, and none of it is fused with Sinter/Stim sampled data (ADR-0020). For
QEC evidence backed by Sinter sampling, see
[`examples/na_qec/`](../../examples/na_qec/) (linked below) — that pack has
its own non-claims about thresholds.

## Status

Owned by pack **#192** (NA pedagogy, parent #184). Five narrative samples
plus the linked QEC seed give the two headline comparisons walkthroughs and
schedule/report evidence (see [Headline comparisons](#headline-comparisons)
below):

- **zoned vs. flat backend** (`--na-backend`) — [`bell_pair.qn`](./bell_pair.qn),
  with [`qft_small.qn`](./qft_small.qn) as the "why flat is a footnote, not
  the default" counter-example.
- **routing-agnostic vs. routing-aware placer** (`--na-placer`, zoned only)
  — [`qaoa_maxcut.qn`](./qaoa_maxcut.qn) (dense graph) and
  [`ising_trotter.qn`](./ising_trotter.qn) (nearest-neighbor chain).
- **dynamic circuit** (mid-circuit measurement scheduling) —
  [`syndrome_round.qn`](./syndrome_round.qn), cross-linked from the
  [teleportation cookbook page](../../website/src/content/docs/cookbook/teleportation.mdx).

All five are owned copies (not forks) adapted from `test/na/`'s smoke
programs — see each file's header comment for the exact `quonc` invocation
and what it demonstrates. `neutral-atom/repetition-d3-memory` continues to
link (not copy) [`examples/na_qec/repetition_d3_memory.qn`](../../examples/na_qec/repetition_d3_memory.qn)
per ADR-0025.

## Headline comparisons

### 1. `--na-backend zoned|flat`

```sh
quonc --target targets/neutral_atom/generic_rna_v0.json --na-backend zoned \
      --emit-na-schedule - samples/neutral-atom/bell_pair.qn
quonc --target targets/neutral_atom/generic_rna_v0.json --na-backend flat \
      --emit-na-schedule - samples/neutral-atom/bell_pair.qn
```

On `bell_pair.qn` (2 qubits, 1 `CNOT`), both backends succeed:

| Backend | Estimated cycles | Rearrangement steps | Trap transfers | Bottleneck |
| --- | ---: | ---: | ---: | --- |
| `zoned` (default) | 4 | 1 | 4 | rearrangement |
| `flat` | 1 | 0 | 0 | rydberg |

`zoned` moves qubit 0 into a dedicated entanglement zone before the Rydberg
pulse; `flat` entangles the two atoms in place on the row-major storage
grid, since a single pair already fits inside the target's Rydberg range.
Try the same `--na-backend flat` flag on [`qft_small.qn`](./qft_small.qn)
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
      --resource-report-format markdown samples/neutral-atom/qaoa_maxcut.qn
quonc --target targets/neutral_atom/generic_rna_v0.json --na-backend zoned \
      --na-placer routing-aware --emit-resource-report - \
      --resource-report-format markdown samples/neutral-atom/qaoa_maxcut.qn
```

| Sample | Placer | Estimated cycles | Rearrangement steps | Trap transfers | Total time (µs) |
| --- | --- | ---: | ---: | ---: | ---: |
| `qaoa_maxcut.qn` (dense, 3-regular graph) | `routing-agnostic` (default) | 34 | 8 | 22 | 1640 |
| `qaoa_maxcut.qn` | `routing-aware` | 37 | 9 | 24 | 1718 |
| `ising_trotter.qn` (nearest-neighbor chain) | `routing-agnostic` (default) | 37 | 9 | 20 | 2206 |
| `ising_trotter.qn` | `routing-aware` | 37 | 9 | 20 | 2245 |

This is the honest result, not a cherry-picked one: at this small size,
`routing-aware` (RAP-style lookahead) is not a strict win over
`routing-agnostic` (ZAC-style) on either graph — on the chain the two
modes produce identical structural metrics (locality already does the
work), and on the denser MaxCut graph `routing-aware` is slightly *more*
expensive here. This aligns with the #111 story: the comparison is about
*when* aware placement pays off (denser, larger interaction graphs), not
that it always wins. Both are analytic-schedule-only comparisons.

## Seeds

| Catalog id | Path | Notes |
| --- | --- | --- |
| `neutral-atom/bell-pair` | [`bell_pair.qn`](./bell_pair.qn) | Headline zoned-vs-flat backend story on the smallest possible program. |
| `neutral-atom/qaoa-maxcut` | [`qaoa_maxcut.qn`](./qaoa_maxcut.qn) | Headline routing-agnostic-vs-aware placer story on a dense (3-regular) interaction graph. |
| `neutral-atom/qft-small` | [`qft_small.qn`](./qft_small.qn) | Long-range controlled-rotation fan-in; the "why flat fails here" counter-example to `bell_pair.qn`. |
| `neutral-atom/ising-trotter` | [`ising_trotter.qn`](./ising_trotter.qn) | Nearest-neighbor-only interaction graph; second routing-agnostic-vs-aware data point, contrasting locality against `qaoa_maxcut.qn`. |
| `neutral-atom/syndrome-round-dynamic` | [`syndrome_round.qn`](./syndrome_round.qn) | Dynamic-circuit / mid-circuit-measurement schedule ordering; cross-linked from the teleportation cookbook page. |
| `neutral-atom/repetition-d3-memory` | [`examples/na_qec/repetition_d3_memory.qn`](../../examples/na_qec/repetition_d3_memory.qn) | Distance-3 repetition-code memory on the NA/QEC hybrid path (ADR-0016). Linked, not copied — see [`examples/na_qec/`](../../examples/na_qec/) for QEC evidence and its own non-claims about thresholds. |

## Adding a seed

See [`../CONTRIBUTING.md`](../CONTRIBUTING.md). One canonical `.qn` per
story: link (don't fork) anything that already has a canonical home in
`examples/na_qec/` or `test/na/`; prefer an owned, narratively-commented
copy (like the five above) when the story is genuinely new pedagogy.
