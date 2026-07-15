#!/usr/bin/env python3
from __future__ import annotations

"""
Quon QEC benchmark harness: workload × compiler ablations with nested Sinter (#254).

Runs a grid over QEC workloads (repetition / surface memory, surface CX) and
compiler ablations (`--na-placer`, `--na-backend`, compaction on/off). Each cell
emits a compiler ResourceReport + dual-emit `*.qec.json`/`.stim`, runs a tiny
fixed-seed Sinter sample, writes a **separate** sampled Sinter CSV, and appends
one labeled join-CSV row for ablation comparison (ADR-0020 amendment / ADR-0023).

Nested Sinter samples are **schedule-agnostic under ADR-0024**: noise comes from
JSON `error_model` rate proxies, not from NA schedule event counts. Analytic
columns track placer/backend/compaction ablations; sampled `logical_failures`
may be invariant across those axes.

This is a **QEC compiler-ablation** experiment class. Issue #111 / RAP Table I is
the physical-NA external methodology anchor only — do **not** claim RAP numbers
for these QEC rows. Neither analytic nor sampled columns are threshold claims.

    # CI smoke — one tiny grid point
    python python/quon_qec_benchmarks.py --mode smoke --csv /tmp/qec_smoke.csv

    # Axis coverage (each supported ablation axis once + CX) — proves full-grid flags
    python python/quon_qec_benchmarks.py --mode axis --csv /tmp/qec_axis.csv

    # Full local grid (zoned; not for CI)
    python python/quon_qec_benchmarks.py --mode full --csv /tmp/qec_full.csv

    just qec-benchmarks-smoke
    just qec-benchmarks-axis
    just qec-benchmarks-full
"""

import argparse
import csv
import json
import os
import shutil
import subprocess
import sys
import tempfile
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
DEFAULT_SHOTS_AXIS = 16
DEFAULT_SHOTS_FULL = 32
DEFAULT_SEED = 7

