#!/usr/bin/env python3
from __future__ import annotations

"""
Quon QEC benchmark harness: workload × compiler ablations with nested Sinter (#254).

Runs a grid over QEC workloads (repetition / surface memory, surface CX) and
compiler ablations (`--na-placer`, `--na-backend`, compaction on/off). Each cell
emits a compiler ResourceReport + dual-emit `*.qec.json`/`.stim`, then takes a
tiny fixed-seed Sinter sample. The sweep CSV joins schedule/resource/error-budget
columns with sampled logical-failure columns while labeling both evidence kinds
(ADR-0020 / ADR-0023).

This is a **QEC compiler-ablation** experiment class. Issue #111 / RAP Table I is
the physical-NA external methodology anchor only — do **not** claim RAP numbers
for these QEC rows. Neither analytic nor sampled columns are threshold claims.

    # CI smoke — one tiny grid point
    python python/quon_qec_benchmarks.py --mode smoke --csv /tmp/qec_smoke.csv

    # Full local grid (not for CI)
    python python/quon_qec_benchmarks.py --mode full --csv /tmp/qec_full.csv

    just qec-benchmarks-smoke
    just qec-benchmarks-full
"""

import argparse
import csv
import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence, TextIO

import quon_qec_sinter as sinter_harness

# ---------------------------------------------------------------------------
# Experiment identity (ADR-0023)
# ---------------------------------------------------------------------------

EXPERIMENT_CLASS = "qec_compiler_ablation"
METHODOLOGY_ANCHOR = "issue_111_rap_table_i_physical_na_only"
DEFAULT_TARGET = "targets/neutral_atom/generic_rna_v0.json"
DEFAULT_SHOTS_SMOKE = 16
DEFAULT_SHOTS_FULL = 32
DEFAULT_SEED = 7

CSV_BANNER = (
    f"# experiment_class: {EXPERIMENT_CLASS}\n"
    "# QEC compiler-ablation sweep with nested tiny Sinter samples (issue #254 / ADR-0023).\n"
    "# Analytic schedule/error-budget columns come from --emit-resource-report; "
    "sampled logical_failures* from Stim/Sinter. Evidence kinds stay distinct "
    "(ADR-0020) — this CSV joins them for ablation comparison only.\n"
    f"# methodology_anchor: {METHODOLOGY_ANCHOR} — #111 / RAP Table I is the "
    "physical-NA external methodology anchor; do not claim RAP numbers for QEC rows.\n"
    "# Not a threshold claim: analytic error_budget and sampled "
    "logical_failure_rate are different evidence kinds (ADR-0020).\n"
)

CSV_COLUMNS = [
    "experiment_class",
    "methodology_anchor",
    "workload",
    "source",
    "na_placer",
    "na_backend",
    "na_compact",
    "distance",
    "memory_rounds",
    "shots",
    "seed",
    "evidence_kind_analytic",
    "estimated_cycles",
    "rydberg_stages",
    "rearrangement_steps",
    "rearrangement_time_us",
    "trap_transfers",
    "transfer_time_us",
    "measurement_rounds",
    "reset_rounds",
    "physical_atoms",
    "logical_qubits",
    "bottleneck",
    "error_budget_rydberg",
    "error_budget_measurement",
    "error_budget_reset",
    "error_budget_movement",
    "error_budget_transfer",
    "error_budget_idle",
    "evidence_kind_sampled",
    "logical_failures",
    "logical_failure_rate",
]

ERROR_BUDGET_KEYS = (
    "rydberg",
    "measurement",
    "reset",
    "movement",
    "transfer",
    "idle",
)

WORKLOADS: tuple[tuple[str, str], ...] = (
    ("repetition_d3_memory", "examples/na_qec/repetition_d3_memory.qn"),
    ("surface_d3_memory", "examples/na_qec/surface_d3_memory.qn"),
    ("surface_d3_cx", "examples/na_qec/surface_d3_cx.qn"),
)

PLACERS: tuple[str, ...] = ("routing-agnostic", "routing-aware")
BACKENDS: tuple[str, ...] = ("zoned", "flat")
COMPACT_FLAGS: tuple[bool, ...] = (True, False)


class BenchmarkError(Exception):
    """Harness failure with an actionable message."""


@dataclass(frozen=True)
class GridCell:
    workload: str
    source: Path
    na_placer: str
    na_backend: str
    na_compact: bool


@dataclass(frozen=True)
class BenchmarkRow:
    workload: str
    source: str
    na_placer: str
    na_backend: str
    na_compact: bool
    distance: int | None
    memory_rounds: int | None
    shots: int
    seed: int
    estimated_cycles: int
    rydberg_stages: int
    rearrangement_steps: int
    rearrangement_time_us: int
    trap_transfers: int
    transfer_time_us: int
    measurement_rounds: int
    reset_rounds: int
    physical_atoms: int
    logical_qubits: int
    bottleneck: str
    error_budget: dict[str, float]
    logical_failures: int
    logical_failure_rate: float


