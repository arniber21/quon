# Visualization samples

Stress programs and **checked-in golden artifacts** — QASM, MLIR snippets, NA
interaction-graph DOT, and NA analytic resource reports — for the viz
implementers building `quonviz` (#134), `quonmapviz` (#135), and `quontopo`
(#136), plus `python/visualize_na_schedule.py` (#113, closed). None of these
viz packages need to exist yet: every entry below is "here is the real
`quonc` artifact your tool will consume, and here is what a human should see
in it."

## Status

Owned by pack **#189** (parent #184). Goldens are refreshed with
[`refresh_goldens.sh`](./refresh_goldens.sh) (documented `quonc` invocations,
one per showcase — see the script for the exact commands) and pinned against
regeneration by
[`quonc/tests/viz_showcase.rs`](../../quonc/tests/viz_showcase.rs). Every
`.qn` program below is **linked, not forked** from its canonical home in
`test/verify/` or `test/na/` (issue #192's "stop forking `test/na/`" review
finding applies here too) — this pack owns the *goldens*, not a second copy
of the circuits.

## Showcase

| Catalog id | Program (linked) | Target / args | Feeds | Goldens |
| --- | --- | --- | --- | --- |
| `visualization/dense-swap-mismatch` | [`test/verify/qaoa.qn`](../../test/verify/qaoa.qn) | `fake_manila_v2` (5-qubit line) | [#135](https://github.com/arniber21/quon/issues/135) | [`goldens/dense_swap_mismatch/`](./goldens/dense_swap_mismatch/) |
| `visualization/teleport-dynamic` | [`test/verify/teleport.qn`](../../test/verify/teleport.qn) | `generic_openqasm` (default) | [#134](https://github.com/arniber21/quon/issues/134) | [`goldens/teleport_dynamic/`](./goldens/teleport_dynamic/) |
| `visualization/qft-depth` | [`test/verify/qft.qn`](../../test/verify/qft.qn) | `generic_openqasm` (default) | [#134](https://github.com/arniber21/quon/issues/134) | [`goldens/qft_depth/`](./goldens/qft_depth/) |
| `visualization/na-interaction-graph` | [`test/na/qaoa_graph.qn`](../../test/na/qaoa_graph.qn) | `generic_rna_v0`, zoned | [#113](https://github.com/arniber21/quon/issues/113), [#136](https://github.com/arniber21/quon/issues/136) | [`goldens/na_interaction_graph/`](./goldens/na_interaction_graph/) |
| `visualization/na-schedule-metrics-bell` | [`test/na/bell.qn`](../../test/na/bell.qn) | `generic_rna_v0`, zoned | [#113](https://github.com/arniber21/quon/issues/113), [#136](https://github.com/arniber21/quon/issues/136) | [`goldens/na_schedule_metrics/bell_zoned.resource_report.json`](./goldens/na_schedule_metrics/bell_zoned.resource_report.json) |
| `visualization/na-schedule-metrics-qaoa-graph` | [`test/na/qaoa_graph.qn`](../../test/na/qaoa_graph.qn) | `generic_rna_v0`, zoned | [#113](https://github.com/arniber21/quon/issues/113), [#136](https://github.com/arniber21/quon/issues/136) | [`goldens/na_schedule_metrics/qaoa_graph_zoned.resource_report.json`](./goldens/na_schedule_metrics/qaoa_graph_zoned.resource_report.json) |
| `visualization/noise-aware-target-overlay` | [`test/verify/ising.qn`](../../test/verify/ising.qn) | `fake_manila_v2` (5-qubit line) | [#136](https://github.com/arniber21/quon/issues/136) | [`goldens/noise_target_overlay/`](./goldens/noise_target_overlay/) |

### 1. Dense SWAP mismatch (#135)

`qaoa.qn`'s cost layer is K3 (every pair of its 3 qubits interacts), but
`fake_manila_v2` is a 5-qubit *line* (`0-1-2-3-4`): qubits 0 and 2 aren't
adjacent. SABRE routes that interaction with a 3-CNOT SWAP network — visible
in [`goldens/dense_swap_mismatch/qaoa_manila.qasm`](./goldens/dense_swap_mismatch/qaoa_manila.qasm)
as the `cx q[1],q[2]; cx q[2],q[1]; cx q[1],q[2];` triple straddling the
`(0,2)` `Rzz` rotation.

**The mismatch**: [`metrics.json`](./goldens/dense_swap_mismatch/metrics.json)'s
`swap_count` is `0`. The metric only counts a literal `quantum.circ.gate`
`SWAP` op, and by the time metrics run, physical passes have already
decomposed that SWAP into the 3 CNOTs above — so the counter and the QASM
disagree about whether a SWAP happened. **#135's mapper visualizer must
recognize the 3-CNOT pattern directly in the trace, not trust this counter.**
That is the actual "what you should see" lesson this entry exists to teach,
not a bug this pack is positioned to fix.

### 2. Teleport / dynamic (#134)

`teleport.qn`'s source has real classical control flow —
`(if z_bit then pauli_x() else id_one()) @ bob` — but
[`goldens/teleport_dynamic/teleport.qasm`](./goldens/teleport_dynamic/teleport.qasm)
has no `if` at all: `measurement_deferral` (SPEC §7.1) proves the correction
can be applied coherently (`cx`/`cz`, deferred past the mid-circuit
measurements) instead of literally branching on the measured bit. **A
circuit visualizer that only shows compiled QASM will never show a user
their own `if`/`else`** — #134's multi-stage view (source AST vs. lowered
IR) needs this exact circuit as its "here's what source-level dynamism looks
like once the optimizer proves it doesn't need to be dynamic" example. (For
a circuit whose mid-circuit measurement genuinely survives to the schedule
— i.e. deferral doesn't or can't apply — see pack #192's
`neutral-atom/repetition-d3-memory` entry instead; this pack doesn't repeat
that story to avoid re-forking its circuit.)

### 3. QFT depth (#134)

`qft_roundtrip(3) = qft(3) |> adjoint(qft(3))` starts, right after lowering,
as a genuine ~30-gate recursive QFT composed with its own inverse — see
[`goldens/qft_depth/before_optimization.mlir`](./goldens/qft_depth/before_optimization.mlir)'s
`qft_roundtrip__elab0` function body (Rz/CNOT/H/SWAP chain). One circ-pass
fixpoint later (`gate_cancellation` + `rotation_merging`, run to fixpoint —
see [`goldens/qft_depth/after_optimization.mlir`](./goldens/qft_depth/after_optimization.mlir)),
that entire function has collapsed to a bare pass-through:
`"quantum.circ.return"(%arg0, %arg1, %arg2)`. Final compiled depth is `1`,
gate count `2` (just the `prep_101` X gates) —
see [`goldens/qft_depth/metrics.json`](./goldens/qft_depth/metrics.json).
This before/after MLIR pair is exactly the input a pass-diff visualizer
(#134's "how did optimization passes change gate count / depth") needs to
render — a dramatic, honest example, not a cherry-picked small delta.

### 4. NA interaction graph (#113, #136)

[`goldens/na_interaction_graph/qaoa_graph.dot`](./goldens/na_interaction_graph/qaoa_graph.dot)
is `qaoa_graph.qn`'s (3-regular MaxCut-style, 4 qubits) weighted interaction
graph, Graphviz DOT, straight from `--emit-na-graph` — the exact artifact
#113's acceptance criteria asked for and #136's hardware-truth canvas needs
as its "which pairs fire" layer. Edge weights are the `Rzz` gamma-scaled
interaction strengths NA scheduling used to order entangling layers.

### 5. NA bell/QAOA schedule (#113, #136)

[`goldens/na_schedule_metrics/`](./goldens/na_schedule_metrics/) holds the
small, deterministic analytic resource reports for `bell.qn` (2 qubits, the
smallest possible NA schedule) and `qaoa_graph.qn` (4 qubits, denser) on the
zoned backend — `rydberg_stages`, `rearrangement_steps`, `trap_transfers`,
`total_time_us`, and the per-mechanism `error_budget` breakdown. This is two
catalog rows, `visualization/na-schedule-metrics-bell` and
`visualization/na-schedule-metrics-qaoa-graph`, one per program, so each
row's `path` owns exactly the one artifact it declares. **Not**
checked in: the full `--emit-na-schedule` envelope, which enumerates every
trap site across a target's zones — for `bell.qn` on `generic_rna_v0` that's
~8,437 `AtomSite`s and a ~1 MB JSON file (verified while building this pack:
`quonc --target targets/neutral_atom/generic_rna_v0.json --na-backend zoned
--emit-na-schedule -` on `test/na/bell.qn`) — still not something we want to
check in and diff on every schedule-layout change. The resource report is
the right-sized golden for "what should the schedule metrics look like";
#113's own script/#136's canvas should still read the full envelope live
from `quonc`, not from a checked-in copy.

### 6. Noise-aware target overlay (#136)

`ising.qn` is already nearest-neighbor, so it maps onto `fake_manila_v2`'s
line with **zero SWAPs** (see
[`goldens/noise_target_overlay/metrics.json`](./goldens/noise_target_overlay/metrics.json)) —
deliberately clean, so the interesting data isn't routing but *noise*:
`targets/ibm/fake_manila_v2.json`'s checked-in `noise.two_qubit_fidelity`
and `noise.readout_error` fields are keyed **per edge / per qubit**. Contrast
that with entry 5's NA `error_budget`, which is **per mechanism**
(rydberg/movement/transfer/measurement/reset/idle), not per edge. #136's
"hardware truth canvas" needs to reconcile both noise vocabularies —
per-component (fixed targets) and per-mechanism (NA analytic budget) — onto
one overlay, not invent a third one.

## Refreshing goldens

```bash
samples/visualization/refresh_goldens.sh          # regenerate in place
samples/visualization/refresh_goldens.sh --check  # diff against committed goldens
```

`quonc/tests/viz_showcase.rs` re-runs the same `quonc` invocations in-process
and byte-/value-compares the output against the committed goldens on every
`cargo test -p quonc`, so drift between this README's prose and the actual
artifacts fails a real test, not just a lint.

## Adding a showcase

See [`../CONTRIBUTING.md`](../CONTRIBUTING.md). Link (don't fork) a program
that already has a canonical home in `test/verify/` or `test/na/`; only add
an owned `.qn` under `samples/visualization/` when the story's circuit is
genuinely new pedagogy that doesn't already exist as a fixture elsewhere.
Add the new golden(s) to [`refresh_goldens.sh`](./refresh_goldens.sh), a
catalog row with an `artifacts` list, and a pinning test in
`quonc/tests/viz_showcase.rs`.
