---
title: quonc CLI
description: Command-line reference for the Quon compiler.
---

`quonc` compiles Quon (`.qn`) source through the shared compiler pipeline and
emits target-specific artifacts.

```bash
quonc program.qn --emit-qasm > program.qasm

quonc program.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule schedule.json \
  --emit-na-graph graph.dot \
  --emit-resource-report report.md
```

Without output or metrics flags, a successful compile prints a short
confirmation to standard error.

## Usage

```text
quonc [OPTIONS] [SOURCE]
```

`SOURCE` is the `.qn` file to compile. It may be omitted with `--print-target`
or `--list-passes`.

## Emission options

### `--emit-qasm`

Write the generated OpenQASM 3 program to standard output. This is the fixed
gate-model output path.

```bash
quonc program.qn --emit-qasm > program.qasm
```

### `--emit-na-schedule [PATH]`

Emit the neutral-atom schedule **visualization envelope** (JSON). This is a
debug/tooling view (`kind: na_schedule_view`, `schema_version: 1`) with
`meta`, `metrics`, `zones`, optional `layout`, and `layers`. Canonical schedule
IR remains `--emit-na-mlir` (`quantum.na`). With no path, or with `-`, the JSON
is written to standard output.

```bash
quonc program.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule schedule.json
```

Render cycle frames with matplotlib (PNG/SVG; no HTML):

```bash
pip install -r python/requirements-viz.txt
python python/visualize_na_schedule.py schedule.json -o /tmp/na --format svg
```

### `--emit-na-graph [PATH]`

Emit the interaction graph as Graphviz DOT. With no path, or with `-`, DOT is
written to standard output.

```bash
quonc program.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-graph graph.dot

python python/visualize_na_schedule.py --graph graph.dot -o /tmp/na --format svg
```

### `--emit-resource-report [PATH]`

Emit the compiler **analytic** neutral-atom resource report (schedule metrics,
optional QEC sizing metadata, and `error_budget` contributions =
`rate × schedule count`). With no path, or with `-`, the report is written to
standard output. A `.md` path selects Markdown; other paths select JSON unless
`--resource-report-format` overrides it.

This artifact is **not** fused with Python/Sinter sampled results
(`python/quon_qec_sinter.py` CSV). JSON includes `evidence_kind: "analytic"`
and a short `evidence_disclaimer`; Markdown uses an analytic H1 and Notes.
Analytic estimates and sampled logical failure rates are different kinds of
evidence; neither is a threshold claim (ADR-0020).

```bash
quonc program.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-resource-report report.md
```

### `--resource-report-format <json|markdown>`

Force the resource-report format.

```bash
quonc program.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-resource-report - \
  --resource-report-format markdown
```

### `--emit-na-stats [PATH]`

Emit compiler-internals telemetry about the compile: per-stage wall times
(extract, schedule-from-graph, entangling-layer scheduling, placement/routing,
compaction, resource-report build), routing-aware search diagnostics (node
expansions, budget, fallbacks), an effective-configuration echo (backend,
placer, placement strategy, compaction options), and tool/target version
identifiers. With no path, or with `-`, stats are written to standard output.

