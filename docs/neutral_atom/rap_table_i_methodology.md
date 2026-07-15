# RAP Table I regression methodology (#111)

External regression anchor for the zoned NA backend (#107), reproducing one
row of [RAP] (Stade, Lin, Cong, Wille, ICCAD 2025, arXiv:2505.22715) Table I,
Sec. VI-B. Companion to
[`literature_notes.md`](./literature_notes.md#rap--stade-lin-cong-wille-iccad-2025-arxiv250522715--the-reproduced-paper)
and
[`architecture_model.md`](./architecture_model.md) §7 (the zoned model) / §8.6
(constant provenance). Distinct from the QEC compiler-ablation benchmarks of
#254 — see [Distinct from QEC benchmarks](#distinct-from-qec-benchmarks-254)
below.

## Two-phase HITL plan

| Phase | Who | What |
| --- | --- | --- |
| **1 (this doc / agent-implementable)** | Agent | Land the circuit, pinned target, this doc, and a metric **dump** test that records agnostic vs aware rearrangement steps/time without failing CI on mismatch. |
| **2 (human then enforce)** | Human sign-off, then agent | Review whether the Phase 1 numbers are a *mechanism-legitimate* reproduction (not just numerically close by accident). Only then flip on hard tolerance asserts (`QUON_RAP_TABLE_I_ENFORCE=1`, below). |

Phase 1 deliberately does not hard-assert against the published numbers. See
[Phase 1 finding](#phase-1-finding-aware--agnostic-on-this-targetcircuit-pair)
for why that caution was warranted.

## Anchor row

[RAP] Table I, `ising`, n = 42 qubits:

| Metric | Routing-agnostic (ZAC-style baseline) | Routing-aware ([RAP]) |
| --- | --- | --- |
| Rearrangement steps | 22 | 9 |
| Rearrangement time | 3.1 ms | 1.6 ms |

Circuit pre-flight (both placer modes see the same circuit): **82** two-qubit
gates, **4** entangling layers. n = 98 (23 → 12 steps in the same table row
group) is **optional / local only, not CI** — see
[n = 98 (optional, local)](#n--98-optional-local).

## Metric mapping

| [RAP] Table I column | `quon_na` field | Where it comes from |
| --- | --- | --- |
| Rearrangement steps | `ResourceReport.rearrangement_steps` | Count of AOD-compatible movement groups emitted by [`schedule_zoned`](../../quon_na/src/zoned.rs) |
| Rearrangement time | `ResourceReport.rearrangement_time_us` | Sum of √-law durations ([`movement_duration_us`](../../quon_na/src/zoned.rs)) per movement group |
| Two-qubit gate count (82) | `ResourceReport.entangle2_count` | Count of `Entangle2` schedule actions — one per circuit-level 2Q gate, placer-independent |
| Entangling layers (4) | `ResourceReport.rydberg_stages` | Count of distinct schedule layers containing an entangling action |

`entangle2_count` and (for this specific fixture/target pairing — see next
section) `rydberg_stages` are **placer-independent pre-flight checks**: they
must hold before either placer's `rearrangement_steps`/`_time_us` are
compared to the paper. A wrong circuit (wrong gate count or wrong layer
structure) invalidates the whole comparison before it starts, so the
pre-flight test fails first and separately from the metric dump.

## Timing model

`t = √(d / a) + 15 µs` per trap transfer (one load + one store per moved
atom), with `a = 2750 m/s²` — [RAP] Sec. VI-B's move-time law, already the
one `quon_na` implements
([`zoned::movement_duration_us`](../../quon_na/src/zoned.rs);
architecture_model.md §5 "Movement timing"). This is the paper's √-law, **not**
the newer jerk-limited timing model used by some QMAP evaluation scripts
(literature_notes.md's [RAP] caveats) — do not swap the model to "match" QMAP
tooling; that would stop reproducing *this* paper's Table I.

## Pre-flight fixture: `test/na/ising_n42.qn`

Hand-authored Quon (not vendored QASM), two Trotter-shaped steps of a
42-qubit nearest-neighbor chain. Each step's "ZZ layer" is split into two
explicit `for` loops — even bonds `(0,1)(2,3)…(40,41)` (21 gates, a perfect
matching), then odd bonds `(1,2)(3,4)…(39,40)` (20 gates, also a perfect
matching) — using native `CZ` gates (not `Rzz`, which would decompose to two
`CNOT`s per gate via `decompose_rzz` and double the count to 164). Full
rationale is in the file's header comment; the load-bearing fact is that
`quon_na`'s dependency-DAG ASAP scheduler assigns `dag_layer` by real
last-qubit-use (not textual/program order), so two gates that touch disjoint
qubits land in the same layer regardless of which `for` loop emitted them.
Two Trotter steps × (1 even layer + 1 odd layer) = 4 layers; 2 × (21 + 20) =
82 gates. Verified empirically against the compiled circuit — this is exactly
why the pre-flight test compiles the real `.qn` file rather than asserting
against a hand-rolled graph fixture.

Single-qubit `Rx` gates round out the Trotter shape but are invisible to
two-qubit interaction-graph extraction (`quon_na::extract`), so they cannot
perturb the 82/4 counts.

## Pinned target: `targets/neutral_atom/rap_table_i.json`

Byte-for-byte freeze of `targets/neutral_atom/generic_rna_v0.json` (same
zone geometry, movement model, timing, fidelity) except `id`. CI for this
regression loads `rap_table_i.json`, never `generic_rna_v0.json` — so tuning
the generic target for an unrelated purpose cannot silently move this
anchor's numbers. See
[`targets/neutral_atom/README.md`](../../targets/neutral_atom/README.md#rap_table_ijson-issue-111).

## Tests

`quonc/tests/rap_table_i.rs`:

- `ising_n42_preflight_gate_and_layer_counts` — **not** `#[ignore]`d; part of
  the default `cargo test --workspace` / `just ci-rust` gate. Compiles
  `test/na/ising_n42.qn` against `rap_table_i.json` with the (fast) default
  routing-agnostic placer and asserts `entangle2_count == 82` and
  `rydberg_stages == 4`. For this fixture/target pairing there is no
  placement-induced deferral (ample entanglement-zone capacity, no spacing
  conflicts), so the post-placement `rydberg_stages` here equals the
  circuit's own dependency-DAG layer count — the same placer-independent
  quantity either mode would report. Fails first, before any placer
  comparison, if the circuit or target regresses.
- `ising_n42_dumps_both_placer_rearrangement_metrics` — `#[ignore]`d (see
  [Runtime](#runtime--ci-wiring) below). Runs `schedule_zoned` via `quonc`
  under both `--na-placer routing-agnostic` and `--na-placer routing-aware`,
  prints a comparison table (steps, time, ×-agnostic-over-aware ratio) to
  stdout, and:
  - **Default (`QUON_RAP_TABLE_I_ENFORCE` unset or not `1`):** dump-only.
    Re-asserts the placer-independent pre-flight counts (belt-and-suspenders)
    and otherwise never fails on a numeric mismatch against the published
    row. Prints a `WARNING` banner (not a failure) if aware is not at least
    as good as agnostic, since [RAP]'s whole point is a meaningful aware
    improvement.
  - **`QUON_RAP_TABLE_I_ENFORCE=1` (Phase 2, after human sign-off):** hard
    tolerance asserts — see [Tolerances](#tolerances-for-phase-2-enforcement).

### Runtime / CI wiring

Routing-aware placement is a best-first search
([`assign_aware_legal`](../../quon_na/src/zoned.rs), budget
`AWARE_NODE_BUDGET = 100_000` expansions/layer) over this target's 340
entanglement-zone pairs. Measured on this fixture: routing-agnostic ≈ 1 s,
routing-aware ≈ 90 s in a `--release` build (debug is substantially slower).
That is too slow for the default `cargo test --workspace` gate (which runs in
debug mode), so the dump test is `#[ignore]`d and wired into `just ci-rust` as
a dedicated `--release --include-ignored` step — the same pattern
`quon_lsp/tests/smoke.rs` uses for the tooling job. This still satisfies "n=42
in CI" (locked decision): it runs every `just ci-rust` / CI `rust` job, just
in release mode via an explicit step instead of the default debug test sweep.

```bash
# What CI runs (see justfile's `ci-rust` recipe)
cargo test --release -p quonc --test rap_table_i -- --include-ignored --nocapture

# Fast pre-flight only (part of the default gate)
cargo test -p quonc --test rap_table_i preflight
```

## Phase 1 finding: aware == agnostic on this target/circuit pair

Measured on `rap_table_i.json` (default row-major initial placement):

| Placer | Rearrangement steps | Rearrangement time (µs) | Rydberg stages | Entangle2 count |
| --- | --- | --- | --- | --- |
| Routing-agnostic | 23 | 3049 | 4 | 82 |
| Routing-aware | 23 | 3049 | 4 | 82 |

Both placers produce **identical** schedules here — no aware improvement,
let alone [RAP]'s reported 22 → 9 (−59%). This is a legitimate Phase 1
finding, not a bug to silently "fix" by tuning numbers: with `rap_table_i`'s
generous entanglement-zone capacity (340 pairs for at most 21 simultaneous
gates) and a simple nearest-neighbor chain, the distance-minimizing greedy
assignment already finds a schedule the routing-aware search cannot improve
on — there is no routing *contention* for "aware" to be aware of. Reproducing
the paper's reported delta faithfully likely requires either a tighter
entanglement-zone geometry (closer to what forces genuine placement/routing
trade-offs) or a different qubit-to-storage-site initial mapping, or both.
**This is exactly the kind of judgment call Phase 2 human sign-off exists
for** — before enabling `QUON_RAP_TABLE_I_ENFORCE=1`, a human must decide
whether closing this gap (and how) still counts as reproducing [RAP]'s
*mechanism*, or whether the target/placement setup needs to change first.

## Tolerances (for Phase 2 enforcement)

Locked decisions (not yet enforced — dump-only in Phase 1):

- **±2 steps** on each placer's `rearrangement_steps` vs its published value
  (22 agnostic / 9 aware).
- **±10%** on each placer's `rearrangement_time_us` vs its published value
  (3.1 ms agnostic / 1.6 ms aware).
- **Meaningful aware improvement**: aware must beat agnostic by a documented
  relative floor (not just `<`), reflecting [RAP]'s reported −59% steps /
  −49% time story. The exact floor is a Phase 2 decision — see the finding
  above for why picking one now, before the mechanism gap is resolved, would
  just be enforcing today's (non-reproducing) numbers.

`QUON_RAP_TABLE_I_ENFORCE=1` is read by
`ising_n42_dumps_both_placer_rearrangement_metrics`; Phase 1 wires the flag
check but leaves the concrete hard asserts as `todo!`-free no-ops beyond the
placer-independent pre-flight re-check, since enforcing tolerances against
numbers we already know don't reproduce the paper's mechanism (see the
finding above) would be worse than not enforcing at all. Phase 2 fills in the
actual ±2 / ±10% / floor comparisons once a human has signed off on which
setup (if any changes) makes the reproduction legitimate.

## n = 98 (optional, local)

[RAP] Table I also reports `ising` n = 98: 23 → 12 rearrangement steps. Not a
CI row (locked decision) — no fixture is checked in for it in Phase 1. A
future local-only addition would follow the same even/odd `for`-loop
construction at `n = 98` (49 even-group gates, 48 odd-group gates per step).

## Distinct from QEC benchmarks (#254)

The QEC compiler-ablation harness (`python/quon_qec_benchmarks.py`,
[`qec_benchmark_methodology.md`](./qec_benchmark_methodology.md)) reuses
RAP-style headline field *names* (rearrangement time, Rydberg stages, …) as a
**methodology style** only. QEC CSV rows are tagged
`methodology_anchor=issue_111_rap_table_i_physical_na_only` precisely so they
are never mistaken for a Table I numeric claim — see
`qec_benchmark_methodology.md`'s "Experiment class" table and
`literature_notes.md`'s "[RAP] Caveats for the reproduction" for the existing
do-not-conflate language this doc extends. Nothing in #254 reports or should
report the 22/9 or 82/4 numbers above; only `rap_table_i.json` +
`ising_n42.qn` do.

## Refs

- Issue #111; blocked-by (done): #107, #110
- [RAP] Stade, Lin, Cong, Wille, ICCAD 2025, arXiv:2505.22715, Table I / Sec.
  VI-B
- `docs/neutral_atom/literature_notes.md` ([RAP] section)
- `docs/neutral_atom/architecture_model.md` §5 (movement timing), §7 (zoned
  model), §8.6 (constant provenance)
- `targets/neutral_atom/README.md` (`rap_table_i.json` note)
- QEC benchmarks: issue #254, `docs/neutral_atom/qec_benchmark_methodology.md`