CSV_BANNER = (
    f"# experiment_class: {EXPERIMENT_CLASS}\n"
    "# QEC compiler-ablation sweep with nested tiny Sinter samples (issue #254 / ADR-0023).\n"
    "# Analytic schedule/error-budget columns come from --emit-resource-report; "
    "sampled logical_failures* from Stim/Sinter. Evidence kinds stay distinct "
    "(ADR-0020) — this CSV is an optional harness join for ablation comparison only.\n"
    "# Primary artifacts stay separate: ResourceReport JSON, *.qec.json/.stim, "
    "and a sibling Sinter CSV (sampled-only columns).\n"
    "# Nested Sinter is schedule-agnostic under ADR-0024 (noise from error_model "
    "rate proxies, not NA schedule counts); analytic columns track ablations; "
    "sampled logical_failures may be invariant across placer/backend/compaction.\n"
    f"# methodology_anchor: {METHODOLOGY_ANCHOR} — #111 / RAP Table I is the "
    "physical-NA external methodology anchor; do not claim RAP numbers for QEC rows.\n"
    "# Not a threshold claim: analytic error_budget and sampled "
    "logical_failure_rate are different evidence kinds (ADR-0020).\n"
    "# Note: --na-placer is a zoned-path option; if --include-flat is used, "
    "flat×placer cells are placer no-ops (quonc ignores --na-placer unless "
    "--na-backend=zoned). Flat QEC on generic_rna_v0 currently fail-closes "
    "(Rydberg geometry) — default grids are zoned-only.\n"
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

# Required ResourceReport integer fields (hard-fail if missing).
REQUIRED_REPORT_INTS = (
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
)

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
# Default successful grid is zoned-only: flat AOD fail-closes on these QEC
# workloads with generic_rna_v0 (Rydberg geometry). Opt in with --include-flat.
BACKENDS: tuple[str, ...] = ("zoned",)
FLAT_BACKEND = "flat"
COMPACT_FLAGS: tuple[bool, ...] = (True, False)

MODES = ("smoke", "axis", "full")


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


def _workload_source(repo_root: Path, name: str) -> Path:
    rel = dict(WORKLOADS)[name]
    return repo_root / rel


def expand_grid(
    *,
    mode: str,
    repo_root: Path | None = None,
    include_flat: bool = False,
) -> list[GridCell]:
    """Build the ablation grid for ``smoke``, ``axis``, or ``full``.

    Default backends are zoned-only. ``include_flat`` adds flat cells that
    currently fail closed on QEC + generic_rna_v0 (clear quonc geometry error).
    """
    root = repo_root or Path.cwd()
    backends: tuple[str, ...] = BACKENDS
    if include_flat:
        backends = tuple(dict.fromkeys((*BACKENDS, FLAT_BACKEND)))

    if mode == "smoke":
        return [
            GridCell(
                workload="repetition_d3_memory",
                source=_workload_source(root, "repetition_d3_memory"),
                na_placer="routing-agnostic",
                na_backend="zoned",
                na_compact=True,
            )
        ]
    if mode == "axis":
        # Reduced coverage: each supported ablation axis once + one CX cell.
        # Proves full-grid flag combinations without the full Cartesian.
        # Flat is not part of the success path — see flat fail-closed tests.
        rep = _workload_source(root, "repetition_d3_memory")
        cx = _workload_source(root, "surface_d3_cx")
        cells = [
            GridCell(
                workload="repetition_d3_memory",
                source=rep,
                na_placer="routing-agnostic",
                na_backend="zoned",
                na_compact=True,
            ),
            GridCell(
                workload="repetition_d3_memory",
                source=rep,
                na_placer="routing-aware",
                na_backend="zoned",
                na_compact=True,
            ),
            GridCell(
                workload="repetition_d3_memory",
                source=rep,
                na_placer="routing-agnostic",
                na_backend="zoned",
                na_compact=False,
            ),
            GridCell(
                workload="surface_d3_cx",
                source=cx,
                na_placer="routing-agnostic",
                na_backend="zoned",
                na_compact=True,
            ),
        ]
        if include_flat:
            cells.append(
                GridCell(
                    workload="repetition_d3_memory",
                    source=rep,
                    na_placer="routing-agnostic",
                    na_backend=FLAT_BACKEND,
                    na_compact=True,
                )
            )
        return cells
    if mode != "full":
        raise BenchmarkError(
            f"unknown mode {mode!r} (expected {', '.join(MODES)})"
        )
    cells: list[GridCell] = []
    for workload, rel in WORKLOADS:
        for placer in PLACERS:
            for backend in backends:
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


def _require_int(doc: Mapping[str, Any], key: str) -> int:
    if key not in doc:
        raise BenchmarkError(f"resource report missing required field {key!r}")
    value = doc[key]
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


def _require_error_budget(doc: Mapping[str, Any]) -> dict[str, float]:
    if "error_budget" not in doc:
        raise BenchmarkError("resource report missing required field 'error_budget'")
    budget_raw = doc["error_budget"]
    if not isinstance(budget_raw, dict):
        raise BenchmarkError("resource report error_budget must be an object")
    budget: dict[str, float] = {}
    for key in ERROR_BUDGET_KEYS:
        if key not in budget_raw:
            raise BenchmarkError(f"resource report error_budget missing key {key!r}")
        value = budget_raw[key]
        if isinstance(value, bool) or not isinstance(value, (int, float)):
            raise BenchmarkError(f"error_budget[{key!r}] must be a number")
        budget[key] = float(value)
    return budget


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
    budget = _require_error_budget(report)
    if "bottleneck" not in report:
        raise BenchmarkError("resource report missing required field 'bottleneck'")

    source = str(cell.source)
    try:
        # Prefer repo-relative paths in CSV when possible.
        source = str(cell.source.resolve().relative_to(Path.cwd().resolve()))
    except ValueError:
        pass

    ints = {key: _require_int(report, key) for key in REQUIRED_REPORT_INTS}

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
        estimated_cycles=ints["estimated_cycles"],
        rydberg_stages=ints["rydberg_stages"],
        rearrangement_steps=ints["rearrangement_steps"],
        rearrangement_time_us=ints["rearrangement_time_us"],
        trap_transfers=ints["trap_transfers"],
        transfer_time_us=ints["transfer_time_us"],
        measurement_rounds=ints["measurement_rounds"],
        reset_rounds=ints["reset_rounds"],
        physical_atoms=ints["physical_atoms"],
        logical_qubits=ints["logical_qubits"],
        bottleneck=str(report["bottleneck"]),
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
            record[f"error_budget_{key}"] = row.error_budget[key]
        writer.writerow(record)


def cell_dir_name(cell: GridCell) -> str:
    return (
        f"{cell.workload}__{cell.na_backend}__{cell.na_placer}__"
        f"{'compact' if cell.na_compact else 'nocompact'}"
    )


def compile_cell_argv(
    cell: GridCell,
    *,
    quonc: Path,
    target: Path,
    report_path: Path,
    experiment_json: Path,
) -> list[str]:
    """Build the quonc argv for one cell (pure; no subprocess)."""
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
    return cmd


def compile_cell(
    cell: GridCell,
    *,
    quonc: Path,
    target: Path,
    work_dir: Path,
    dry_run: bool = False,
) -> tuple[Path, Path, list[str]]:
    """Run quonc dual emit: resource report JSON + QEC experiment pair.

    Returns ``(report_path, experiment_json, argv)``. On ``dry_run``, validates
    paths and returns argv without invoking quonc (artifacts are not written).
    """
    if not cell.source.is_file():
        raise BenchmarkError(f"workload source not found: {cell.source}")
    if not target.is_file():
        raise BenchmarkError(f"NA target not found: {target}")

    cell_dir = work_dir / cell_dir_name(cell)
    cell_dir.mkdir(parents=True, exist_ok=True)
    report_path = cell_dir / "resource_report.json"
    experiment_json = cell_dir / f"{cell.workload}.qec.json"
    cmd = compile_cell_argv(
        cell,
        quonc=quonc,
        target=target,
        report_path=report_path,
        experiment_json=experiment_json,
    )
    if dry_run:
        return report_path, experiment_json, cmd

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
            f"unsupported or failed grid cell for {cell.workload} "
            f"(backend={cell.na_backend}, placer={cell.na_placer}, "
            f"compact={cell.na_compact}): quonc exited {proc.returncode}\n"
            f"{detail}"
        )
    if not report_path.is_file() or not experiment_json.is_file():
        raise BenchmarkError(
            f"quonc did not write expected artifacts under {cell_dir}"
        )
    # Dual-emit writes <stem>.stim next to <stem>.qec.json (ADR-0018).
    stim = sinter_harness.sibling_stim_path(experiment_json)
    if not stim.is_file():
        raise BenchmarkError(
            f"quonc did not write sibling Stim circuit expected at {stim}"
        )
    return report_path, experiment_json, cmd


