---
title: quonc CLI
description: Command-line reference for the Quon compiler.
---

`quonc` compiles a Quon (`.qn`) source file for a selected **hardware target**.
Fixed gate-model targets can emit OpenQASM 3 as an intermediary; neutral-atom
targets emit schedules and resource reports. See
[ADR-0010](https://github.com/arniber21/quon/blob/main/docs/adr/0010-hardware-targets-primary-openqasm-intermediary.md).

```sh
# Neutral-atom hardware path
quonc test/na/bell.qn \
  --target targets/neutral_atom/generic_rna_v0.json \
  --emit-na-schedule \
  --emit-resource-report

# Fixed path: OpenQASM intermediary
quonc program.qn --emit-qasm > program.qasm
```

Without emit or metrics flags, a successful compile prints a short confirmation
to standard error. `quonc --list-passes` prints the pass outline for both
architecture families.

## Usage

```text
quonc [OPTIONS] [SOURCE]
```

`SOURCE` is the `.qn` file to compile. It may be omitted for `--print-target`
or `--list-passes`.

## Options

### Emit

#### `--emit-qasm`

Write OpenQASM 3 for a **fixed** target to standard output (intermediary for
Aer / tooling).

```sh
quonc program.qn --emit-qasm > program.qasm
```

#### `--emit-na-schedule [PATH]`

Write the neutral-atom schedule as JSON (`-` or omitted = stdout). Requires a
`neutral_atom_reconfigurable` target. Production `quantum.na` MLIR emit is
tracked in [#167](https://github.com/arniber21/quon/issues/167).

#### `--emit-resource-report [PATH]`

Write a neutral-atom resource report (`-` = stdout; `.md` → Markdown, else
JSON). Requires a neutral-atom target.

#### `--resource-report-format <json|markdown>`

Force the resource-report format (overrides PATH extension).

### Target

#### `--target <PATH>`

Load a backend target descriptor from a JSON file. Without this option, `quonc`
uses the built-in 64-qubit, all-to-all `generic_openqasm` **fixed** target.

```sh
quonc program.qn --target targets/device.json --emit-qasm
```

#### `--print-target`

Print a summary of the selected target and exit without compiling.

#### `--sabre-gamma <F>`

SABRE noise-weight coefficient γ for **fixed** targets (default `0.3`).

### Neutral atom

#### `--na-backend <zoned|flat>`

Movement backend after entangling-layer scheduling (default `zoned` = RAP).

#### `--na-placer <routing-agnostic|routing-aware>`

Zoned placer mode (default `routing-agnostic`).

#### `--no-na-compact`

Skip schedule compaction after movement / zoned scheduling.

#### `--na-placement <row-major|degree|clustering>`

Flat AOD placement strategy (default `row-major`).

### Debug

#### `--dump-ir`

Print MLIR / schedule checkpoints to standard error.

#### `--verify-linear`

Run debug linearity verifiers on circuit IR and after dynamic lowering.

#### `--list-passes`

Print the shared front-end and per-architecture backend stages, then exit.

#### `--quiet` / `-q`

Suppress the “compiled successfully” hint.

#### `--color <auto|always|never>`

Colorize help and diagnostics (`QUONC_COLOR` env also honored).

### Metrics / watch

#### `--metrics`

Print a one-line metrics summary to standard error after a successful compile.

#### `--metrics-json <PATH>`

Write metrics JSON (`-` for stdout/stderr depending on other emits).

#### `--metrics-snapshot <save|compare> <PATH>`

Save or compare a metrics snapshot baseline.

#### `--regression-config <PATH>`

Tolerance file for `--metrics-snapshot compare`.

#### `--watch` / `--watch-debounce-ms <N>`

Recompile on source (and target) changes.

## Target selection

`BackendTarget` has an `id` and a `TargetKind`. The JSON loader chooses the
kind from the descriptor's `kind` field:

- `fixed` — gate-model connectivity, native gates, optional noise. Emit with
  `--emit-qasm`.
- `neutral_atom_reconfigurable` — zones, AOD movement, Rydberg parameters.
  Emit with `--emit-na-schedule` / `--emit-resource-report`.

```sh
quonc --target targets/neutral_atom/generic_rna_v0.json --print-target
```

See the
[neutral-atom architecture model](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/architecture_model.md)
and the [backends guide](../backends/).

## What runs

Every compile shares the front-end (parse → typecheck → `quantum.circ` →
dynamic lowering), then forks on `TargetKind`. See the
[compiler pipeline reference](../compiler/).
