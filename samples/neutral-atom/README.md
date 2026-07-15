# Neutral-atom samples

Narrative walkthroughs of the neutral-atom (NA) compilation path: zoned vs.
flat movement planning, the RAP placer, and NA/QEC hybrid scheduling.

## Status

Owned by pack **#192** (NA pedagogy). This directory currently holds only a
`README.md` — the seed registered in `samples/catalog.yaml` for this
category (`neutral-atom/repetition-d3-memory`) **links** to the existing
canonical program in [`examples/na_qec/repetition_d3_memory.qn`](../../examples/na_qec/repetition_d3_memory.qn)
rather than forking a copy into `samples/`, per ADR-0025: `examples/na_qec/`
stays the compiler/QEC examples home and is linked from here, not moved.

#192 is expected to add the zoned-vs-flat and RAP-placer walkthroughs
directly under this directory, each with its own catalog row, while
continuing to link (not fork) any `.qn` source that already has a canonical
home in `examples/na_qec/` or `test/na/`.

## Seeds

| Catalog id | Path | Notes |
| --- | --- | --- |
| `neutral-atom/repetition-d3-memory` | [`examples/na_qec/repetition_d3_memory.qn`](../../examples/na_qec/repetition_d3_memory.qn) | Distance-3 repetition-code memory on the NA/QEC hybrid path (ADR-0016). Linked, not copied. |

## Do not extend this stub

Coordinate with #192 before adding real narrative content here — one
canonical `.qn` per story (see [`../CONTRIBUTING.md`](../CONTRIBUTING.md)).