This is a **separate artifact** from `--emit-resource-report` (issue #307) —
compiler-internals telemetry about *how* the compile ran, not schedule/QEC
evidence about the program. It requires the neutral-atom backend (the same
target/backend constraints as `--emit-na-schedule`) and does not yet
instrument the QEC hybrid per-round pipeline.

```bash
quonc program.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-stats stats.json
```


### `--emit-naviz <PATH>`

Emit [MQT NAViz](https://github.com/munich-quantum-toolkit/naviz) interop
artifacts (issue #303): a `.naviz` instruction file plus a sibling `.namachine`
(zones, SLM traps, rydberg range) for rendering the atom shuttling animation in
the NAViz visualizer. Requires a `neutral_atom_reconfigurable` target and a
filesystem `PATH` (writes two sibling files; stdout is not supported). See the
[NAViz visualization guide](/guides/naviz-visualization/) for the rendering
workflow.

```bash
quonc program.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-naviz /tmp/program.naviz
# writes /tmp/program.naviz and /tmp/program.namachine
```

### `--emit-qec-experiment <PATH>`

Dual-emit the QEC experiment artifact from one `quon_qec` workload IR pass
(ADR-0018): a versioned semantic `*.qec.json` (family, distance, rounds,
logical observables, check graph, atom/site map, `error_model` snapshot, refs
into `quantum.na`) and a sibling structure-level `<stem>.stim` circuit for
Sinter. Requires a `neutral_atom_reconfigurable` target and a QEC-backed
program (e.g. `repetition_code` / `memory_round`); bare-qubit NA programs have
no experiment IR.

The `.stim` is **structure only** — no physical noise channels (ADR-0024).
Python (`python/quon_qec_sinter.py`) loads both files and annotates noise from
the JSON `error_model` before sampling. stdout dual-emit is not supported;
`PATH` must be a filesystem path.

```bash
quonc examples/na_qec/repetition_d3_memory.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-qec-experiment /tmp/rep_d3.qec.json
# writes /tmp/rep_d3.qec.json and /tmp/rep_d3.stim
```

See [`docs/neutral_atom/qec_experiment_schema.md`](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/qec_experiment_schema.md)
for the full field reference.

### `--emit-qec-validation <PATH>`

The strongest end-to-end artifact: compiles, dual-emits the QEC experiment,
builds the analytic `ResourceReport`, shells out to `python/quon_qec_sinter.py`
to sample logical failures through Stim/Sinter, and fuses analytic + sampled
evidence into `*.validation.json` (plus a sibling `*.validation.md` rendering)
with a provenance fingerprint tying the two together
([ADR-0020 amendment #280](https://github.com/arniber21/quon/blob/main/docs/adr/0020-qec-reports-remain-separate.md)).
Analytic and sampled evidence live in **separate labeled sections**; neither is
a threshold claim. Requires the Python/Sinter stack (`just setup-python`); use
`--attach-sampled` for offline fusion without shelling out. Requires a
`neutral_atom_reconfigurable` target. stdout is not supported.

```bash
quonc examples/na_qec/repetition_d3_memory.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-qec-validation /tmp/rep_d3.validation.json --validation-shots 256
```

See [`docs/neutral_atom/qec_validation_report.md`](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/qec_validation_report.md)
for the full report schema and provenance semantics, and the
[neutral-atom FT compiler demo](/guides/na-ft-demo/) for a walked-through example.

## Target options

### `--target <PATH>`

Load a backend target descriptor from JSON. Without this option, `quonc` uses
the built-in 64-qubit all-to-all `generic_openqasm` fixed target.

```bash
quonc program.qn --target backend/tests/fixtures/device_5q.json --emit-qasm
```

### `--print-target`

Print a summary of the selected target and exit without compiling. A source
file is not required.

```bash
quonc --target targets/neutral_atom/generic_rna_v0.json --print-target
```

### `--sabre-gamma <FLOAT>`
### `--sabre-beta <FLOAT>`
### `--sabre-lookahead <N>`

Tune the fixed-target SABRE routing cost model. Defaults are `0.3`, `0.5`, and
`20`.

```bash
quonc program.qn \
  --target backend/tests/fixtures/device_5q.json \
  --sabre-gamma 0.2 \
  --sabre-beta 0.7 \
  --sabre-lookahead 30 \
  --emit-qasm
```

## Neutral-atom options

### `--na-backend <zoned|flat>`

Select the neutral-atom movement backend. `zoned` is the default; aliases such
as `rap`, `aod`, and `enola` are accepted by the parser.

### `--na-placer <routing-agnostic|routing-aware>`

Select the zoned placement mode.

### `--no-na-compact`

Skip schedule compaction after neutral-atom movement or zoned scheduling.

### `--na-placement <row-major|degree|clustering>`

Select the flat AOD placement strategy.

## QEC validation options

These tune `--emit-qec-validation` (ADR-0020 amendment #280).

### `--validation-shots <N>`

Sinter shots for `--emit-qec-validation`. Default `64`.

### `--validation-seed <SEED>`

Deterministic Stim detector-sampler seed for `--emit-qec-validation`. Default `7`.

### `--validation-decoder <NAME>`

Sinter decoder for `--emit-qec-validation`. Default `pymatching`.

### `--attach-sampled <PATH>`

Fuse a pre-sampled evidence JSON (from `python/quon_qec_sinter.py --json`)
instead of shelling out to Python. Useful for offline validation or CI
environments without the Stim stack.

### `--allow-sampled-mismatch`

Warn and record the discrepancy (instead of refusing) when attached sampled
data provenance does not match the compiled artifact.

### `--python <PATH>`

Python interpreter for the Stim/Sinter harness. Default: the repo `.venv` if
present, then `python3`. Also settable via `QUON_PYTHON`.

### `--sinter-harness <PATH>`

Path to `quon_qec_sinter.py`. Default: search up from the current working
directory for `python/quon_qec_sinter.py`.

## Debug options

### `--dump-ir`

Print MLIR snapshots to standard error at the compiler's lowering, circuit,
monadic, dynamic, and physical checkpoints.

### `--verify-linear`

Run debug linearity verifiers on circuit IR and again after lowering to dynamic
IR.

### `--list-passes`

Print the compiler pass stages and exit. A source file is not required.

### `-q`, `--quiet`

Suppress the successful-compile hint.

### `--color <auto|always|never>`

Control colorized diagnostics and help. The same setting can be supplied with
`QUONC_COLOR`.

## Metrics and watch options

### `--metrics`

Print a one-line human-readable metrics summary to standard error after a
successful compile.

### `--metrics-json <PATH>`

Write the versioned metrics snapshot as JSON. Use `-` for standard output; when
combined with `--emit-qasm`, `-` writes JSON to standard error so QASM can keep
standard output.

### `--metrics-snapshot <save|compare> <PATH>`

Save the current metrics snapshot as a baseline, or compare the current run
with a saved baseline.

### `--regression-config <PATH>`

Load TOML or JSON metric tolerances for `--metrics-snapshot compare`.

### `--watch`

Watch the source file and the target JSON, when `--target` is set, then
recompile after changes. Watch mode implicitly enables metrics.

### `--watch-debounce-ms <MILLISECONDS>`

Set the watch-mode filesystem-event debounce window. The default is `300`.

## Target selection

`BackendTarget` descriptors use a `kind` field to select the architecture
family:

- `fixed` selects the fixed-connectivity gate-model path. Legacy fixed
  descriptors may omit `kind`.
- `neutral_atom_reconfigurable` selects the reconfigurable neutral-atom path.

Fixed targets use `--emit-qasm`. Neutral-atom targets use
`--emit-na-schedule` and/or `--emit-resource-report`.

## What runs

Every compile parses and typechecks the source, elaborates circuit calls,
lowers through MLIR generic-form IR, runs optimization and normalization
passes, then adapts to the selected target family. See the
[compiler pipeline reference](../compiler/).

For baseline formats, tolerance semantics, watch behavior, output routing, and
exit codes, read the
[experiment-loop guide](https://github.com/arniber21/quon/blob/main/docs/agents/experiment-loop.md).
