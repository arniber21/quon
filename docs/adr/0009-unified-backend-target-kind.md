# Unify `BackendTarget` behind a `TargetKind` enum instead of separate target types per architecture

`BackendTarget` gains a `TargetKind` discriminant (`Fixed` for today's gate-model
descriptor, `NeutralAtomReconfigurable(NeutralAtomTarget)` for the new backend, room
for future architecture families) rather than shipping `NeutralAtomTarget` as a wholly
separate type with its own `quonc` flag and loader.

The two shapes have almost nothing in common — `Fixed` carries a connectivity graph,
native gate set, and noise model; `NeutralAtomReconfigurable` carries zones, grid
geometry, movement model, and a cost model. A truly separate type would have been
simpler to build. This is a deliberate bet on Quon's longer-term identity as unified
quantum compiler infrastructure spanning multiple architecture families, not just an
MLIR/OpenQASM gate-model compiler — worth the extra indirection now so `--target`
stays one concept as more architecture families are added later.

## Considered Options

**Separate `NeutralAtomTarget` type, separate `--na-target` CLI flag, separate loader.**
Simpler for this one addition, but doesn't scale: each new architecture family would
add its own flag/loader/type, and `quonc --target` stops being a single unified concept.

## Consequences

- Each `TargetKind` variant owns a fully independent payload — no fields are hoisted to
  a shared level except `id: String`. Do not add a "shared" field to save duplication;
  that reintroduces the awkward-`None`-fields problem this ADR exists to avoid.
- `backend::json::load`/`from_str` and `TargetDescriptor` (backend/src/descriptor.rs,
  backend/src/json.rs) need to dispatch on kind rather than assuming the current flat
  `BackendTarget` shape.
- `quonc --target <path>` stays a single flag for all architecture families; the loader
  determines the kind from the JSON rather than the CLI needing to know in advance.
