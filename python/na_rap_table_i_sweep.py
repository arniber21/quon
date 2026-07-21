#!/usr/bin/env python3
from __future__ import annotations

"""
RAP Table I full sweep harness (issue #306, follow-up to #111 / #297 / #307).

Runs every checked-in RAP Table I benchmark row (currently: `ising` at
n=42 and n=98 — see "Scope: #304" below) through `quonc` under both
`--na-placer` modes (`routing-agnostic`, `routing-aware`) against the pinned
`targets/neutral_atom/rap_table_i.json` target, and emits **one CSV** joining
the analytic resource-report columns ([RAP] Table I's own metric names) with
the routing-aware search diagnostics `--emit-na-stats` exposes (issue #307).

Scope: #304 (QASM ingestion)
-----------------------------
[RAP] Table I (Stade, Lin, Cong, Wille, ICCAD 2025, arXiv:2505.22715) reports
several QASMBench/MQT Bench benchmark circuits at multiple qubit counts (see
docs/neutral_atom/literature_notes.md's "[RAP]" section). Quon has no QASM
*ingestion* path (only OpenQASM *emission* for fixed targets) — issue #304
tracks that gap and is **not implemented**. This script therefore sweeps only
the `ising` rows, which are hand-authored Quon fixtures
(`test/na/ising_n42.qn`, `test/na/ising_n98.qn`) rather than vendored QASM.
Do **not** add non-`ising` Table I rows here without #304 landing first — see
docs/neutral_atom/rap_table_i_methodology.md's "#304 scope gap" section for
the full rationale.

CI status
---------
Only `ising_n42`'s *structural* pre-flight (gate/layer counts) is part of the
default `cargo test --workspace` gate (run inside `just ci-rust`). Its slow
metrics dump (`just rap-table-i`, release + `--include-ignored`) is a
local-only convenience recipe, not invoked by any CI job — see
docs/neutral_atom/rap_table_i_methodology.md's "Runtime / CI wiring" section
(a stale claim there, that this recipe was CI-wired, was corrected while
building this script). This script (`just na-rap-sweep`) follows the same
local-only precedent, for the same reason (wall time): `ising_n98`'s
routing-aware search alone is a double-digit-second `--release` run (see
module-level timing note in that methodology doc's "n = 98" section), several
times slower than n42's.

qmap CSV comparability
-----------------------
qmap's `eval/na/zoned/eval_ids_relaxed_routing.py` (munich-quantum-toolkit/qmap)
emits a flat per-cell CSV with columns `circuit_name, num_qubits, setting,
status, two_qubit_gates, scheduling_time, reuse_analysis_time, placement_time,
routing_time, code_generation_time, total_time, two_qubit_gate_layer,
max_two_qubit_gates, rearrangement_duration` (fetched from that file's
`print_header()` while implementing this script — not independently verified
against a live qmap run, since installing/running qmap was out of scope here;
see "Optional: direct qmap comparison" below). This script's CSV_COLUMNS below
reuse quon's own `ResourceReport`/`NaStats` field names (the existing
convention in `python/quon_qec_benchmarks.py` and
docs/neutral_atom/rap_table_i_methodology.md's "Metric mapping" table) rather
than renaming to qmap's exact headers, but a `circuit_name` column is included
verbatim (qmap's own primary-key column name) for easy joining, and the
mapping from quon's names to qmap's is:

    quon CSV column          qmap CSV column        notes
    ------------------------ ----------------------- --------------------------------
    circuit_name              circuit_name            e.g. "ising_n42" (same shape)
    n                         num_qubits               qubit count
    placer                    setting                  DIFFERENT VOCABULARY: quon uses
                                                         routing-agnostic/routing-aware;
                                                         qmap uses ids/astar/relaxed —
                                                         not a literal value join
    entangle2_count            two_qubit_gates          gate count
    rydberg_stages             two_qubit_gate_layer     entangling-layer count
    rearrangement_time_us      rearrangement_duration   quon: move-only sqrt-law time
                                                         (see methodology doc's "Timing
                                                         model"); qmap: jerk-limited
                                                         model (documented divergence,
                                                         do not treat as directly
                                                         comparable without conversion)
    wall_time_us                total_time               quon: whole-process wall time
                                                         (subprocess start to exit);
                                                         qmap: compiler-internal timer
                                                         only — NOT apples-to-apples
    (no quon analog)          status                   qmap distinguishes
                                                         ok/timeout/memout/error; this
                                                         script hard-fails the whole
                                                         sweep on any non-zero quonc
                                                         exit instead of recording a
                                                         per-row status
    (no quon analog)          scheduling_time,         qmap's internal phase-by-phase
                               reuse_analysis_time,      breakdown has no quon
                               placement_time,           equivalent; quon's closest
                               routing_time,             analog is --emit-na-stats'
                               code_generation_time      stage_timings_us, included
                                                         here as separate columns
                                                         instead (finer-grained, not
                                                         phase-name-compatible)
    transfer_time_us,          (no qmap analog in the   quon-only: see methodology
     rearrangement_steps,       fetched column list)     doc's "Timing model" for why
     aware_search_*                                     rearrangement_steps and the
                                                         aware-search diagnostics are
                                                         kept as quon-native columns
                                                         (they are the paper's own
                                                         Table I columns / #111 review
                                                         instrumentation, not qmap
                                                         output fields)

A side-by-side comparison notebook should rename/join on this table rather
than assume identical headers.

Optional: direct qmap comparison
---------------------------------
Not implemented. Installing and running mqt-qmap (a full Python package with
its own native build) in this sandboxed environment was judged nontrivial
relative to its value for this issue — left as a follow-up. This script's
qmap-recognizable column naming (above) is the mitigation in the meantime.

Run:
    QUONC=target/release/quonc python python/na_rap_table_i_sweep.py \\
        --csv /tmp/na_rap_table_i_sweep.csv

    just na-rap-sweep
"""

