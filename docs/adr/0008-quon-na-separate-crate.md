# Put neutral-atom passes and dialect in a new `quon_na` crate, not inside `mlir_bridge`

Neutral-atom placement, scheduling, movement planning, and the `quantum.na` dialect
(ADR-0007) live in a new workspace crate `quon_na`, depending on `quon_core` (shared
types) and `mlir_bridge` (dialect registration primitives) — not as new modules inside
`mlir_bridge` itself.

`mlir_bridge` today is scoped to the core compiler: Melior wrappers, `quantum.circ`/
`quantum.dynamic` dialect registration, optimization passes, and the OpenQASM emitter.
Neutral-atom compilation is a second, optional architecture family with its own
substantial pass pipeline (interaction-graph extraction, placement, gate layering,
movement planning, zoned routing, schedule compaction, resource estimation). Folding
that into `mlir_bridge` would blur the crate's purpose and make "optional backend path"
untrue at the build-graph level — every build of the core compiler would compile
neutral-atom passes whether or not they're used.

## Consequences

- `quon_na` is the crate to point to in outreach material ("here's the neutral-atom
  backend") — it should be buildable/testable independently of the rest of the workspace.
- `quon_na` depends on `mlir_bridge` for dialect registration machinery, so it is not
  fully decoupled — changes to how `quantum.circ` registers itself (ADR-0004) can affect
  `quon_na`.