def expand_grid(*, mode: str, repo_root: Path | None = None) -> list[GridCell]:
    """Build the ablation grid for ``smoke`` (1 cell) or ``full`` (all cells)."""
    root = repo_root or Path.cwd()
    if mode == "smoke":
        rel = dict(WORKLOADS)["repetition_d3_memory"]
        return [
            GridCell(
                workload="repetition_d3_memory",
                source=root / rel,
                na_placer="routing-agnostic",
                na_backend="zoned",
                na_compact=True,
            )
        ]
    if mode != "full":
        raise BenchmarkError(f"unknown mode {mode!r} (expected smoke or full)")
    cells: list[GridCell] = []
    for workload, rel in WORKLOADS:
        for placer in PLACERS:
            for backend in BACKENDS:
                for compact in COMPACT_FLAGS:
                    cells.append(
                        GridCell(
                            workload=workload,
                            source=root / rel,
                            na_placer=placer,
                            na_backend=backend,
                            na_compact=compact,
                        )
                    )
    return cells


def resolve_quonc(repo_root: Path, explicit: str | None = None) -> Path | None:
    """Locate quonc: ``--quonc`` / ``QUONC`` / release / debug / PATH."""
    candidates: list[Path] = []
    if explicit:
        candidates.append(Path(explicit))
    env = os.environ.get("QUONC")
    if env:
        candidates.append(Path(env))
    candidates.append(repo_root / "target" / "release" / "quonc")
    candidates.append(repo_root / "target" / "debug" / "quonc")
    which = shutil.which("quonc")
    if which:
        candidates.append(Path(which))
    for path in candidates:
        if path.is_file() and os.access(path, os.X_OK):
            return path.resolve()
    return None


def _require_int(doc: Mapping[str, Any], key: str, default: int = 0) -> int:
    value = doc.get(key, default)
    if isinstance(value, bool) or not isinstance(value, int):
        raise BenchmarkError(f"resource report field {key!r} must be an int (got {value!r})")
    return value


def _optional_int(doc: Mapping[str, Any], key: str) -> int | None:
    if key not in doc or doc[key] is None:
        return None
    value = doc[key]
    if isinstance(value, bool) or not isinstance(value, int):
        raise BenchmarkError(f"resource report field {key!r} must be an int (got {value!r})")
    return value


def load_resource_report(path: Path) -> dict[str, Any]:
    try:
        doc = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise BenchmarkError(f"failed to read resource report {path}: {exc}") from exc
    if not isinstance(doc, dict):
        raise BenchmarkError(f"{path}: expected a JSON object")
    if doc.get("evidence_kind") not in (None, "analytic"):
        raise BenchmarkError(
            f"{path}: evidence_kind must be 'analytic' (got {doc.get('evidence_kind')!r})"
        )
    return doc


def join_cell(
    cell: GridCell,
    *,
    report: Mapping[str, Any],
    sample: sinter_harness.SampleResult,
    seed: int,
) -> BenchmarkRow:
    budget_raw = report.get("error_budget") or {}
    if not isinstance(budget_raw, dict):
        raise BenchmarkError("resource report error_budget must be an object when present")
    budget: dict[str, float] = {}
    for key in ERROR_BUDGET_KEYS:
        value = budget_raw.get(key, 0.0)
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise BenchmarkError(f"error_budget[{key!r}] must be a number")
        budget[key] = float(value)

    source = str(cell.source)
    try:
        # Prefer repo-relative paths in CSV when possible.
        source = str(cell.source.resolve().relative_to(Path.cwd().resolve()))
    except ValueError:
        pass

    return BenchmarkRow(
        workload=cell.workload,
        source=source,
        na_placer=cell.na_placer,
        na_backend=cell.na_backend,
        na_compact=cell.na_compact,
        distance=_optional_int(report, "distance"),
        memory_rounds=_optional_int(report, "memory_rounds"),
        shots=sample.shots,
        seed=seed,
        estimated_cycles=_require_int(report, "estimated_cycles"),
        rydberg_stages=_require_int(report, "rydberg_stages"),
        rearrangement_steps=_require_int(report, "rearrangement_steps"),
        rearrangement_time_us=_require_int(report, "rearrangement_time_us"),
        trap_transfers=_require_int(report, "trap_transfers"),
        transfer_time_us=_require_int(report, "transfer_time_us"),
        measurement_rounds=_require_int(report, "measurement_rounds"),
        reset_rounds=_require_int(report, "reset_rounds"),
        physical_atoms=_require_int(report, "physical_atoms"),
        logical_qubits=_require_int(report, "logical_qubits"),
        bottleneck=str(report.get("bottleneck", "none")),
        error_budget=budget,
        logical_failures=sample.logical_failures,
        logical_failure_rate=sample.logical_failure_rate,
    )


