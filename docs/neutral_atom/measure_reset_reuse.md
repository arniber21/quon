# Measure–Reset–Reuse: ancilla lifecycle model (issue #282)

This document describes the `quantum.na.reuse` op, the schedule verifier's
reuse-barrier checks, the `TemporalAtomMetrics` report struct, and the QEC
ancilla-reuse example. It covers the model assumptions and current limitations.

## 1. Motivation

In QEC workloads (surface code, repetition code, QLDPC), ancilla qubits are
consumed each syndrome round: the ancilla is entangled with data qubits,
measured for syndrome extraction, and must be reset before the next round.

A **no-reuse** schedule allocates a fresh physical atom (trap) for each round's
ancilla. Peak atom pressure grows linearly with the number of rounds. A
**reuse** schedule measures, resets, and reclaims the *same* atom across
rounds, keeping peak atom pressure constant regardless of round count.

Issue #282 makes this lifecycle a first-class schedule concept with explicit
`Reuse` events, verifier barriers, and resource-report visibility.

## 2. Schedule artifacts: `NeutralAtomAction::Reuse`

The `Reuse` variant of `NeutralAtomAction` represents an explicit reclaim of a
physical atom after its measurement and reset barriers have completed:

```rust
pub enum NeutralAtomAction {
    // ...
    Measure { atom: AtomId, basis: MeasurementBasis, duration_us: u64 },
    Reset   { atom: AtomId, duration_us: u64 },
    Reuse   { atom: AtomId, region: Option<ReuseRegionId>, duration_us: u64 },
    // ...
}
```

**`Reuse` is qubit-lifecycle reuse** (measure → reset → reclaim), *not* RAP
AOD movement "reuse". It is a bookkeeping reclaim event: it contributes
wall-clock time via the layer max-duration, and is tallied in
`TemporalAtomMetrics`, but does not participate in same-cycle occupancy
conflict checking.