def evaluate_artifacts(
    cell: GridCell,
    *,
    report_path: Path,
    experiment_json: Path,
    shots: int,
    seed: int,
    sinter_csv_path: Path | None = None,
) -> tuple[BenchmarkRow, list[sinter_harness.ResultRow]]:
    """Join a ResourceReport JSON with a tiny Sinter sample; optionally write sinter CSV."""
    report = load_resource_report(report_path)
    sinter_rows = sinter_harness.run_experiments(
        [experiment_json],
        shots_list=[shots],
        seed=seed,
    )
    if len(sinter_rows) != 1:
        raise BenchmarkError(f"expected one Sinter row, got {len(sinter_rows)}")
    if sinter_csv_path is not None:
        sinter_csv_path.parent.mkdir(parents=True, exist_ok=True)
        with open(sinter_csv_path, "w", newline="") as f:
            sinter_harness.write_csv(f, sinter_rows)
    sample = sinter_harness.SampleResult(
        shots=sinter_rows[0].shots,
        logical_failures=sinter_rows[0].logical_failures,
    )
    row = join_cell(cell, report=report, sample=sample, seed=seed)
    return row, sinter_rows


def run_suite(
    *,
    mode: str,
    repo_root: Path,
    quonc: Path,
    target: Path,
    work_dir: Path,
    shots: int,
    seed: int,
    dry_run_compile: bool = False,
    include_flat: bool = False,
) -> tuple[list[BenchmarkRow], list[sinter_harness.ResultRow]]:
    cells = expand_grid(
        mode=mode, repo_root=repo_root, include_flat=include_flat
    )
    rows: list[BenchmarkRow] = []
    all_sinter: list[sinter_harness.ResultRow] = []
    if dry_run_compile:
        for cell in cells:
            _, _, argv = compile_cell(
                cell,
                quonc=quonc,
                target=target,
                work_dir=work_dir,
                dry_run=True,
            )
            # Validate flag shape without invoking quonc.
            if "--na-backend" not in argv or "--na-placer" not in argv:
                raise BenchmarkError(f"dry-run argv missing NA flags: {argv}")
            if "--emit-resource-report" not in argv or "--emit-qec-experiment" not in argv:
                raise BenchmarkError(f"dry-run argv missing dual-emit flags: {argv}")
            if not cell.na_compact and "--no-na-compact" not in argv:
                raise BenchmarkError(f"dry-run argv missing --no-na-compact: {argv}")
        return [], []

    for cell in cells:
        report_path, experiment_json, _ = compile_cell(
            cell, quonc=quonc, target=target, work_dir=work_dir
        )
        cell_sinter = report_path.parent / "sinter.csv"
        row, sinter_rows = evaluate_artifacts(
            cell,
            report_path=report_path,
            experiment_json=experiment_json,
            shots=shots,
            seed=seed,
            sinter_csv_path=cell_sinter,
        )
        rows.append(row)
        all_sinter.extend(sinter_rows)
    return rows, all_sinter