def write_csv(out: TextIO, rows: Sequence[BenchmarkRow]) -> None:
    """Write the ablation join CSV with evidence banners (not a threshold claim)."""
    out.write(CSV_BANNER)
    writer = csv.DictWriter(out, fieldnames=CSV_COLUMNS)
    writer.writeheader()
    for row in rows:
        record = {
            "experiment_class": EXPERIMENT_CLASS,
            "methodology_anchor": METHODOLOGY_ANCHOR,
            "workload": row.workload,
            "source": row.source,
            "na_placer": row.na_placer,
            "na_backend": row.na_backend,
            "na_compact": "true" if row.na_compact else "false",
            "distance": "" if row.distance is None else row.distance,
            "memory_rounds": "" if row.memory_rounds is None else row.memory_rounds,
            "shots": row.shots,
            "seed": row.seed,
            "evidence_kind_analytic": "analytic",
            "estimated_cycles": row.estimated_cycles,
            "rydberg_stages": row.rydberg_stages,
            "rearrangement_steps": row.rearrangement_steps,
            "rearrangement_time_us": row.rearrangement_time_us,
            "trap_transfers": row.trap_transfers,
            "transfer_time_us": row.transfer_time_us,
            "measurement_rounds": row.measurement_rounds,
            "reset_rounds": row.reset_rounds,
            "physical_atoms": row.physical_atoms,
            "logical_qubits": row.logical_qubits,
            "bottleneck": row.bottleneck,
            "evidence_kind_sampled": "sampled",
            "logical_failures": row.logical_failures,
            "logical_failure_rate": row.logical_failure_rate,
        }
        for key in ERROR_BUDGET_KEYS:
            record[f"error_budget_{key}"] = row.error_budget.get(key, 0.0)
        writer.writerow(record)


def evaluate_artifacts(
    cell: GridCell,
    *,
    report_path: Path,
    experiment_json: Path,
    shots: int,
    seed: int,
) -> BenchmarkRow:
    """Join a ResourceReport JSON with a tiny Sinter sample (no quonc invoke)."""
    report = load_resource_report(report_path)
    sinter_rows = sinter_harness.run_experiments(
        [experiment_json],
        shots_list=[shots],
        seed=seed,
    )
    if len(sinter_rows) != 1:
        raise BenchmarkError(f"expected one Sinter row, got {len(sinter_rows)}")
    sample = sinter_harness.SampleResult(
        shots=sinter_rows[0].shots,
        logical_failures=sinter_rows[0].logical_failures,
    )
    return join_cell(cell, report=report, sample=sample, seed=seed)


def compile_cell(
    cell: GridCell,
    *,
    quonc: Path,
    target: Path,
    work_dir: Path,
) -> tuple[Path, Path]:
    """Run quonc dual emit: resource report JSON + QEC experiment pair."""
    if not cell.source.is_file():
        raise BenchmarkError(f"workload source not found: {cell.source}")
    if not target.is_file():
        raise BenchmarkError(f"NA target not found: {target}")

    cell_dir = work_dir / (
        f"{cell.workload}__{cell.na_backend}__{cell.na_placer}__"
        f"{'compact' if cell.na_compact else 'nocompact'}"
    )
    cell_dir.mkdir(parents=True, exist_ok=True)
    report_path = cell_dir / "resource_report.json"
    experiment_json = cell_dir / f"{cell.workload}.qec.json"

    cmd = [
        str(quonc),
        str(cell.source),
        "--target",
        str(target),
        "--na-backend",
        cell.na_backend,
        "--na-placer",
        cell.na_placer,
        "--emit-resource-report",
        str(report_path),
        "--emit-qec-experiment",
        str(experiment_json),
        "--quiet",
    ]
    if not cell.na_compact:
        cmd.append("--no-na-compact")

    try:
        proc = subprocess.run(
            cmd,
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError as exc:
        raise BenchmarkError(f"failed to invoke quonc: {exc}") from exc
    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout or "").strip()
        raise BenchmarkError(
            f"quonc failed for {cell.workload} "
            f"(backend={cell.na_backend}, placer={cell.na_placer}, "
            f"compact={cell.na_compact}):\n{detail}"
        )
    if not report_path.is_file() or not experiment_json.is_file():
        raise BenchmarkError(
            f"quonc did not write expected artifacts under {cell_dir}"
        )
    return report_path, experiment_json


