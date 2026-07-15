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
