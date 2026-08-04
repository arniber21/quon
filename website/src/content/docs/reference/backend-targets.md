---
title: Backend targets and artifact contracts
description: Reference for backend target descriptors, capability flags, validation, and the schema/evidence contract of every emitted artifact.
---

Quon's backend is split into two architecture families selected by a target
descriptor. Each family emits a *different* set of artifacts, and those
artifacts are not interchangeable: some are canonical compiler IR, some are
debug/visualization views, and some are evaluation evidence of distinct kinds.
This page is the contract reference — what each target requires, what each
emission flag produces, who consumes it, what kind of evidence it is, and what
it is *not*.

For the task-oriented "how do I run a compile" walkthrough, see
[Backends and verification](/guides/backends/). For the hardware model and
citations behind the neutral-atom target, see the
[neutral-atom architecture model](/architecture/na-model/). This page
complements those: it does not re-derive the model, it documents the contract.

## Two axes that organize everything

Two distinctions run through the whole reference. Keep them in mind or the
artifact list will look more tangled than it is.

**Canonical vs visualization.** The neutral-atom path has one canonical
schedule IR — `quantum.na` MLIR ([ADR-0011](https://github.com/arniber21/quon/blob/main/docs/adr/0011-quantum-na-canonical-schedule-ir.md))
— and several derived views built *from* it for tooling. A view can be deleted
without changing what the compiler proved; the canonical IR cannot. Quoting a
view as if it were the IR (or vice versa) is a category error.

**Evidence kinds.** Quon is deliberate about not collapsing different kinds of
result into one "number" ([ADR-0020](https://github.com/arniber21/quon/blob/main/docs/adr/0020-qec-reports-remain-separate.md)):

- **Analytic** — compiler-computed schedule/QEC metrics (`rate × count` budgets,
  Enola-Eq.-(1) fidelity estimates). Deterministic, reproducible, but a model,
  not a measurement.
- **Sampled** — Monte-Carlo logical failure rates from Stim/Sinter. Stochastic;
  a different estimation method than the analytic budget.
- **Structure** — IR/experiment shapes (a Stim circuit's detector graph, a QEC
  experiment's check graph) with no physical noise until a later stage annotates
  them.

Analytic and sampled evidence are kept in *separately labeled* sections and are
never fused into an undifferentiated below-threshold claim. None of the three is
a threshold claim on its own.

## Target families

A `BackendTarget` carries an `id` and one architecture-specific payload selected
by a `kind` discriminant ([ADR-0009](https://github.com/arniber21/quon/blob/main/docs/adr/0009-unified-backend-target-kind.md)):

| `kind` | Payload | Output path |
| --- | --- | --- |
| `"fixed"` (default when `kind` is absent) | `FixedTarget` | OpenQASM 3 |
| `"neutral_atom_reconfigurable"` | `NeutralAtomTarget` | `quantum.na` MLIR + schedule/resource/QEC artifacts |

Load a descriptor with `--target <PATH>`. With no `--target`, `quonc` uses the
built-in `generic_openqasm` fixed target: 64 all-to-all qubits, the standard
OpenQASM gate set, no device noise. Inspect either without compiling:

```bash
quonc --target backend/tests/fixtures/device_5q.json --print-target
quonc --target targets/neutral_atom/generic_rna_v0.json --print-target
```

The two families share only `id` at the outer level; every other field is owned
by the architecture payload. The JSON wire form uses `#[serde(deny_unknown_fields)]`,
so unknown keys are rejected rather than silently ignored.

## Fixed target reference

The fixed-connectivity gate-model descriptor models a device with a static
coupling graph. Every field except `kind` and `noise` is **required** (no
`#[serde(default)]`); a missing required field fails loading with a typed
`InvalidTargetConfig` error.

| Field | Type | Required | Meaning |
| --- | --- | --- | --- |
| `kind` | `"fixed"` | optional | Defaults to `fixed` for backward compatibility |
| `id` | string | yes | Target identifier in diagnostics and metrics |
| `num_qubits` | integer | yes | Physical qubit count; **must agree** with the topology |
| `topology.edges` | `[[int,int]]` | yes | Directly connected qubit pairs; routing inserts SWAPs for non-adjacent 2Q ops |
| `native_gates` | `[string]` | yes | OpenQASM gate names the target accepts (non-empty) |
| `noise` | object | optional | Gate fidelity, T1/T2, readout error (metadata, **not** an Aer noise model) |
| `meas_latency_us` | number | yes | Measurement latency |
| `supports_mid_circuit_meas` | bool | yes | Mid-circuit measurement capability |
| `supports_feed_forward` | bool | yes | Classical feed-forward capability |

`noise` is a map keyed by gate name and qubit-string (JSON keys are strings):
`single_qubit_fidelity`, `two_qubit_fidelity`, `t1_us`, `t2_us`,
`readout_error`. All sub-maps default to empty, so a target may carry partial
or no noise data.

**Validation.** The loader (`FixedTarget::try_new`) checks that `num_qubits`
matches the qubit count derived from the topology — the two cannot disagree.
`native_gates` must be non-empty. Noise qubit-keys are decoded and
bounds-checked against `num_qubits`. The built-in `generic_openqasm` target
bypasses the descriptor (it is constructed in code) and is always available.

See [`backend/tests/fixtures/device_5q.json`](https://github.com/arniber21/quon/blob/main/backend/tests/fixtures/device_5q.json)
and [`targets/ibm/fake_manila_v2.json`](https://github.com/arniber21/quon/blob/main/targets/ibm/fake_manila_v2.json)
for complete examples.

**Capability flags.** `supports_mid_circuit_meas` and `supports_feed_forward`
record dynamic-circuit capability. They are target *metadata* reported through
metrics; the fixed emit path does not currently reject programs that exercise
capabilities the target lacks — they describe the device, not a compile-time
gate.

## Neutral-atom target reference

The reconfigurable neutral-atom descriptor models a DPQA/zoned array: zones,
array geometry, AOD movement, Rydberg interaction, timing, fidelity, and a cost
model. All fields except `error_model` and `atom_loss_model` are **required**.

| Field | Type | Required | Meaning |
| --- | --- | --- | --- |
| `id` | string | yes | Target identifier |
| `kind` | `"neutral_atom_reconfigurable"` | yes | Discriminant |
| `grid` | `{width_um, height_um}` | yes | Bounding box |
| `zones` | `[Zone]` | yes | Zone list — at least one `storage`, `entanglement`, and `readout` |
| `movement` | object | yes | AOD movement model + speed model + transfer time |
| `interaction` | object | yes | Rydberg range, isolation spacing, parallel-pair cap |
| `native_gates` | `[string]` | yes | Gate names executable natively (non-empty) |
| `timing` | object | yes | `cz_us`, `single_qubit_us`, `measurement_us`, `reset_us` |
| `fidelity` | object | yes | `cz`, `single_qubit`, `atom_transfer`, `coherence_time_us` |
| `error_model` | object | optional | Explicit physical error probabilities for QEC (sibling to `fidelity`) |
| `atom_loss_model` | object | optional | Movement-induced heating/loss parameters |
| `cost_model` | object | yes | Linear cost weights |

A **Zone** declares a region's capability:

| Field | Type | Meaning |
| --- | --- | --- |
| `zone_id` | integer | Unique zone identifier |
| `kind` | `"storage"` \| `"entanglement"` \| `"readout"` | Zone capability |
| `rows`, `cols` | integer | Static-trap grid extent (`entanglement`: trap *pairs*) |
| `origin_um` | `[number, number]` | Lower-left corner |
| `site_pitch_um` | `[number, number]` | Trap spacing in x and y |
| `pair_gap_um` | number, `entanglement` only | Distance between the two traps of a pair |

**Movement** selects row/column-coupled AOD semantics (the only legal value —
`"free_manhattan"` is deliberately not a variant, enforcing constraints M1–M5
in the movement-legality verifier). `speed_model.kind` is `"sqrt"` (the
$\sqrt{d/a}$ timing model) or `"jerk_limited"` (symmetric S-curve; `jerk_m_s3`
and `max_velocity_m_s` default to `0.0`).

**Validation.** The loader enforces architecture invariants, not just types:

- Every target must contain at least one `storage`, `entanglement`, and
  `readout` zone.
- Entanglement zones *require* `pair_gap_um`; non-entanglement zones must *not*
  supply it.
- `interaction.max_parallel_entangling_pairs` must not exceed the entanglement
  zone capacity (`rows × cols` summed over entanglement zones).
- Zones may not overlap.
- `movement.speed_model` parameters are checked: `acceleration_m_s2` is
  positive; under `jerk_limited`, `jerk_m_s3` is positive and finite.
- `error_model` and `atom_loss_model` fields are non-negative finite scalars
  (where applicable; `error_model` rates are probabilities in $[0,1]$).

**`error_model` is a sibling of `fidelity`, not a replacement.** Per
[ADR-0017](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/architecture_model.md),
you must never convert rates as `1 − fidelity.*`. A target without `error_model`
still loads for non-QEC paths; requesting QEC error artifacts on such a target
is a hard failure (`MissingErrorModel`), never a silent fallback. `atom_loss_model`
is a placeholder analytic knob (see the architecture model §2/§8.6), not a
measured calibration.

The checked-in sample is
[`targets/neutral_atom/generic_rna_v0.json`](https://github.com/arniber21/quon/blob/main/targets/neutral_atom/generic_rna_v0.json).
For the full field-by-field schema, units, and citation provenance of every
constant, read the
[architecture model source document](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/architecture_model.md).

## Artifact matrix

The table below is the index; each row links to a detail block with producer,
schema, consumer, evidence kind, and explicit non-claims.

| Artifact | Producer flag | Canonical? | Schema / version | Evidence kind |
| --- | --- | --- | --- | --- |
| [OpenQASM 3](#openqasm-3) | `--emit-qasm` | yes (fixed path) | OpenQASM 3.0 text | n/a (target input) |
| [Canonical NA MLIR](#canonical-na-mlir-quantumna) | `--emit-na-mlir` | **yes** (NA path) | `quantum.na` MLIR text | structure |
| [Schedule JSON](#schedule-json) | `--emit-na-schedule` | no — view | `na_schedule_view`, v1 | analytic (embedded metrics) |
| [Interaction graph](#interaction-graph) | `--emit-na-graph` | no — view | Graphviz DOT | structure |
| [Resource report](#resource-report) | `--emit-resource-report` | yes (QEC sizing) | `ResourceReport` JSON/Markdown | **analytic** |
| [Compiler statistics](#compiler-statistics) | `--emit-na-stats` | no — telemetry | `na_compiler_stats`, v1 | n/a (internals) |
| [NAViz interop](#naviz-interop) | `--emit-naviz` | no — interop | `.naviz` + `.namachine` | structure |
| [QEC experiment](#qec-experiment) | `--emit-qec-experiment` | yes (QEC IR) | `qec_experiment`, v1 | structure |
| [Stim circuit](#stim-circuit) | (sibling of QEC experiment) | no — structure | Stim text | structure |
| [Fused validation](#fused-validation) | `--emit-qec-validation` | no — fusion | `qec_validation_report`, v1 | analytic + sampled (separate) |

A few flags write *two* sibling files from one `PATH` (QEC experiment, NAViz,
fused validation); these require a filesystem path, never `-`/stdout.

### OpenQASM 3

- **Producer:** `quonc program.qn --target <fixed.json> --emit-qasm` (or no
  `--target` for `generic_openqasm`).
- **Schema / version:** OpenQASM 3.0 textual program. No Quon envelope; the
  output is a standalone program consumed by any OpenQASM 3 runner.
- **Consumer:** The Qiskit Aer bridge (`python/quon_aer.py`), reference verifiers
  (`test/verify/`), or any external OpenQASM 3 toolchain.
- **Evidence kind:** none — this is target-bound gate IR, not a result. It is the
  *input* to simulation/verification, not a performance estimate.
- **Non-claims:** Aer simulation counts are raw ideal-simulator samples, not
  live-hardware performance. Device `noise` in the target is metadata for
  scheduling/metrics, not an injected Aer noise model.
- **Target constraint:** fixed targets only. Passing `--emit-qasm` with a
  neutral-atom target is a hard error.

### Canonical NA MLIR (`quantum.na`)

- **Producer:** `quonc program.qn --target <na.json> --emit-na-mlir` (path or
  `-` for stdout).
- **Schema / version:** `quantum.na` dialect textual MLIR
  ([ADR-0007](https://github.com/arniber21/quon/blob/main/docs/adr/0007-quantum-na-as-separate-dialect.md)).
  Ops: `alloc_atom`, `place`, `move`, `entangle`, `measure`, `layer`.
- **Canonical status:** **this is the primary neutral-atom schedule artifact**
  ([ADR-0011](https://github.com/arniber21/quon/blob/main/docs/adr/0011-quantum-na-canonical-schedule-ir.md)).
  The planner's in-memory `ScheduleLayer` is never serialized as a primary
  artifact; the schedule JSON is a derived view.
- **Consumer:** the `--verify-na` verifier, downstream IR tooling, and
  reproducibility/archival. For QEC-backed programs, verification runs
  automatically ([ADR-0021](https://github.com/arniber21/quon/blob/main/docs/adr/0021-verify-na-auto-on-qec.md)).
- **Evidence kind:** structure — a verified schedule shape, not a result.
- **Non-claims:** a verified `quantum.na` schedule proves movement legality
  (M1–M5) and layer scheduling, *not* that the schedule is optimal, *not* a
  fidelity or threshold number.
- **stdout precedence:** when both `--emit-na-mlir -` and `--emit-na-schedule -`
  target stdout, the canonical MLIR owns stdout.

### Schedule JSON

- **Producer:** `quonc program.qn --target <na.json> --emit-na-schedule [PATH]`.
- **Schema / version:** `kind: "na_schedule_view"`, `schema_version: 1`. Fields:
  `meta` (target id, backend, placer mode), `metrics` (an embedded
  `ResourceReport`), `zones` (geometry subset), optional `layout`, `layers`.
- **Canonical status:** **debug/visualization view, not the schedule IR.**
  Built from the canonical MLIR for Python tooling. `meta.na_placer` /
  `meta.na_backend` are reserved for before/after comparison without a schema
  bump.
- **Consumer:** `python/visualize_na_schedule.py` (matplotlib frame rendering).
- **Evidence kind:** the `layers`/`layout` are structure; the embedded `metrics`
  carry the same analytic disclaimer as the resource report.
- **Non-claims:** not a second source of truth for the schedule. Treat it as a
  rendering aid; the canonical artifact is `--emit-na-mlir`.

### Interaction graph

- **Producer:** `quonc program.qn --target <na.json> --emit-na-graph [PATH]`.
- **Schema / version:** Graphviz DOT. No version envelope.
- **Canonical status:** view — the atom-indexed interaction graph
  ([ADR-0029](https://github.com/arniber21/quon/blob/main/docs/adr/0029-atom-indexed-hybrid-interaction-graph.md))
  is the planner's input; DOT is its rendered form.
- **Consumer:** Graphviz / `visualize_na_schedule.py --graph`.
- **Evidence kind:** structure.
- **Non-claims:** the graph shows which pairs must interact, not a placement or
  a schedule.

### Resource report

- **Producer:** `quonc program.qn --target <na.json> --emit-resource-report [PATH]`,
  `--resource-report-format <json|markdown>`. A `.md` path selects Markdown;
  otherwise JSON unless overridden.
- **Schema / version:** `ResourceReport` (`#[serde(deny_unknown_fields)]`).
  Carries `evidence_kind: "analytic"` and an `evidence_disclaimer`. Fields:
  schedule counts (`rydberg_stages`, `rearrangement_steps`,
  `trap_transfers`, `entangle2_count`, …), timing, QEC sizing
  (`logical_qubits`, `physical_atoms`, `atoms_per_logical`, `code_family`,
  `distance`, `memory_rounds`), `t_count`, an `error_budget` of
  `rate × count`, and an Enola-Eq.-(1) fidelity estimate.
- **Canonical status:** the canonical QEC sizing/budget artifact.
- **Consumer:** humans (Markdown), metric baselines/regression, and the fused
  validation report (which embeds it unmodified).
- **Evidence kind:** **analytic** — compiler-computed metrics, deterministic and
  reproducible.
- **Non-claims:** **not** fused with the Python/Sinter sampled CSV
  ([ADR-0020](https://github.com/arniber21/quon/blob/main/docs/adr/0020-qec-reports-remain-separate.md)).
  `error_budget` is a model (`rate × count`), not a sampled logical failure
  rate; the fidelity estimate is a distinct analytic estimate from the budget.
  Neither is a threshold claim. The optional ablation join CSV is a comparison
  aid only and does not mutate this DTO.

### Compiler statistics

- **Producer:** `quonc program.qn --target <na.json> --emit-na-stats [PATH]`.
- **Schema / version:** `kind: "na_compiler_stats"`, `schema_version: 1`.
  Fields: `version` (tool/target ids), `config` (effective backend/placer/
  placement/compaction echo), `stage_timings_us` (per-stage wall times),
  `search` (routing-aware node expansions, budget, fallbacks).
- **Canonical status:** telemetry — a **separate artifact from the resource
  report** (issue #307). Nothing in the pipeline reads it back.
- **Consumer:** performance profiling and config auditing.
- **Evidence kind:** none about the *program* — this is internals telemetry
  about *how the compile ran*, not schedule or QEC evidence.
- **Non-claims:** not a resource or fidelity number. Supported for both
  bare-qubit and QEC-backed programs (#317).

### NAViz interop

- **Producer:** `quonc program.qn --target <na.json> --emit-naviz <PATH>`
  (filesystem path only; writes `<stem>.naviz` + sibling `<stem>.namachine`).
- **Schema / version:** MQT NAViz instruction file + machine descriptor.
- **Canonical status:** interop — a rendering format for the
  [MQT NAViz](https://github.com/munich-quantum-toolkit/naviz) visualizer.
- **Consumer:** the NAViz visualizer; see the
  [NAViz visualization guide](/guides/naviz-visualization/).
- **Evidence kind:** structure.
- **Non-claims:** a visualization export, not a schedule IR or a result.

### QEC experiment

- **Producer:** `quonc examples/na_qec/repetition_d3_memory.qn --target <na.json>
  --emit-qec-experiment <PATH>` (filesystem path only; dual-emits
  `<stem>.qec.json` + sibling `<stem>.stim`).
- **Schema / version:** `kind: "qec_experiment"`, `schema_version: 1`. Fields:
  `family`, `code_family`, `distance`, `rounds`, logical observables, check
  graph, atom/site map, an `error_model` snapshot, and refs into `quantum.na`.
  See the
  [experiment schema reference](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/qec_experiment_schema.md).
- **Canonical status:** the canonical QEC evaluation IR
  ([ADR-0018](https://github.com/arniber21/quon/blob/main/docs/adr/0018-qec-experiment-dual-emit.md)).
- **Consumer:** `python/quon_qec_sinter.py` (which annotates noise from the
  JSON `error_model` and samples the sibling Stim circuit), and the fused
  validation report.
- **Evidence kind:** structure — the experiment shape, with no physical noise
  channels in the sibling Stim.
- **Target/program constraint:** requires a `neutral_atom_reconfigurable` target
  *and* a QEC-backed program (e.g. `repetition_code` / `memory_round`);
  bare-qubit NA programs have no experiment IR.

### Stim circuit

- **Producer:** sibling of the QEC experiment — written alongside `<stem>.qec.json`.
- **Schema / version:** Stim circuit text.
- **Canonical status:** structure-level — **no physical noise channels**
  ([ADR-0024](https://github.com/arniber21/quon/blob/main/docs/adr/0024-stim-noise-applied-in-python.md)).
- **Consumer:** `python/quon_qec_sinter.py`, which annotates noise from the JSON
  `error_model` before sampling.
- **Evidence kind:** structure.
- **Non-claims:** the bare Stim is a detector/check graph, not a noisy circuit
  and not a logical failure rate. Do not sample it directly expecting noise.

### Fused validation

- **Producer:** `quonc <qec.qn> --target <na.json> --emit-qec-validation <PATH>
  [--validation-shots N]` (filesystem path only). Compiles, dual-emits the QEC
  experiment, builds the analytic resource report, shells out to
  `python/quon_qec_sinter.py` to sample, and fuses — after a provenance check —
  into `<stem>.validation.json` + sibling `<stem>.validation.md`. Writes the
  QEC experiment, the analytic resource report, and the sampled-evidence JSON as
  separate sibling primaries beside the report.
- **Schema / version:** `kind: "qec_validation_report"`, `schema_version: 1`.
  Fields: `disclaimer`, `provenance` (fingerprint tying sampled data to the
  compiled artifact), `analytic` (`evidence_kind: "analytic"`, embedding the
  unmodified `ResourceReport`), `sampled` (`evidence_kind: "sampled"`), and
  `mismatch_warnings` (only with `--allow-sampled-mismatch`). See the
  [validation report reference](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/qec_validation_report.md).
- **Canonical status:** an **optional third artifact**
  ([ADR-0020 amendment](https://github.com/arniber21/quon/blob/main/docs/adr/0020-qec-reports-remain-separate.md))
  — a fusion, not a new primary. It embeds the analytic report unmodified and
  keeps the two evidence kinds in separate labeled sections.
- **Consumer:** humans (`.md`), end-to-end review, the
  [neutral-atom FT demo](/guides/na-ft-demo/).
- **Evidence kind:** analytic + sampled, **kept separate** — never collapsed.
- **Non-claims:** **validation evidence, not a threshold claim.** Analytic and
  sampled numbers are different kinds of evidence shown side by side. Use
  `--attach-sampled` to fuse a pre-sampled JSON without shelling out (offline/CI
  without the Stim stack); `--allow-sampled-mismatch` downgrades a provenance
  mismatch from a refusal to a recorded warning.

## A complete neutral-atom walkthrough

This walkthrough goes source → target → canonical schedule → inspection →
interpretation for a bare-qubit NA program, without inferring semantics from
the architecture notes. (For the QEC-backed end-to-end path, see the
[neutral-atom FT demo](/guides/na-ft-demo/).)

### 1. Source

A QAOA-style program over a graph. The frontend parses, typechecks (linear
ownership, depth bounds), elaborates parametric circuit calls, and lowers to
`quantum.circ` / `quantum.dynamic` MLIR. Nothing target-specific has happened
yet — the same IR feeds the fixed path.

```bash
quonc test/na/qaoa_graph.qn --dump-ir
```

### 2. Target

Select the reconfigurable neutral-atom target. `--print-target` confirms the
descriptor loaded and which zones/capabilities are in effect:

```bash
quonc --target targets/neutral_atom/generic_rna_v0.json --print-target
```

With `--target` set, the emit stage branches to the NA path: extract an
atom-indexed interaction graph, schedule entangling layers (Misra–Gries /
ASAP), plan movement (zoned RAP or flat AOD), optionally compact, and lower the
planner's `ScheduleLayer`s through a single converter into the canonical
`quantum.na` spec.

### 3. Canonical schedule

Emit the canonical schedule IR. For a bare-qubit program, pass `--verify-na`
explicitly (QEC-backed programs auto-verify):

```bash
quonc test/na/qaoa_graph.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-mlir schedule.mlir --verify-na
```

`schedule.mlir` is the artifact to archive and reason about. It is a verified
`quantum.na` program: `alloc_atom`/`place`/`move`/`entangle`/`layer` ops over
atoms, with movement legality (M1–M5) and layer scheduling checked. A successful
`--verify-na` means the schedule is *legal*, not that it is *optimal*.

### 4. Inspection

The canonical MLIR is for IR tooling and verification. For human/Python
inspection, emit the derived views from the *same* compile:

```bash
quonc test/na/qaoa_graph.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule schedule.json \
  --emit-na-graph graph.dot \
  --emit-resource-report report.md \
  --emit-na-stats stats.json
```

- `schedule.json` (`na_schedule_view`) — zones, layout, layers, and embedded
  metrics for `visualize_na_schedule.py` frame rendering. Render with:

  ```bash
  pip install -r python/requirements-viz.txt
  python python/visualize_na_schedule.py schedule.json --graph graph.dot \
    -o /tmp/na-viz --format svg
  ```

- `graph.dot` — the interaction graph rendered as Graphviz.
- `report.md` — the analytic resource report (schedule counts, timing, QEC
  sizing when applicable, `error_budget`, fidelity estimate).
- `stats.json` (`na_compiler_stats`) — per-stage wall times and the effective
  backend/placer/compaction config, separate from the resource report.

### 5. Interpretation

Read the outputs against their evidence kinds:

- The **schedule MLIR** is structure: it answers "is this movement plan legal
  and what does it do?" — not "how good is it?"
- The **resource report** is analytic: `rydberg_stages`, `rearrangement_steps`,
  `total_time_us`, and `error_budget = rate × count` are compiler-computed
  model estimates. They are deterministic and reproducible, but a model, not a
  measurement. The fidelity estimate is a separate analytic quantity from the
  error budget.
- The **stats** are internals: they explain how the compile ran (timings,
  search effort, effective config), not what the program costs on hardware.
- **Nothing here is a sampled logical failure rate** — that comes only from
  `python/quon_qec_sinter.py` against a QEC experiment's Stim circuit, a
  different artifact for QEC-backed programs. **Nothing is a threshold claim.**

To change the schedule, change the target or the planner knobs
(`--na-backend`, `--na-placer`, `--na-placement`, `--no-na-compact`) and
re-emit. The canonical MLIR is the comparison point; the JSON view and resource
report are derived from it.

## Where to go deeper

- [Backends and verification](/guides/backends/) — task-oriented run guide.
- [Neutral-atom architecture model](/architecture/na-model/) — the hardware
  model, target schema provenance, and citations (this page documents the
  *contract*, not the model).
- [Compiler internals](/architecture/compiler-internals/) — the pipeline stages
  and the `quantum.na` dialect's place in them.
- [quonc CLI reference](/reference/quonc/) — every flag.
- [QEC experiment schema](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/qec_experiment_schema.md)
  and
  [validation report schema](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/qec_validation_report.md)
  — full field references for the QEC artifacts.