def run_suite(
    *,
    mode: str,
    repo_root: Path,
    quonc: Path,
    target: Path,
    work_dir: Path,
    shots: int,
    seed: int,
) -> list[BenchmarkRow]:
    cells = expand_grid(mode=mode, repo_root=repo_root)
    rows: list[BenchmarkRow] = []
    for cell in cells:
        report_path, experiment_json = compile_cell(
            cell, quonc=quonc, target=target, work_dir=work_dir
        )
        rows.append(
            evaluate_artifacts(
                cell,
                report_path=report_path,
                experiment_json=experiment_json,
                shots=shots,
                seed=seed,
            )
        )
    return rows


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        formatter_class=argparse.RawDescriptionHelpFormatter,
        description=(
            "QEC workload × compiler ablation grid with nested tiny Sinter "
            "samples (issue #254 / ADR-0023).\n"
            "Joins analytic ResourceReport fields with sampled logical failures "
            "in one sweep CSV while keeping evidence kinds labeled (ADR-0020).\n"
            "Experiment class is QEC compiler-ablation — #111 / RAP Table I is "
            "physical-NA methodology only; no RAP number claims; no thresholds."
        ),
        epilog="""
Modes
  smoke   One tiny grid point for CI (repetition_d3_memory, zoned,
          routing-agnostic, compaction on). Default --shots 16.
  full    Local-only full grid: repetition + surface memory + surface CX
          × --na-placer × --na-backend × compaction on/off. Default --shots 32.

Examples
  # CI smoke
  python python/quon_qec_benchmarks.py --mode smoke --csv /tmp/qec_smoke.csv

  # Full local ablation grid (not for CI)
  python python/quon_qec_benchmarks.py --mode full --csv /tmp/qec_full.csv

  just qec-benchmarks-smoke
  just qec-benchmarks-full

Methodology
  Field names align with the physical-NA #111 / RAP Table I reporting style
  (cycles, Rydberg stages, rearrangement time, transfers, …), but QEC rows are
  a distinct experiment class. See docs/neutral_atom/qec_benchmark_methodology.md.
""",
    )
    parser.add_argument(
        "--mode",
        choices=("smoke", "full"),
        default="smoke",
        help="smoke = CI one-cell; full = local ablation grid (default: smoke)",
    )
    parser.add_argument(
        "--repo-root",
        type=Path,
        default=None,
        help="repository root (default: cwd)",
    )
    parser.add_argument(
        "--quonc",
        type=str,
        default=None,
        help="path to quonc (default: QUONC env, then target/release/quonc)",
    )
    parser.add_argument(
        "--target",
        type=Path,
        default=None,
        help=f"NA target JSON (default: {{repo}}/{DEFAULT_TARGET})",
    )
    parser.add_argument(
        "--work-dir",
        type=Path,
        default=None,
        help="directory for per-cell quonc artifacts (default: temp under cwd)",
    )
    parser.add_argument(
        "--shots",
        type=int,
        default=None,
        help=f"Sinter shots per cell (default: {DEFAULT_SHOTS_SMOKE} smoke / "
        f"{DEFAULT_SHOTS_FULL} full)",
    )
    parser.add_argument(
        "--seed",
        type=int,
        default=DEFAULT_SEED,
        help=f"fixed Stim sampler seed (default: {DEFAULT_SEED})",
    )
    parser.add_argument(
        "--csv",
        type=str,
        default=None,
        help="write sweep CSV to this path (default: stdout)",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_arg_parser()
    args = parser.parse_args(argv)
    repo_root = (args.repo_root or Path.cwd()).resolve()
    target = (args.target or (repo_root / DEFAULT_TARGET)).resolve()
    shots = args.shots
    if shots is None:
        shots = DEFAULT_SHOTS_SMOKE if args.mode == "smoke" else DEFAULT_SHOTS_FULL
    if shots < 1:
        print("error: --shots must be >= 1", file=sys.stderr)
        return 2

    quonc = resolve_quonc(repo_root, args.quonc)
    if quonc is None:
        print(
            "error: quonc not found. Build with `cargo build --release -p quonc` "
            "or set QUONC / pass --quonc.",
            file=sys.stderr,
        )
        return 1

    work_dir = args.work_dir
    cleanup = False
    if work_dir is None:
        import tempfile

        work_dir = Path(tempfile.mkdtemp(prefix="quon_qec_bench_"))
        cleanup = True
    else:
        work_dir = work_dir.resolve()
        work_dir.mkdir(parents=True, exist_ok=True)

    try:
        rows = run_suite(
            mode=args.mode,
            repo_root=repo_root,
            quonc=quonc,
            target=target,
            work_dir=work_dir,
            shots=shots,
            seed=args.seed,
        )
    except (BenchmarkError, sinter_harness.HarnessError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    finally:
        if cleanup:
            shutil.rmtree(work_dir, ignore_errors=True)

    if args.csv:
        with open(args.csv, "w", newline="") as f:
            write_csv(f, rows)
    else:
        write_csv(sys.stdout, rows)
    return 0


if __name__ == "__main__":
    sys.exit(main())