The `region: Option<ReuseRegionId>` field optionally tags which logical reuse
region the atom is being reclaimed into (e.g. "ancilla patch A" vs "ancilla
patch B"). This is a labelling aid for resource reports and tooling — it does
not change schedule legality (the measure→reset barrier does that).

## 3. Verifier barriers

The `quantum.na` dialect verifier enforces the following ordering rules for
`Reuse` events (see `dialect.rs::verify_schedule_ordering`):

| Rule | `VerifyError` variant | Condition |
| --- | --- | --- |
| Reuse before any measure | `ReuseBeforeMeasure` | Atom is reused but was never measured. |
| Reuse before reset | `ReuseBeforeReset` | Atom was measured but not yet reset (or reset and reuse in the same cycle — the barrier hasn't crossed a cycle boundary). |
| Reuse without prior measure | `ReuseBeforeMeasure` | Atom was reset without ever being measured — there is no completed measurement barrier to reuse against. |
| Stale measurement dependency | `StaleMeasurementDependency` | Atom is reused a second time without a fresh measure→reset pair. |

### Legal lifecycle

A legal reuse requires:
1. **Measure** the atom at cycle `c_m`.
2. **Reset** the atom at cycle `c_r` where `c_r > c_m` (or `c_r ≥ c_m` with
   the reset in a later layer — but same-cycle reset+reuse is rejected because
   the barrier hasn't completed across a cycle boundary).
3. **Reuse** the atom at cycle `c_u` where `c_u > c_r`.

After reuse, a fresh measure→reset→reuse cycle is legal. A second reuse
without a fresh measure→reset is a stale-measurement dependency.

### Same-cycle interactions

Reuse does *not* participate in same-cycle occupancy or entangling-conflict
checks — it is a bookkeeping reclaim, not a physical gate. Same-cycle
measure↔use and reset↔use conflicts are still rejected (those rules predate
#282).

## 4. `TemporalAtomMetrics`

The `TemporalAtomMetrics` struct (in `report.rs`) makes peak-atom pressure and
qubit-lifecycle reuse visible:

```rust
pub struct TemporalAtomMetrics {
    pub peak_atoms: u64,
    pub allocated_atoms_series: Vec<u64>,
    pub measurement_count: u64,
    pub reset_count: u64,
    pub reuse_count: u64,
    pub reused_ancilla_count: u64,
}
```

### Allocation model

An atom is **allocated** from the first cycle it appears in any action and
stays allocated for the rest of the schedule (there is no free event within
a schedule — a physical trap holds its atom). Qubit **reuse** shows up as
*fewer distinct atom ids*: a schedule that measures→resets→reuses the same
ancilla across rounds allocates fewer atoms than a no-reuse variant that
consumes a fresh ancilla each round.

- `allocated_atoms_series[c]` is the count of distinct atoms seen in cycles
  `0..=c` (monotonic non-decreasing).
- `peak_atoms` is the maximum of the series (the last value).
- `measurement_count` / `reset_count` are **per-op** tallies (distinct from
  `ResourceReport::measurement_rounds` / `reset_rounds`, which count layers).
- `reuse_count` is the total number of `Reuse` actions.
- `reused_ancilla_count` is the count of distinct atoms reclaimed by at least
  one `Reuse` event.

`TemporalAtomMetrics` is embedded in `ResourceReport` as
`temporal_atom_metrics` and appears in JSON/Markdown output.

## 5. QEC example: ancilla reuse across rounds

The `qec_ancilla_reuse` example binary builds two simplified
surface-code-like syndrome-extraction schedules:

- **No-reuse**: each round allocates a fresh ancilla. Peak atoms = 1 + rounds.
- **Reuse**: the same ancilla is measured, reset, and reused each round.
  Peak atoms = 2 regardless of rounds.

Run it:

```sh
cargo run -p quon_na --example qec_ancilla_reuse
```

Sample output (3 rounds):

```
=== QEC Ancilla Reuse vs No-Reuse (3 rounds) ===

Metric                             No-Reuse        Reuse
------------------------------------------------------
peak_atoms                                4            2
measurement_count                         3            3
reset_count                               0            2
reuse_count                               0            2
reused_ancilla_count                      0            1

Peak-atom reduction: 4 → 2 (2 fewer atoms)

Allocated-atoms series (no-reuse): [2, 2, 3, 3, 4, 4]
Allocated-atoms series (reuse):    [2, 2, 2, 2, 2, 2, 2, 2, 2, 2]
```

## 6. Model assumptions and limitations

### Assumptions

- **Atom allocation is monotonic**: an atom, once allocated, stays allocated
  for the entire schedule. There is no physical free event. Reuse reduces peak
  pressure by recycling *existing* atoms, not by freeing them.
- **Reuse requires completed barriers**: a `Reuse` is only legal after a
  measure→reset pair has completed across cycle boundaries. The verifier
  enforces this; it is not advisory.
- **`region` is a label only**: `ReuseRegionId` tags which logical region an
  atom is reclaimed into for reporting. It does not affect legality or
  scheduling.
- **Reuse is not AOD movement reuse**: this is qubit-lifecycle reuse
  (measure→reset→reclaim), not RAP-style AOD trap reuse or rearrangement.

### Current limitations

- **No automated reuse insertion**: the scheduler does not yet *insert*
  `Reuse` events automatically. Schedules must include them explicitly
  (via `ActionSpec::Reuse` / `NeutralAtomAction::Reuse`). Future work could
  add a pass that detects measure→reset patterns and inserts reuse events.
- **No cross-schedule free event**: the allocation model has no free event
  within a schedule. A schedule that truly releases an atom (e.g. moves it to
  a storage zone and stops using it) still counts it as allocated. This is
  conservative and may overcount in scenarios with long idle periods.
- **Same-cycle reset+reuse is rejected**: the verifier requires the reset
  barrier to complete across a cycle boundary before reuse. This is stricter
  than some hardware models that allow pipelined reset+reuse within one
  cycle, but it matches the hard-barrier semantics used for `Wait` ops.
- **No spatial reuse tracking**: `ReuseRegionId` is a logical label; the
  model does not track which physical site or trap region an atom is
  reclaimed into. Spatial reuse analysis (à la RAP) is out of scope.
- **Per-op vs per-layer counts**: `measurement_count` / `reset_count` in
  `TemporalAtomMetrics` count individual ops, while
  `ResourceReport::measurement_rounds` / `reset_rounds` count layers that
  contain at least one such op. These are distinct metrics for different
  purposes.

## 7. Testing

Tests covering all acceptance criteria are in:

- `quon_na/tests/quantum_na_dialect.rs` — verifier rejection cases (legal
  reuse, reuse-before-measure, reuse-before-reset, stale-measurement-dependency,
  same-cycle reset+reuse, reuse-after-reset-without-measure, multi-round reuse
  with distinct regions).
- `quon_na/tests/measure_reset_reuse.rs` — `TemporalAtomMetrics` field
  coverage and QEC reuse-vs-no-reuse peak-atom comparison.
