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
[Phase 1 finding](#phase-1-finding-routing-aware-falls-back-to-greedy-on-this-targetcircuit-pair)
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
| Rearrangement time | `ResourceReport.rearrangement_time_us` | Sum of √-law **move-only** durations ([`movement_duration_us`](../../quon_na/src/zoned.rs)) per movement group — see [Timing model](#timing-model) for why this is deliberately *not* `rearrangement_time_us + transfer_time_us` |
| Two-qubit gate count (82) | `ResourceReport.entangle2_count` | Count of `Entangle2` schedule actions — one per circuit-level 2Q gate, placer-independent |
| Entangling layers (4) | `ResourceReport.rydberg_stages` | Count of distinct schedule layers containing an entangling action |
| *(not in the paper; #111 review instrumentation)* | `ResourceReport.aware_search_completed_layers` / `aware_search_fell_back_layers` | Per-layer count of whether [`assign_aware_legal`](../../quon_na/src/zoned.rs)'s best-first search found a full assignment within budget, or exhausted it and fell back to [`assign_greedy_legal`](../../quon_na/src/zoned.rs) — see [Phase 1 finding](#phase-1-finding-routing-aware-falls-back-to-greedy-on-this-targetcircuit-pair) |

`entangle2_count` and (for this specific fixture/target pairing — see next
section) `rydberg_stages` are **placer-independent pre-flight checks**: they
must hold before either placer's `rearrangement_steps`/`_time_us` are
compared to the paper. A wrong circuit (wrong gate count or wrong layer
structure) invalidates the whole comparison before it starts, so the
pre-flight test fails first and separately from the metric dump. The dump
test additionally hard-asserts both of these on **both** placers (not just
routing-agnostic), since a routing-aware-only circuit regression would
otherwise slip past the (agnostic-only, deliberately fast) pre-flight test.

## Timing model

[RAP] Sec. VI-B's rearrangement-time metric is `t = √(d/a) + 15 µs per
transfer`, with `a = 2750 m/s²` (literature_notes.md's [RAP] section). This is
the paper's √-law, **not** the newer jerk-limited timing model used by some
QMAP evaluation scripts (literature_notes.md's [RAP] caveats) — do not swap
the model to "match" QMAP tooling; that would stop reproducing *this* paper's
Table I.

**How this maps onto the two separate `ResourceReport` fields (#111 review
finding — an earlier draft of this doc conflated them):**

- `rearrangement_time_us` is **move-only**: the √-law duration
  ([`movement_duration_us`](../../quon_na/src/zoned.rs)) of each AOD-coupled
  movement group, summed across groups. It does **not** include any transfer
  overhead.
- `transfer_time_us` is a **separate aggregate**: `quon_na` emits one
  `Transfer` schedule action per atom per load and per store (so a
  two-atom movement group contributes 4 transfer actions), each with duration
  `arch.trap_transfer_us` (15 µs on this target). `ResourceReport::from_layers`
  *sums* those action durations — it is a rate×count-style aggregate (used
  for the physical error-budget, `movement × trap_transfers`), not a
  per-group wall-clock quantity. A `k`-atom group's transfer overhead is
  therefore counted here as `2k × 15 µs`, not the `2 × 15 µs` that a single
  simultaneous AOD load/store grab would cost in wall-clock time.

**Phase 1 decision (documented, not silently chosen): the paper comparison
uses `rearrangement_time_us` (move-only) only.** `rearrangement_time_us +
transfer_time_us` is *not* used as the comparison quantity, because
`transfer_time_us`'s per-atom-instance accounting (previous bullet) would
inflate the transfer contribution above what the paper's per-group `+15 µs
per transfer` term implies whenever a movement group has more than one atom
— that would make the comparison *less* honest, not more, despite looking
more "complete." The dump test's printed table reflects this: it labels the
compared column `time_us(move)` and prints `transfer_us` alongside only for
transparency, never folded into the paper comparison. Closing this gap
properly (a per-group wall-clock transfer field) is left to a follow-up;
Phase 1 does not hack around it by picking whichever quantity numerically
looks closer to the paper.

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
  prints a comparison table (steps, move-only time, transfer time,
  ×-agnostic-over-aware ratio, aware search completed/fell-back status) to
  stdout, and:
  - **Hard, regardless of `QUON_RAP_TABLE_I_ENFORCE`** (structural — under
    `just ci-rust`'s `set -euo pipefail` these are what actually gate CI, per
    the #111 review): `entangle2_count == 82` and `rydberg_stages == 4` on
    **both** placers (not just agnostic — closing the gap the fast
    agnostic-only pre-flight test above deliberately leaves open), and the
    routing-aware search's `aware_search_completed_layers` /
    `aware_search_fell_back_layers` diagnostic fields are present and
    internally consistent (agnostic reports `(0, 0)`; aware reports at least
    one outcome). Prints a `FALLBACK` banner (not a failure — Phase 1 stays
    soft on the *numeric* mismatch, see below) whenever
    `aware_search_fell_back_layers > 0`.
  - **Soft (`QUON_RAP_TABLE_I_ENFORCE` unset or not `1`):** never fails on a
    numeric mismatch against the published row. Prints a `WARNING` banner
    (not a failure) if aware is not at least as good as agnostic, since
    [RAP]'s whole point is a meaningful aware improvement.
  - **`QUON_RAP_TABLE_I_ENFORCE=1` (Phase 2, after human sign-off):** hard
    tolerance asserts — see [Tolerances](#tolerances-for-phase-2-enforcement).

`quon_na/src/zoned.rs`'s `mod tests` additionally has
`aware_search_completes_and_beats_greedy_on_contended_pairs` — a small
(2-gate, 2-pair, hand-constructed positions), fixture-independent unit test
proving the aware search mechanism itself (not this specific 42-qubit
circuit) can both **complete** (`AwareSearchOutcome::Completed`, not a
fallback) and find a strictly better joint assignment than the greedy
placer on a genuinely contended layout. This is the evidence backing the
[Phase 1 finding](#phase-1-finding-routing-aware-falls-back-to-greedy-on-this-targetcircuit-pair)'s
claim that the `ising_n42` gap is plausibly budget/scaling, not "nothing to
find."

### Runtime / CI wiring

Routing-aware placement is a best-first search
([`assign_aware_legal`](../../quon_na/src/zoned.rs), budget
`AWARE_NODE_BUDGET = 100_000` expansions/layer) over this target's 340
entanglement-zone pairs. Measured on this fixture: routing-agnostic ≈ 1 s,
routing-aware ≈ 90–125 s in a `--release` build (debug is substantially
slower) — the search burns its full budget on every one of the circuit's
4 layers without finding a full assignment (see the
[Phase 1 finding](#phase-1-finding-routing-aware-falls-back-to-greedy-on-this-targetcircuit-pair)),
so this is worst-case-budget time, not early-termination time. That is too
slow for the default `cargo test --workspace` gate (which runs in debug
mode), so the dump test is `#[ignore]`d and wired into `just ci-rust` as a
dedicated `--release --include-ignored` step — the same pattern
`quon_lsp/tests/smoke.rs` uses for the tooling job. This still satisfies "n=42
in CI" (locked decision): it runs every `just ci-rust` / CI `rust` job, just
in release mode via an explicit step instead of the default debug test sweep.
`ci-rust`'s recipe carries `set -euo pipefail` so a failure in this step (or
any earlier one) actually fails the job instead of being swallowed.

```bash
# What CI runs (see justfile's `ci-rust` recipe)
cargo test --release -p quonc --test rap_table_i -- --include-ignored --nocapture

# Fast pre-flight only (part of the default gate)
cargo test -p quonc --test rap_table_i preflight
```

## Phase 1 finding: routing-aware falls back to greedy on this target/circuit pair

**Corrected finding (#111 review; an earlier draft of this doc claimed "no
routing contention" — that claim was wrong and has been retracted).** Measured
on `rap_table_i.json` (default row-major initial placement), including the
new `aware_search_completed_layers` / `aware_search_fell_back_layers`
instrumentation:

| Placer | Rearrangement steps | Rearrangement time_us (move-only) | Rydberg stages | Entangle2 count | Aware search |
| --- | --- | --- | --- | --- | --- |
| Routing-agnostic | 23 | 3049 | 4 | 82 | n/a |
| Routing-aware | 23 | 3049 | 4 | 82 | **0 of 4 layers completed; 4 of 4 fell back to greedy** |

Both placers produce **identical** schedules — but *not* because there is "no
routing contention." The routing-aware `assign_aware_legal` search
([`quon_na/src/zoned.rs`](../../quon_na/src/zoned.rs)) exhausted its
`AWARE_NODE_BUDGET` (100,000 expansions/layer) on **all four** of this
circuit's entangling layers and fell back to `assign_greedy_legal` every
single time, per the instrumentation added in the #111 review fix — so
routing-aware's output on this fixture is, byte-for-byte, greedy's output.
That is the actual mechanism, and it is unsurprising in retrospect: the
search is uniform-cost (`h = 0`, [RAP] Sec. IV-B's inadmissible heuristic is
not implemented — see the [Tests](#tests) / follow-up notes), each of this
circuit's four layers has 20–21 simultaneous gates, and this target's
entanglement zone offers 340 candidate pairs — the reachable state space per
layer is astronomically larger than 100,000 uniform-cost expansions, so the
search **never gets close to finishing** before the budget cap fires.

This means the earlier "no routing contention, greedy already optimal"
framing was an unverified guess dressed up as a finding: with the search
never completing, there was no way to know whether an *exhaustive* aware
search would have beaten greedy here — only that the *budget-limited* search
didn't. [`aware_search_completes_and_beats_greedy_on_contended_pairs`](../../quon_na/src/zoned.rs)
(a new, tiny, non-fixture-dependent unit test added in the #111 review fix)
demonstrates the search mechanism itself *can* find a strictly better joint
assignment than greedy when it completes — so the gap on `ising_n42` is
plausibly a **budget/scaling** problem on this fixture, not a "there was
nothing to find" problem, though Phase 1 stops short of proving that for this
specific 21-gate/340-pair layer (that would require either a real heuristic
so the search terminates in reasonable time, or raising the budget and
re-measuring, both left to Phase 2 / a follow-up).

**This is exactly the kind of judgment call Phase 2 human sign-off exists
for** — before enabling `QUON_RAP_TABLE_I_ENFORCE=1`, a human must decide how
to close this gap: a real A* heuristic (Eqs. (3)–(5), not yet implemented),
a larger budget, a tighter entanglement-zone geometry, or some combination —
and whether the result still counts as reproducing [RAP]'s *mechanism*.
Phase 1's contract is only to make the fallback **visible** (the dump test
now hard-asserts the diagnostic fields are present and prints a `FALLBACK`
banner whenever `aware_search_fell_back_layers > 0`), not to fix it.

## Tolerances (for Phase 2 enforcement)

Locked decisions (implemented behind the enforce flag; **off by default** in
Phase 1):

- **±2 steps** on each placer's `rearrangement_steps` vs its published value
  (22 agnostic / 9 aware).
- **±10%** on each placer's `rearrangement_time_us` vs its published value
  (3.1 ms agnostic / 1.6 ms aware).
- **Meaningful aware improvement**: aware must beat agnostic by a documented
  relative floor (not just `<`), reflecting [RAP]'s reported −59% steps /
  −49% time story. The exact floor wired today is whatever
  `rap_table_i.rs` encodes; Phase 2 may revise it after human sign-off.

`QUON_RAP_TABLE_I_ENFORCE=1` is read by
`ising_n42_dumps_both_placer_rearrangement_metrics`. The ±2 / ±10% / floor
asserts are **already implemented** in that test — they are not stubs or
no-ops. With the flag unset (CI default), those numeric asserts are skipped
and only the dump + structural checks run. With the flag set to `1` today,
the test **will hard-fail** on this fixture because routing-aware still falls
back to greedy on every layer (23/23 steps, no paper delta) — see the finding
above. Do **not** enable the flag in CI until Phase 2 human sign-off closes
the mechanism gap (real A* / larger budget / tighter geometry / etc.).

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
