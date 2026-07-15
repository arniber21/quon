# QEC benchmark methodology (#254)

Reproducible QEC evaluation joins **compiler ablations** with **tiny fixed-seed
Sinter samples**. The harness is `python/quon_qec_benchmarks.py` (also
`just qec-benchmarks-smoke` / `just qec-benchmarks-full`). Locked decisions:
ADR-0023; artifact separation: ADR-0020; Stim stack: ADR-0022.

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
2. Copies analytic schedule / sizing / `error_budget` fields into the sweep CSV
   (`evidence_kind_analytic=analytic`).
3. Runs a tiny fixed-seed Sinter sample via `quon_qec_sinter`
   (`evidence_kind_sampled=sampled`).

The sweep CSV is a **harness-level join for ablation comparison**. It does not
mutate the compiler `ResourceReport` DTO and does not replace the separate
Sinter CSV path (ADR-0020). Primary artifacts remain distinct; the join only
labels both evidence kinds on one row.

Headline columns (aligned with §11 / RAP-style physical-NA reporting names):

- `estimated_cycles`, `rydberg_stages`, `rearrangement_time_us` (movement time),
  `trap_transfers`, `measurement_rounds`, `physical_atoms`
- `error_budget_*` analytic contributions
- `logical_failures`, `logical_failure_rate` (sampled)

## Modes

| Mode | Grid | Default shots | Intended use |
| --- | --- | --- | --- |
| `smoke` | One cell: `repetition_d3_memory`, zoned, routing-agnostic, compaction on | 16 | CI (`just qec-benchmarks-smoke` / unittest) |
| `full` | All workloads × placers × backends × compaction on/off | 32 | Local only |

```bash
python python/quon_qec_benchmarks.py --mode smoke --csv /tmp/qec_smoke.csv
python python/quon_qec_benchmarks.py --mode full --csv /tmp/qec_full.csv
just qec-benchmarks-smoke
just qec-benchmarks-full
```

## Refs

- Issue #254, parent epic #245; physical-NA methodology #111
- ADR-0020, ADR-0022, ADR-0023
- `docs/neutral_atom/architecture_model.md` §11.0
- `docs/neutral_atom/literature_notes.md` ([RAP] Table I notes)
