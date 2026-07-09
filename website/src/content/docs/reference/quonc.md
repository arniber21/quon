---
title: quonc CLI
description: Command-line reference for the Quon compiler.
---

`quonc` compiles a Quon (`.qn`) source file for a fixed-connectivity target. It
always runs the compiler; pass `--emit-qasm` when you also want the generated
OpenQASM 3 on standard output.

```sh
quonc program.qn --emit-qasm > program.qasm
```

Without output or metrics flags, a successful compile prints a short
confirmation to standard error.

## Usage

```text
quonc [OPTIONS] [SOURCE]
```

`SOURCE` is the `.qn` file to compile. It may be omitted only when
`--print-target` is used.

## Options

### `--emit-qasm`

Write the generated OpenQASM 3 program to standard output.

```sh
quonc program.qn --emit-qasm > program.qasm
```

### `--target <PATH>`

Load a backend target descriptor from a JSON file. Without this option, `quonc`
uses the built-in 64-qubit, all-to-all `generic_openqasm` fixed target.

```sh
quonc program.qn --target targets/device.json --emit-qasm
```

See [Target selection](#target-selection) for the descriptor kinds the loader
accepts and the kinds the compiler can currently compile.

### `--print-target`

Print a summary of the selected target and exit without compiling. A source
file is not required.

```sh
quonc --target targets/device.json --print-target
```

### `--dump-ir`

Print MLIR snapshots to standard error at the compiler's lowering, circuit,
monadic, dynamic, and physical pipeline checkpoints.

```sh
quonc program.qn --dump-ir
```

### `--verify-linear`

Run the debug linearity verifiers on circuit IR and again after lowering to
dynamic IR.

```sh
quonc program.qn --verify-linear
```

### `--metrics`

Print a one-line, human-readable metrics summary to standard error after a
successful compile.

```sh
quonc program.qn --metrics
```

### `--metrics-json <PATH>`

Write the versioned metrics snapshot as JSON. Use `-` for standard output; when
combined with `--emit-qasm`, `-` writes JSON to standard error so QASM can keep
standard output.

```sh
quonc program.qn --metrics-json metrics/run.json
```

### `--metrics-snapshot <ACTION> <PATH>`

Save the current metrics snapshot as a baseline, or compare it with a saved
baseline. `ACTION` is `save` or `compare`.

```sh
quonc program.qn --metrics-snapshot save metrics/baseline.json
quonc program.qn --metrics-snapshot compare metrics/baseline.json
```

### `--regression-config <PATH>`

Load TOML or JSON metric tolerances for a `--metrics-snapshot compare`.

```sh
quonc program.qn \
  --metrics-snapshot compare metrics/baseline.json \
  --regression-config metrics/tolerances.toml
```

### `--watch`

Watch the source file, and the target JSON when `--target` is set, then
recompile after changes. Watch mode implicitly enables `--metrics`.

```sh
quonc program.qn --watch --target targets/device.json
```

### `--watch-debounce-ms <MILLISECONDS>`

Set the watch-mode filesystem-event debounce window. The default is 300 ms.

```sh
quonc program.qn --watch --watch-debounce-ms 500
```

### `-h`, `--help`

Print command help and exit.

```sh
quonc --help
```

### `-V`, `--version`

Print the `quonc` version and exit.

```sh
quonc --version
```

For baseline formats, tolerance semantics, watch behavior, output routing, and
exit codes, read the
[experiment-loop guide](https://github.com/arniber21/quon/blob/main/docs/agents/experiment-loop.md).

## Target selection

`BackendTarget` has an `id` and a `TargetKind`. The JSON loader chooses the
kind from the descriptor's `kind` field:

- `fixed` selects the fixed-connectivity gate-model architecture. Legacy fixed
  descriptors may omit `kind`.
- `neutral_atom_reconfigurable` selects the separate reconfigurable
  neutral-atom architecture family.

The current OpenQASM compile pipeline accepts **fixed targets only**. For a
fixed target, `--target` controls its qubit count, connectivity, native gates,
noise data, measurement latency, and dynamic-circuit capability flags. Native
gate decomposition, routing, scheduling, metrics, and emission use that loaded
target.

Neutral-atom descriptors can currently be loaded and inspected with
`--print-target`, but `quonc` rejects them for compilation. This is a distinct
architecture family, not a fixed target with extra fields. See the
[neutral-atom architecture model](https://github.com/arniber21/quon/blob/main/docs/neutral_atom/architecture_model.md).

```sh
quonc \
  --target targets/neutral_atom/generic_rna_v0.json \
  --print-target
```

## What runs

Every compile follows the same high-level path: parse and typecheck the source,
elaborate and lower it to MLIR, optimize and lower the IR, adapt it to the
fixed target, then emit OpenQASM 3. See the
[compiler pipeline reference](../compiler/).