def default_shots_for_mode(mode: str) -> int:
    if mode == "smoke":
        return DEFAULT_SHOTS_SMOKE
    if mode == "axis":
        return DEFAULT_SHOTS_AXIS
    return DEFAULT_SHOTS_FULL


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        formatter_class=argparse.RawDescriptionHelpFormatter,
        description=(
            "QEC workload × compiler ablation grid with nested tiny Sinter "
            "samples (issue #254 / ADR-0023).\n"
            "Writes an optional join CSV (labeled analytic + sampled columns) "
            "plus a separate Sinter CSV; keeps ResourceReport / *.qec.json / "
            ".stim primaries (ADR-0020 amendment).\n"
            "Experiment class is QEC compiler-ablation — #111 / RAP Table I is "
            "physical-NA methodology only; no RAP number claims; no thresholds."
        ),
        epilog="""
Modes
  smoke   One tiny grid point for CI (repetition_d3_memory, zoned,
          routing-agnostic, compaction on). Default --shots 16.
  axis    Axis-coverage grid: each supported ablation axis once + one CX
          cell (zoned placers × compaction + CX). Proves full-grid flags.
          Default --shots 16. CI integration test when quonc is present.
  full    Local-only full grid: repetition + surface memory + surface CX
          × --na-placer × compaction on/off on zoned (default). Default
          --shots 32. Gated by axis coverage. Add --include-flat to
          enumerate flat cells (currently fail-closed on QEC + generic_rna_v0).

Notes
  Nested Sinter is schedule-agnostic (ADR-0024): noise from error_model
  proxies, not NA schedule counts. Analytic columns track ablations;
  sampled logical_failures may be invariant across placer/backend/compaction.
  --na-placer is zoned-path only; flat×placer (with --include-flat) are
  placer no-ops. Flat QEC currently fail-closes (Rydberg geometry).

Examples
  # CI smoke (also covered by python/test_quon_qec_benchmarks.py in just ci-rust)
  python python/quon_qec_benchmarks.py --mode smoke --csv /tmp/qec_smoke.csv

  # Axis coverage (proves full-grid axes)
  python python/quon_qec_benchmarks.py --mode axis --csv /tmp/qec_axis.csv

  # Full local ablation grid (not for CI)
  python python/quon_qec_benchmarks.py --mode full --csv /tmp/qec_full.csv

  # Validate argv/flags without compiling or sampling
  python python/quon_qec_benchmarks.py --mode full --dry-run-compile

  just qec-benchmarks-smoke   # local convenience (CI uses unittest via ci-rust)
  just qec-benchmarks-axis
  just qec-benchmarks-full

Methodology
  Field names align with the physical-NA #111 / RAP Table I reporting style
  (cycles, Rydberg stages, rearrangement time, transfers, …), but QEC rows are
  a distinct experiment class. See docs/neutral_atom/qec_benchmark_methodology.md.
""",
    )
    parser.add_argument(
        "--mode",
        choices=MODES,
        default="smoke",
        help="smoke = CI one-cell; axis = axis coverage; full = local grid "
        "(default: smoke)",
    )
    parser.add_argument(
        "--include-flat",
        action="store_true",
        help="include --na-backend=flat cells (currently fail-closed on QEC "
        "+ generic_rna_v0; flat×placer are placer no-ops)",
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
        help="directory for per-cell primary artifacts (default: temp under "
        "cwd; kept unless --cleanup)",
    )
    parser.add_argument(
        "--cleanup",
        action="store_true",
        help="delete auto-created --work-dir after the run (default: keep "
        "ResourceReport / *.qec.json / .stim / sinter.csv)",
    )
    parser.add_argument(
        "--dry-run-compile",
        action="store_true",
        help="validate per-cell quonc argv/flags without compiling or Sinter",
    )
    parser.add_argument(
        "--shots",
        type=int,
        default=None,
        help=f"Sinter shots per cell (default: {DEFAULT_SHOTS_SMOKE} smoke / "
        f"{DEFAULT_SHOTS_AXIS} axis / {DEFAULT_SHOTS_FULL} full)",
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
        help="write join CSV to this path (default: stdout)",
    )
    parser.add_argument(
        "--sinter-csv",
        type=str,
        default=None,
        help="write aggregated sampled-only Sinter CSV (default: sibling of "
        "--csv with .sinter.csv suffix, or <work-dir>/sinter_aggregated.csv)",
    )
    return parser