import argparse
import csv
import json
import os
import re
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Sequence, TextIO

EXPERIMENT_CLASS = "rap_table_i_sweep"
METHODOLOGY_ANCHOR = "issue_111_rap_table_i"
DEFAULT_TARGET = "targets/neutral_atom/rap_table_i.json"

PLACERS: tuple[str, ...] = ("routing-agnostic", "routing-aware")

# Checked-in RAP Table I `ising` rows only — see module docstring's
# "Scope: #304" section for why the rest of [RAP] Table I is not here.
BENCHMARKS: tuple[tuple[str, int, str], ...] = (
    ("ising", 42, "test/na/ising_n42.qn"),
    ("ising", 98, "test/na/ising_n98.qn"),
)

# [RAP] Table I published rows (Sec. VI-B), for the printed comparison only —
# NOT asserted against (this script never fails on numeric drift; see
# docs/neutral_atom/rap_table_i_methodology.md's Phase 1/2 split, which this
# sweep does not change the enforcement status of).
PUBLISHED_STEPS: dict[tuple[str, int, str], int] = {
    ("ising", 42, "routing-agnostic"): 22,
    ("ising", 42, "routing-aware"): 9,
    ("ising", 98, "routing-agnostic"): 23,
    ("ising", 98, "routing-aware"): 12,
}

