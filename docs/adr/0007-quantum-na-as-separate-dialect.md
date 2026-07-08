# Model neutral-atom scheduling as a separate `quantum.na` dialect, not `quantum.physical` annotations

Neutral-atom placement, movement, and time-layered scheduling will be modeled as a new
MLIR dialect (`quantum.na`) with its own ops (`alloc_atom`, `place`, `move`, `entangle`,
`measure`, `layer`, `schedule`), rather than as attributes on existing `quantum.dynamic`
ops in the style of `quantum.physical` (see `CONTEXT.md`).

This deviates from the precedent set by `quantum.physical`, where routing/scheduling
decisions for generic hardware backends are represented as attributes on
`quantum.dynamic` ops rather than a new dialect. Issue #82 explicitly recommended the
annotation approach to avoid dialect explosion.

## Considered Options

**Annotate `quantum.dynamic`** (consistent with `quantum.physical` and issue #82).
Rejected: atom movement, time-layered scheduling, and zone occupancy don't fit naturally
as scalar attributes, and a from-scratch dialect is a stronger, more legible IR-design
artifact that demonstrates compiler-engineering skill in this niche.

## Consequences

- `quantum.na` should follow the same registration mechanism as `quantum.circ`
  (ADR-0004): unregistered dialect, generic op form, Rust verifier callbacks — not
  TableGen/C++, to stay consistent with the rest of the workspace.
- Two different representations now exist for "physical/hardware-aware" IR
  (`quantum.physical` attributes for generic backends, `quantum.na` ops for neutral-atom).
  A future reader may ask why neutral-atom didn't reuse the attribute model — this ADR is
  the answer.
