# Atom-indexed hybrid QEC interaction graph

## Context

ADR-0015 owns `LogicalQubitId` in `quon_qec` and re-exports it into `quon_na::graph`
as the interaction-graph vertex for the **bare-qubit** NA path (where a bare
logical qubit *is* what is being scheduled). CONTEXT distinguishes "Logical
qubit" (one encoded logical, `LogicalQubitId`) from "Atom" (one physical site
occupant, `PhysicalAtomId` after `expand_workload`).

The hybrid QEC path (`quon_na::qec_schedule`) feeds **physical atoms** into the
interaction graph by numerically casting `LogicalQubitId(atom.0)`. The cast hid
a missing seam: placers and AOD planners looked like they operated on logical
qubits while scheduling atoms, and report `logical_qubits` (block count) could
disagree with graph vertex-count semantics.

## Decision

The hybrid QEC path's interaction graph is explicitly **atom-indexed**.

1. `InteractionGraph`, `Interaction`, `InteractionEdge`, and `GraphError` are
   generic over a vertex id type `V` with a **default** `V = LogicalQubitId`,
   constrained by a small `VertexId` trait (`Copy + Clone + Debug + PartialEq +
   Eq + PartialOrd + Ord + Hash + Serialize + DeserializeOwned + index()`).
   The default keeps every existing bare-path call site (extract, placement,
   zoned, movement, compaction, entangling) compiling **unchanged** — no call
   site names the parameter unless it fixes `V = AtomVertexId`.
2. A new `AtomVertexId(pub u32)` newtype in `quon_na::graph` labels a physical
   atom's identity at the hybrid seam. It is structurally identical to
   `LogicalQubitId` but never shares a call site with it: the hybrid path fixes
   `V = AtomVertexId`; the bare path keeps `V = LogicalQubitId`.
3. `qec_schedule` builds `InteractionGraph<AtomVertexId>` with vertices
   `all_atoms.iter().copied().map(AtomVertexId::from)` and CNOT qubits
   `AtomVertexId::from_atom(cnot.control)` / `AtomVertexId::from_atom(cnot.target)`.
   **No `LogicalQubitId(atom.0)` numeric cast remains** in the hybrid graph seam.
4. `NaScheduleArtifacts` stays non-generic: at the schedule-artifact boundary
   the hybrid path projects `AtomVertexId -> LogicalQubitId(index)`, since by
   that point atom identity is baked into the schedule actions as `AtomId` and
   the vertex id is just a label. The projection loses no information (both
   newtypes wrap `u32`).

## Consequences

- Placers and AOD planners that consume the hybrid graph now name physical
  atoms (`AtomVertexId`) in their API surface, not logical qubits — code
  matches the CONTEXT glossary ("Logical qubit" vs "Atom").
- Reintroducing `LogicalQubitId(atom.0)` at the hybrid seam fails to compile:
  the hybrid helpers' signatures are `fn(...) -> Result<...,
  NaPipelineError<AtomVertexId>>` / `InteractionGraph<AtomVertexId>`.
- Resource report `logical_qubits` stays block-level (unchanged); the graph
  vertex set is atom-level, which is now explicit in the types.
- Bare-qubit path observable artifacts unchanged (default `V = LogicalQubitId`).
- Neutral-atom scheduling stays on `quantum.na` / `ScheduleLayer -> ScheduleSpec`
  (ADR-0007, ADR-0011, ADR-0009); this decision only changes the vertex-id
  *type* at one seam, not the schedule IR.

## References

- #318 — issue
- ADR-0015 — `LogicalQubitId` owned by `quon_qec`
- ADR-0007, ADR-0011 — canonical NA schedule IR (`ScheduleSpec`)
- ADR-0009 — unified `BackendTargetKind`
- CONTEXT "Logical qubit", "Atom", "ScheduleLayer"