def _default_sinter_csv(csv_path: str | None, work_dir: Path) -> Path:
    if csv_path:
        p = Path(csv_path)
        return p.with_name(p.stem + ".sinter.csv")
    return work_dir / "sinter_aggregated.csv"


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_arg_parser()
    args = parser.parse_args(argv)
    repo_root = (args.repo_root or Path.cwd()).resolve()
    target = (args.target or (repo_root / DEFAULT_TARGET)).resolve()
    shots = args.shots
    if shots is None:
        shots = default_shots_for_mode(args.mode)
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
    auto_work_dir = False
    if work_dir is None:
        work_dir = Path(tempfile.mkdtemp(prefix="quon_qec_bench_", dir=str(Path.cwd())))
        auto_work_dir = True
    else:
        work_dir = work_dir.resolve()
        work_dir.mkdir(parents=True, exist_ok=True)

    try:
        rows, sinter_rows = run_suite(
            mode=args.mode,
            repo_root=repo_root,
            quonc=quonc,
            target=target,
            work_dir=work_dir,
            shots=shots,
            seed=args.seed,
            dry_run_compile=args.dry_run_compile,
            include_flat=args.include_flat,
        )
    except (BenchmarkError, sinter_harness.HarnessError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1
    finally:
        if auto_work_dir and args.cleanup:
            shutil.rmtree(work_dir, ignore_errors=True)

    if args.dry_run_compile:
        n = len(
            expand_grid(
                mode=args.mode,
                repo_root=repo_root,
                include_flat=args.include_flat,
            )
        )
        print(f"dry-run-compile ok: {n} cells validated under mode={args.mode}")
        if auto_work_dir and not args.cleanup:
            print(f"work-dir (empty dry-run dirs): {work_dir}")
        return 0

    if args.csv:
        with open(args.csv, "w", newline="") as f:
            write_csv(f, rows)
    else:
        write_csv(sys.stdout, rows)

    sinter_out = (
        Path(args.sinter_csv)
        if args.sinter_csv
        else _default_sinter_csv(args.csv, work_dir)
    )
    sinter_out.parent.mkdir(parents=True, exist_ok=True)
    with open(sinter_out, "w", newline="") as f:
        sinter_harness.write_csv(f, sinter_rows)

    if auto_work_dir and not args.cleanup:
        print(f"kept primary artifacts under {work_dir}", file=sys.stderr)
    print(f"wrote separate Sinter CSV: {sinter_out}", file=sys.stderr)
    return 0


if __name__ == "__main__":
    sys.exit(main())