CSV_BANNER = (
    f"# experiment_class: {EXPERIMENT_CLASS}\n"
    "# RAP Table I full sweep (issue #306): both placer modes over every "
    "checked-in ising row against the pinned targets/neutral_atom/rap_table_i.json "
    "target (issue #111).\n"
    "# evidence_kind: analytic (ADR-0020) — from --emit-resource-report + "
    "--emit-na-stats (#307); no sampled/Sinter evidence in this CSV.\n"
    "# Scope: #304 (QASM ingestion) is NOT implemented, so only the hand-authored "
    "`ising` rows are swept here — the rest of [RAP] Table I's QASMBench/MQT "
    "Bench benchmarks are out of scope until #304 lands. See this script's "
    "module docstring and docs/neutral_atom/rap_table_i_methodology.md.\n"
    "# CI status: only ising_n42's fast structural pre-flight runs in CI (part of "
    "`cargo test --workspace` inside `just ci-rust`). The release-mode metric dump "
    "(`just rap-table-i`) and this full sweep (`just na-rap-sweep`) are BOTH "
    "local/nightly-only — neither is invoked by any CI job; see "
    "docs/neutral_atom/rap_table_i_methodology.md's 'Runtime / CI wiring' section.\n"
    "# Timing model: rearrangement_time_us is move-only sqrt-law time (does NOT "
    "include transfer_time_us) — see the methodology doc's 'Timing model' "
    "section for why these stay separate columns rather than being summed.\n"
    "# qmap comparability: column names reuse quon's own ResourceReport/NaStats "
    "fields, not qmap's exact CSV headers; see this script's module docstring "
    "for the documented quon<->qmap column mapping (not independently verified "
    "against a live qmap run).\n"
    "# Not a threshold claim; the published_steps column is [RAP]'s own Table I "
    "number for that (benchmark, n, placer), printed for comparison only, never "
    "asserted against.\n"
)

CSV_COLUMNS = [
    "experiment_class",
    "methodology_anchor",
    "benchmark",
    "circuit_name",
    "n",
    "target_id",
    "placer",
    "rearrangement_steps",
    "published_steps",
    "rearrangement_time_us",
    "transfer_time_us",
    "rydberg_stages",
    "entangle2_count",
    "aware_search_completed_layers",
    "aware_search_fell_back_layers",
    "aware_search_node_expansions",
    "aware_search_node_budget",
    "wall_time_us",
]


class SweepError(Exception):
    """Harness failure with an actionable message."""


@dataclass(frozen=True)
class SweepCell:
    benchmark: str
    n: int
    source: Path
    placer: str


@dataclass(frozen=True)
class SweepRow:
    benchmark: str
    n: int
    circuit_name: str
    target_id: str
    placer: str
    rearrangement_steps: int
    rearrangement_time_us: int
    transfer_time_us: int
    rydberg_stages: int
    entangle2_count: int
    aware_search_completed_layers: int | None
    aware_search_fell_back_layers: int | None
    aware_search_node_expansions: int | None
    aware_search_node_budget: int | None
    wall_time_us: int


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


# `--emit-na-stats -` writes pretty-printed JSON to stderr via `eprintln!`
# whenever stdout is already claimed by an earlier `-` emit (here,
# --emit-resource-report). Native-gate-decomposition warnings
# (`quon_na: native-gate-decomp: ...`) also land on stderr, interleaved
# *before* the stats JSON (they're logged during compilation, stats is
# printed at the very end) — so the stats block is always the last
# contiguous `{...}` in the stream. serde_json::to_string_pretty always
# opens/closes on their own lines, so anchoring on a standalone "{" line is
# robust without a full brace-matching scanner.
_STATS_JSON_START = re.compile(r"^\{\s*$", re.MULTILINE)


def extract_na_stats_json(stderr_text: str) -> dict[str, Any] | None:
    starts = [m.start() for m in _STATS_JSON_START.finditer(stderr_text)]
    if not starts:
        return None
    candidate = stderr_text[starts[-1] :]
    try:
        return json.loads(candidate)
    except json.JSONDecodeError:
        return None


def build_argv(
    *, quonc: Path, source: Path, target: Path, placer: str, verify_na: bool
) -> list[str]:
    cmd = [
        str(quonc),
        str(source),
        "--target",
        str(target),
        "--na-backend",
        "zoned",
        "--na-placer",
        placer,
        "--emit-resource-report",
        "-",
        "--emit-na-stats",
        "-",
        "--quiet",
    ]
    if verify_na:
        cmd.append("--verify-na")
    return cmd


