# `quantum.na` is the canonical schedule IR; `ScheduleLayer` is planner-internal; JSON is a view

ADR-0007 chose a separate `quantum.na` MLIR dialect as the scheduling IR for the
neutral-atom backend. The driver path that actually shipped (#112 / PR #166) bypassed it:
`quantum.dynamic` → extract → Rust planners → `Vec<ScheduleLayer>` → JSON, leaving the
dialect (~1.4k LOC of `ScheduleSpec` / builders / verifiers in `quon_na/src/dialect.rs`)
tested and round-trippable but unregistered and unused by `quonc` — deleting `dialect.rs`
would not have broken the NA CLI. Issue #211 forced the question of which surface is the
compiler's schedule IR.

**Decision (Option B of #211): `quantum.na` is canonical.** The driver's primary
artifact for a scheduled neutral-atom program is `quantum.na` textual MLIR, produced by
lowering the planners' output through a single named converter
(`Vec<ScheduleLayer>` + layout → `ScheduleSpec`, implemented by #167). `ScheduleLayer`
remains the planners' in-memory working representation — extract / place / entangle /
move-or-zoned / compact keep producing and validating it — but it is never serialized as
a primary artifact. JSON schedule emit (`--emit-na-schedule`) is demoted to a
debug/visualization view (kept for tooling such as #113), not a second source of truth.

## Considered Options

**Option A — `ScheduleLayer` canonical**, dialect becomes a printer/parser adapter or
test-only code, amend ADR-0007. Rejected: it abandons the strongest IR-design artifact in
the backend (ops, verifiers, round-trip parsing already exist and work), reduces the
hardware-target path to ad-hoc JSON, and reverses ADR-0007 for no capability gain.

**Option C — permanent dual ownership with a documented adapter**, `ScheduleLayer` as
planner IR and the dialect as serialization IR, coexisting indefinitely. Rejected as an
end state: it institutionalizes two sources of truth for zone/layer concepts. The
converter seam it prescribes is real, but under Option B it is a lowering step with a
clear direction (planner representation → canonical IR), not a peer-to-peer bridge.

## Consequences

- #167 is the implementation of this decision: the `ScheduleLayer → ScheduleSpec`
  converter plus a `quonc` flag emitting `quantum.na` MLIR for neutral-atom targets.
- The deletion test flips: `dialect.rs` becomes load-bearing for the NA driver path;
  the converter module is the only place allowed to translate between the two
  representations. No new zone/layer types may be introduced outside it.
- JSON emit stays available but its schema is a view contract for visualization/tooling,
  free to lag the dialect; FileCheck tests over the MLIR form are the regression surface.
- `quonc` help/docs must present MLIR emit as the primary NA artifact and JSON as debug
  output.
- ADR-0007 stands unamended; this ADR records that its dialect is now the driver's
  canonical schedule IR rather than a parallel bet.
