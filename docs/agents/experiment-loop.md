# Experiment loop — watch mode, metrics, and regression checks

Algorithm iteration in Quon should be a tight **edit → recompile → compare** loop instead of ad-hoc shell scripts. Issue #48 adds structured compile metrics, baseline snapshots, and optional filesystem watching to `quonc`.

## Quick start

```bash
# Human-readable metrics on stderr after compile
quonc programs/algo.qn --metrics

# Machine-readable JSON (stdout, or stderr when --emit-qasm is active)
quonc programs/algo.qn --metrics-json metrics.json
quonc programs/algo.qn --emit-qasm --metrics-json -   # JSON on stderr

# Watch mode: recompile when the source (and optional --target JSON) changes
quonc programs/algo.qn --watch --target devices/chain5.json --metrics
```

Watch mode implicitly enables `--metrics`. Each successful recompile prints a metrics line with deltas vs the previous run.

## Metrics schema

Each compile produces a versioned JSON document (`schema_version: 1`) with:

- **Program provenance** — source path, SHA-256, entry name
- **Target** — backend id and optional descriptor path
- **Toolchain** — `quonc` version, git commit (best-effort), dirty flag
- **Compile** — status, wall-clock `compile_ms`, error string on failure
- **Metrics** — `depth`, `gate_count`, `t_count`, `qubit_count`, `swap_count`

`depth` is scheduled critical-path depth (`max(schedule_time) + 1`) after the physical pipeline. `t_count` is collected after routing, before the second native-gate decomposition pass.

**Conservative semantics:** programs with `quantum.dynamic.if` count gates in **all** branches (upper bound), not executed-path metrics.

## Baseline snapshots

Save a baseline before refactoring or pass changes:

```bash
quonc programs/algo.qn --target devices/chain5.json \
  --metrics-snapshot save metrics/baselines/algo-chain5.json
```

Compare later runs against the baseline:

```bash
quonc programs/algo.qn --target devices/chain5.json \
  --metrics-snapshot compare metrics/baselines/algo-chain5.json \
  --regression-config metrics/tolerances.toml
```

Exit codes:

- `0` — compare passed (or successful one-shot compile)
- `1` — regression detected (`compare` only)
- `2` — tool error (bad paths, schema mismatch, compile failure on save)

In **watch + compare** mode, regressions print `FAIL` to stderr but the watch loop continues.

## Regression tolerances

Default tolerances (no config file):

| Metric | Default |
|--------|---------|
| `depth`, `gate_count`, `t_count`, `swap_count` | absolute 0 |
| `compile_ms` | relative 10% |

Example `metrics/tolerances.toml`:

```toml
[metrics.depth]
absolute = 2

[metrics.gate_count]
absolute = 5

[metrics.compile_ms]
relative = 0.25
absolute = 50

[ignore]
fields = ["compile_ms"]
```

When both absolute and relative bounds are set, **either** bound passing is enough (lenient).

## Recommended terminal layout

Split pane: editor on one side, watch loop on the other.

```
┌─────────────────────┬──────────────────────────────┐
│  algo.qn (editor)   │  quonc algo.qn --watch       │
│                     │    --target device.json      │
│                     │    --metrics                 │
└─────────────────────┴──────────────────────────────┘
```

Use `--watch-debounce-ms 300` (default) to coalesce editor save bursts.

## CI example

Commit baselines under `metrics/baselines/` and compare in CI:

```yaml
- name: Metrics regression
  run: |
    quonc programs/reference.qn --target devices/generic.json \
      --metrics-snapshot compare metrics/baselines/reference.json \
      --regression-config metrics/tolerances.toml
```

## Library API

Integration tests and tools can call the compiler without subprocesses:

```rust
use quonc::{CompileRequest, compile};

let report = compile(&request);
assert_eq!(report.snapshot.metrics.as_ref().unwrap().depth, 87);
```

See `quonc/src/compile.rs` and `quon_core::metrics` for snapshot types and comparison helpers.