def run_cell(
    cell: SweepCell,
    *,
    quonc: Path,
    target: Path,
    target_id: str,
    verify_na: bool,
    dry_run: bool = False,
) -> tuple[SweepRow, list[str]]:
    if not cell.source.is_file():
        raise SweepError(f"benchmark source not found: {cell.source}")
    argv = build_argv(
        quonc=quonc, source=cell.source, target=target, placer=cell.placer, verify_na=verify_na
    )
    if dry_run:
        placeholder = SweepRow(
            benchmark=cell.benchmark,
            n=cell.n,
            circuit_name=f"{cell.benchmark}_n{cell.n}",
            target_id=target_id,
            placer=cell.placer,
            rearrangement_steps=0,
            rearrangement_time_us=0,
            transfer_time_us=0,
            rydberg_stages=0,
            entangle2_count=0,
            aware_search_completed_layers=None,
            aware_search_fell_back_layers=None,
            aware_search_node_expansions=None,
            aware_search_node_budget=None,
            wall_time_us=0,
        )
        return placeholder, argv

    start = time.perf_counter()
    try:
        proc = subprocess.run(argv, check=False, capture_output=True, text=True)
    except OSError as exc:
        raise SweepError(f"failed to invoke quonc: {exc}") from exc
    wall_time_us = int((time.perf_counter() - start) * 1_000_000)

    if proc.returncode != 0:
        detail = (proc.stderr or proc.stdout or "").strip()
        raise SweepError(
            f"quonc failed for {cell.benchmark}_n{cell.n} (placer={cell.placer}): "
            f"exit {proc.returncode}\n{detail}"
        )

    try:
        report = json.loads(proc.stdout)
    except json.JSONDecodeError as exc:
        raise SweepError(
            f"failed to parse --emit-resource-report JSON for {cell.benchmark}_n{cell.n} "
            f"(placer={cell.placer}): {exc}\nstdout: {proc.stdout[:500]}"
        ) from exc

    stats = extract_na_stats_json(proc.stderr)
    search = stats.get("search", {}) if stats else {}

    def _report_int(key: str) -> int:
        if key not in report:
            raise SweepError(
                f"resource report missing required field {key!r} for "
                f"{cell.benchmark}_n{cell.n} (placer={cell.placer})"
            )
        return int(report[key])

    row = SweepRow(
        benchmark=cell.benchmark,
        n=cell.n,
        circuit_name=f"{cell.benchmark}_n{cell.n}",
        target_id=target_id,
        placer=cell.placer,
        rearrangement_steps=_report_int("rearrangement_steps"),
        rearrangement_time_us=_report_int("rearrangement_time_us"),
        transfer_time_us=_report_int("transfer_time_us"),
        rydberg_stages=_report_int("rydberg_stages"),
        entangle2_count=_report_int("entangle2_count"),
        aware_search_completed_layers=report.get("aware_search_completed_layers"),
        aware_search_fell_back_layers=report.get("aware_search_fell_back_layers"),
        aware_search_node_expansions=search.get("aware_search_node_expansions"),
        aware_search_node_budget=search.get("aware_search_node_budget"),
        wall_time_us=wall_time_us,
    )
    return row, argv


def expand_cells(
    *, repo_root: Path, benchmarks: Sequence[tuple[str, int, str]] = BENCHMARKS,
    placers: Sequence[str] = PLACERS,
) -> list[SweepCell]:
    cells = []
    for benchmark, n, rel_source in benchmarks:
        for placer in placers:
            cells.append(
                SweepCell(
                    benchmark=benchmark, n=n, source=repo_root / rel_source, placer=placer
                )
            )
    return cells


