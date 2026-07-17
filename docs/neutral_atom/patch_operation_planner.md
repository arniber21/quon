# Patch-operation planner (issue #281)

The QEC compiler replaces the fixed logical-CX lattice-surgery template with a
generalized **patch-operation planner** that represents logical patches,
boundaries, ancilla patches, merge/split phases, patch measurements, and
adjacency constraints explicitly.

## Model

All logical operations (CX, measurement, bare merge/split) lower through one
shared planner model:

- **`Patch`** — one logical code patch (data or transitional ancilla)
- **`PatchBoundary`** — rough (ZZ) or smooth (XX) edge
- **`PatchEdge`** — left/right/top/bottom edge of a rotated surface patch
- **`PatchOperation`** — merge, split, measure-patch, measure-ancilla,
  prepare-ancilla, frame-update (all explicit, ordered)
- **`PatchPlan`** — ordered sequence of operations + patch registry

The plan is then lowered to physical rounds by `lower_patch_plan`, which reuses
the existing lattice-surgery geometry helpers (seam allocation, merge/split
round builders) but now drives them from the explicit plan.

## What changed

Previously, `logical_cx` lowered through a single hardcoded `expand_logical_cx`
function that produced a fixed L-shaped three-patch gadget. Logical measurement
(`measure_logical_z`/`measure_logical_x`) lowered through a separate ad hoc
`measure_logical_round` function.

Now both paths go through `patch_ops::plan_*` → `patch_ops::lower_patch_plan`,
which produces the same physical rounds but through one explicit model. The
fixed template (`expand_logical_cx`) is kept as a regression reference.

## Current limitations

This is a **scoped** planner, not a full patch-router research project:

- **Fixed L-shaped geometry**: the planner reuses the existing lattice-surgery
  geometry (L-shaped control|ancilla over target). Different patch/ancilla
  placements are supported through the plan's edge-pair parameters, but the
  physical layout is still the canonical L-shape.
- **Surface-code only**: merge/split operations require surface-code patches
  (same as the fixed CX template).
- **No routing**: the planner validates adjacency and boundary compatibility
  but does not route patches around each other or optimize placement.
- **No multi-patch merge**: each merge involves exactly two patches.
- **Seam stabilizer data**: the planner attaches seam atoms to the ancilla
  block with empty `data` fields in `StabilizerDef` (the original template
  populated these from the facing data atoms; this is a known simplification).

## Verification

The planner is validated by:

- `validate_plan` — checks patch registry, boundary compatibility, distinct
  patches in merges
- `lower_patch_plan` — produces physical rounds with correct merge/split/
  measure/frame phases
- Unit tests: 12 planner tests + 11 integration tests
- Existing CX regression tests pass unchanged through the new path
