# NA path: compile + schedule + resource report

The neutral-atom (NA) command loop from a workflow angle: compile against an
NA target, dump the schedule JSON, and read the analytic resource report —
three flags, three artifacts. This directory does **not** own NA narrative
(zoned vs. flat, the RAP placer, why the schedule looks the way it does) —
that's pack #192's job. It links to the canonical NA source and to
[`samples/neutral-atom/`](../../neutral-atom/README.md) rather than
duplicating either.

## Status

Owned by pack [#188](https://github.com/arniber21/quon/issues/188) for the
command-loop framing only. NA pedagogy itself belongs to
[#192](https://github.com/arniber21/quon/issues/192) — see
[`samples/neutral-atom/README.md`](../../neutral-atom/README.md), which
registers the same canonical program under its own catalog id
(`neutral-atom/repetition-d3-memory`) for that story, still `ci: none` since
#192 hasn't landed real narrative content yet. Catalog id here:
`workflows/na-path`, `ci: smoke` — the target-only compile in step 1 below
(typecheck + full NA lowering, no artifact) is what CI runs; it's fast
(well under a second) and doesn't collide with the `neutral-atom/*` row
above, since that row stays `ci: none` until #192 lands. If #192 later
flips its own row to `ci: smoke`, drop one of the two to avoid a duplicate
compile of the same canonical `.qn` (the CI-dedup concern #185's review
raised).

## The loop

The linked program is
[`examples/na_qec/repetition_d3_memory.qn`](../../../examples/na_qec/repetition_d3_memory.qn)
(ADR-0016) — a distance-3 repetition-code memory experiment on the NA/QEC
hybrid path. It is not copied into `samples/`; both this catalog row and
`neutral-atom/repetition-d3-memory`'s point at the one canonical file.

```bash
export QUONC=$PWD/target/debug/quonc   # cargo build -p quonc first
SRC=examples/na_qec/repetition_d3_memory.qn
TARGET=targets/neutral_atom/generic_rna_v0.json

# 1. compile — a real typecheck + full NA lowering, no artifact written
$QUONC $SRC --target $TARGET

# 2. schedule JSON — the visualization envelope (kind: na_schedule_view)
$QUONC $SRC --target $TARGET --emit-na-schedule schedule.json

# 3. analytic resource report — schedule metrics + QEC sizing + error budget
$QUONC $SRC --target $TARGET --emit-resource-report report.json
```

The resource report is explicit about what kind of evidence it is: its JSON
form carries `evidence_kind: "analytic"` and a disclaimer that it is *not*
fused with sampled Stim/Sinter results, and neither is a threshold claim
(ADR-0020) — reproduced here because it's easy to over-claim from a single
number:

```json
{
  "evidence_kind": "analytic",
  "evidence_disclaimer": "Compiler analytic metrics only — not fused with Python/Sinter sampled CSV; neither artifact is a threshold claim (ADR-0020).",
  "logical_qubits": 1,
  "physical_atoms": 5,
  "code_family": "repetition_code_toy",
  "distance": 3,
  "bottleneck": "rearrangement"
}
```

(trimmed — the full report also carries stage counts, timing breakdowns,
and a per-source `error_budget`.)

## Rendering the schedule

The JSON envelope from step 2 is a debug/tooling view, not the canonical
schedule IR (`--emit-na-mlir` remains canonical). Render it as PNG/SVG cycle
frames:

```bash
pip install -r python/requirements-viz.txt
python python/visualize_na_schedule.py schedule.json -o /tmp/na --format svg
```

## Where the NA narrative actually lives

For *why* the schedule looks the way it does — zoned vs. flat movement,
placement strategy, what "rearrangement" and "trap_transfers" mean
physically — read [#192](https://github.com/arniber21/quon/issues/192)'s
walkthroughs under [`samples/neutral-atom/`](../../neutral-atom/README.md)
once they land, rather than this workflow. This directory's job stops at
"here are the three commands and what each artifact is," matching the rest
of `samples/workflows/`'s command-loop framing.

## See also

- [`quonc` CLI reference — Neutral-atom options](../../../website/src/content/docs/reference/quonc.md#neutral-atom-options) —
  `--na-backend`, `--na-placer`, `--na-placement`, `--no-na-compact`.
- [`quonc` CLI reference — Emission options](../../../website/src/content/docs/reference/quonc.md#emission-options) —
  `--emit-na-schedule`, `--emit-na-graph`, `--emit-resource-report`.
- [`samples/neutral-atom/README.md`](../../neutral-atom/README.md) — pack
  #192's NA pedagogy home; the "why", not the "how to invoke".
