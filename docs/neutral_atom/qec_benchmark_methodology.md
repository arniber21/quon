# QEC benchmark methodology (#254)

Reproducible QEC evaluation joins **compiler ablations** with **tiny fixed-seed
Sinter samples**. The harness is `python/quon_qec_benchmarks.py` (also
`just qec-benchmarks-smoke` / `just qec-benchmarks-axis` /
`just qec-benchmarks-full`). Locked decisions: ADR-0023; artifact separation:
ADR-0020 (amended for optional join CSV); Stim stack: ADR-0022; noise
annotation: ADR-0024.

## Experiment class

| Class | What it measures | External anchor |
| --- | --- | --- |
| **Physical-NA** (`#111`) | Gate-model / physical entangling workloads on zoned NA (e.g. RAP Table I `ising` rearrangement steps and √-law time) | [RAP] Table I (Sec. VI-B) — see `architecture_model.md` §4 / `literature_notes.md` |
| **QEC compiler-ablation** (`#254`) | Hybrid QEC workloads (repetition / surface memory, optional lattice-surgery CX) under `--na-placer` × `--na-backend` × compaction, plus a nested Stim/Sinter logical-failure sample | **Methodology style** of #111 (same schedule headline field names) — **not** RAP Table I numeric claims |

CSV rows set `experiment_class=qec_compiler_ablation` and
`methodology_anchor=issue_111_rap_table_i_physical_na_only`. Do **not** report
QEC rows as reproducing RAP Table I, and do **not** treat analytic
`error_budget` or sampled `logical_failure_rate` as a threshold claim.

## What each cell records

For every grid cell the harness:

1. Compiles with `quonc` (`--emit-resource-report` + `--emit-qec-experiment`).
2. Keeps primary artifacts under `--work-dir`: `resource_report.json`,
   `*.qec.json`, sibling `.stim`, and a per-cell `sinter.csv`.
3. Copies analytic schedule / sizing / `error_budget` fields into the optional
   **join** sweep CSV (`evidence_kind_analytic=analytic`).
4. Runs a tiny fixed-seed Sinter sample via `quon_qec_sinter` and also writes an
   aggregated **separate** Sinter CSV (sampled-only columns;
   `evidence_kind=sampled`).

The join CSV is a **harness-level comparison aid** (ADR-0020 amendment). It does
not mutate the compiler `ResourceReport` DTO and does **not** replace the
separate Sinter CSV or the dual-emit / report primaries. Primaries are kept by
default (`--cleanup` opts into deleting an auto-created work dir).

### Nested Sinter is schedule-agnostic (ADR-0024)

Noise is annotated in Python from JSON `error_model` **rate proxies**, not from
NA schedule event counts. Therefore:

- **Analytic** columns (`estimated_cycles`, `rydberg_stages`,
  `rearrangement_time_us`, `trap_transfers`, `error_budget_*`, …) track
  placer / backend / compaction ablations.
- **Sampled** `logical_failures` / `logical_failure_rate` may be **invariant**
  across placer / backend / compaction for the same workload + error model.
  Do not interpret flat sampled columns as “ablation had no schedule effect.”

### Flat × placer Cartesian

`--na-placer` is a **zoned-path** option (`quonc` ignores it unless
`--na-backend=zoned`). When flat cells are enumerated (`--include-flat`),
flat×placer pairs are **placer no-ops** (uniform CSV shape only).

**Default grids are zoned-only.** Flat AOD currently **fail-closes** on these
QEC workloads with `generic_rna_v0` (Rydberg geometry / B11). Unsupported cells
raise a clear `BenchmarkError` wrapping the quonc message — no silent skip.
Use `--include-flat` only to exercise that fail-closed path; do not expect a
green full grid with flat until a QEC-compatible flat target exists.

Headline columns (aligned with §11 / RAP-style physical-NA reporting names):

- `estimated_cycles`, `rydberg_stages`, `rearrangement_time_us` (movement time),
  `trap_transfers`, `measurement_rounds`, `physical_atoms`
- `error_budget_*` analytic contributions (hard-fail if missing from the report)
- `logical_failures`, `logical_failure_rate` (sampled)

## Modes

| Mode | Grid | Default shots | Intended use |
| --- | --- | --- | --- |
| `smoke` | One cell: `repetition_d3_memory`, zoned, routing-agnostic, compaction on | 16 | CI unittest in `just ci-rust`; local convenience via `just qec-benchmarks-smoke` |
| `axis` | Each supported ablation axis once + one CX cell (4 zoned cells: placers × compaction + CX) | 16 | **Gates** the full grid: compile+Sinter per placer, compaction, plus CX. CI unittest when quonc present (`CI=1` fails if quonc missing). Also `just qec-benchmarks-axis`. Flat is covered by a separate fail-closed test. |
| `full` | All workloads × placers × compaction on/off on **zoned** (12 cells) | 32 | Local only (`just qec-benchmarks-full`). Proven by `axis` coverage. `--include-flat` adds flat cells that currently fail closed. |

```bash
python python/quon_qec_benchmarks.py --mode smoke --csv /tmp/qec_smoke.csv
python python/quon_qec_benchmarks.py --mode axis --csv /tmp/qec_axis.csv
python python/quon_qec_benchmarks.py --mode full --csv /tmp/qec_full.csv
python python/quon_qec_benchmarks.py --mode full --dry-run-compile
just qec-benchmarks-smoke   # local convenience
just qec-benchmarks-axis
just qec-benchmarks-full
```

**CI vs just recipes:** `just ci-rust` runs
`python -m unittest python/test_quon_qec_benchmarks.py` (smoke + axis when
quonc is built). The `just qec-benchmarks-*` recipes are local convenience
wrappers around the CLI; they are not a substitute for the CI unittest gate.

## Refs

- Issue #254, parent epic #245; physical-NA methodology #111
- ADR-0020 (amended), ADR-0022, ADR-0023, ADR-0024
- `docs/neutral_atom/architecture_model.md` §11.0
- `docs/neutral_atom/literature_notes.md` ([RAP] Table I notes)