def load_target_id(target: Path) -> str:
    try:
        doc = json.loads(target.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise SweepError(f"failed to read target {target}: {exc}") from exc
    target_id = doc.get("id")
    if not isinstance(target_id, str):
        raise SweepError(f"{target}: missing string 'id' field")
    return target_id


def write_csv(out: TextIO, rows: Sequence[SweepRow]) -> None:
    out.write(CSV_BANNER)
    writer = csv.DictWriter(out, fieldnames=CSV_COLUMNS)
    writer.writeheader()
    for row in rows:
        published = PUBLISHED_STEPS.get((row.benchmark, row.n, row.placer))
        writer.writerow(
            {
                "experiment_class": EXPERIMENT_CLASS,
                "methodology_anchor": METHODOLOGY_ANCHOR,
                "benchmark": row.benchmark,
                "circuit_name": row.circuit_name,
                "n": row.n,
                "target_id": row.target_id,
                "placer": row.placer,
                "rearrangement_steps": row.rearrangement_steps,
                "published_steps": "" if published is None else published,
                "rearrangement_time_us": row.rearrangement_time_us,
                "transfer_time_us": row.transfer_time_us,
                "rydberg_stages": row.rydberg_stages,
                "entangle2_count": row.entangle2_count,
                "aware_search_completed_layers": (
                    "" if row.aware_search_completed_layers is None
                    else row.aware_search_completed_layers
                ),
                "aware_search_fell_back_layers": (
                    "" if row.aware_search_fell_back_layers is None
                    else row.aware_search_fell_back_layers
                ),
                "aware_search_node_expansions": (
                    "" if row.aware_search_node_expansions is None
                    else row.aware_search_node_expansions
                ),
                "aware_search_node_budget": (
                    "" if row.aware_search_node_budget is None
                    else row.aware_search_node_budget
                ),
                "wall_time_us": row.wall_time_us,
            }
        )


def print_summary(rows: Sequence[SweepRow]) -> None:
    print(
        f"{'circuit':<12} {'placer':<18} {'steps':>6} {'(paper)':>8} "
        f"{'time_us':>9} {'xfer_us':>9} {'search':>24} {'wall_ms':>9}"
    )
    for row in rows:
        published = PUBLISHED_STEPS.get((row.benchmark, row.n, row.placer))
        if row.placer == "routing-aware":
            search = (
                f"{row.aware_search_completed_layers} ok / "
                f"{row.aware_search_fell_back_layers} fell back"
            )
        else:
            search = "n/a"
        print(
            f"{row.circuit_name:<12} {row.placer:<18} {row.rearrangement_steps:>6} "
            f"{'' if published is None else published:>8} "
            f"{row.rearrangement_time_us:>9} {row.transfer_time_us:>9} {search:>24} "
            f"{row.wall_time_us / 1000:>9.1f}"
        )


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        formatter_class=argparse.RawDescriptionHelpFormatter,
        description=(
            "RAP Table I full sweep (issue #306): both --na-placer modes over "
            "every checked-in ising row, emitting one qmap-comparable CSV. "
            "See this file's module docstring for the #304 scope-gap rationale."
        ),
        epilog="""
Examples
  # Full sweep (both ising rows x both placers), CSV to stdout
  python python/na_rap_table_i_sweep.py

  # Write CSV to a file
  python python/na_rap_table_i_sweep.py --csv /tmp/na_rap_table_i_sweep.csv

  # Validate argv shape only, no quonc invocation (fast CI-adjacent smoke)
  python python/na_rap_table_i_sweep.py --dry-run

  just na-rap-sweep   # local convenience recipe (not part of ci-rust)

Notes
  ising_n98's routing-aware cell is the slow one (double-digit seconds in
  --release; see docs/neutral_atom/rap_table_i_methodology.md). This script
  has no timeout; if you need to bound wall time, run --benchmarks ising_n42
  only.
""",
    )
    parser.add_argument(
        "--repo-root", type=Path, default=None, help="repository root (default: cwd)"
    )
    parser.add_argument(
        "--quonc", type=str, default=None,
        help="path to quonc (default: QUONC env, then target/release/quonc)",
    )
    parser.add_argument(
        "--target", type=Path, default=None,
        help=f"NA target JSON (default: {{repo}}/{DEFAULT_TARGET}, the pinned #111 target)",
    )
    parser.add_argument(
        "--benchmarks", type=str, default=None,
        help="comma-separated circuit_names to include, e.g. 'ising_n42' "
        "(default: all checked-in rows)",
    )
    parser.add_argument(
        "--placers", type=str, default=",".join(PLACERS),
        help=f"comma-separated placer modes (default: {','.join(PLACERS)})",
    )
    parser.add_argument(
        "--no-verify-na", action="store_true",
        help="skip --verify-na schedule-legality verification per cell (default: verify)",
    )
    parser.add_argument(
        "--dry-run", action="store_true",
        help="validate cells/argv without invoking quonc",
    )
    parser.add_argument(
        "--csv", type=str, default=None, help="write CSV to this path (default: stdout)"
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_arg_parser()
    args = parser.parse_args(argv)
    repo_root = (args.repo_root or Path.cwd()).resolve()
    target = (args.target or (repo_root / DEFAULT_TARGET)).resolve()

    quonc = resolve_quonc(repo_root, args.quonc)
    if quonc is None:
        print(
            "error: quonc not found. Build with `cargo build --release -p quonc` "
            "or set QUONC / pass --quonc.",
            file=sys.stderr,
        )
        return 1
    if not target.is_file():
        print(f"error: NA target not found: {target}", file=sys.stderr)
        return 1

    benchmarks = BENCHMARKS
    if args.benchmarks:
        wanted = {b.strip() for b in args.benchmarks.split(",") if b.strip()}
        benchmarks = tuple(
            (b, n, src) for (b, n, src) in BENCHMARKS if f"{b}_n{n}" in wanted
        )
        if not benchmarks:
            print(
                f"error: --benchmarks matched nothing; available: "
                f"{[f'{b}_n{n}' for b, n, _ in BENCHMARKS]}",
                file=sys.stderr,
            )
            return 2

    placers = tuple(p.strip() for p in args.placers.split(",") if p.strip())
    for p in placers:
        if p not in PLACERS:
            print(f"error: unknown placer {p!r}; expected one of {PLACERS}", file=sys.stderr)
            return 2

    try:
        target_id = load_target_id(target)
        cells = expand_cells(repo_root=repo_root, benchmarks=benchmarks, placers=placers)
        rows: list[SweepRow] = []
        for cell in cells:
            row, cell_argv = run_cell(
                cell,
                quonc=quonc,
                target=target,
                target_id=target_id,
                verify_na=not args.no_verify_na,
                dry_run=args.dry_run,
            )
            if args.dry_run:
                if "--na-placer" not in cell_argv or "--emit-resource-report" not in cell_argv:
                    raise SweepError(f"dry-run argv missing expected flags: {cell_argv}")
                continue
            rows.append(row)
            print(
                f"==> {row.circuit_name} {row.placer}: "
                f"steps={row.rearrangement_steps} time_us={row.rearrangement_time_us} "
                f"wall_ms={row.wall_time_us / 1000:.1f}",
                file=sys.stderr,
            )
    except SweepError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if args.dry_run:
        print(f"dry-run ok: {len(cells)} cells validated")
        return 0

    if args.csv:
        with open(args.csv, "w", newline="") as f:
            write_csv(f, rows)
        print(f"wrote {args.csv} ({len(rows)} rows)", file=sys.stderr)
    else:
        write_csv(sys.stdout, rows)

    print(file=sys.stderr)
    print_summary(rows)
    return 0


if __name__ == "__main__":
    sys.exit(main())
