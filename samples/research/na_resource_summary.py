#!/usr/bin/env python3
"""Neutral-atom resource summary demo (issue #196).

Compiles a neutral-atom program with `quonc`, then renders the
`--emit-resource-report` and `--emit-na-schedule` envelopes through the
`python/quon_viz.py` presentation helpers (`summarize_na_report` +
`summarize_na_schedule`) instead of dumping raw JSON.

This is the runnable demo companion to `na_resource_study.ipynb`: where that
notebook *derives* the headline numbers and its `_smoke.py` twin *asserts*
them against goldens, this script just *shows* them — a one-glance readout of
what a NA compile produced. It is a demo, not a CI oracle: it exits 0 as long
as `quonc` runs and the helpers format something printable.

Run:
  QUONC=target/release/quonc python samples/research/na_resource_summary.py
  python samples/research/na_resource_summary.py test/na/qaoa_graph.qn
"""

from __future__ import annotations

import argparse
import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(REPO_ROOT / "python"))

import quon_viz  # noqa: E402

NA_TARGET = REPO_ROOT / "targets" / "neutral_atom" / "generic_rna_v0.json"
DEFAULT_SOURCES = [
    REPO_ROOT / "test" / "na" / "bell.qn",
    REPO_ROOT / "test" / "na" / "qaoa_graph.qn",
]


class DemoError(Exception):
    pass


def resolve_quonc() -> Path:
    env = os.environ.get("QUONC")
    candidates = [Path(env)] if env else []
    candidates += [
        REPO_ROOT / "target" / "release" / "quonc",
        REPO_ROOT / "target" / "debug" / "quonc",
    ]
    which = shutil.which("quonc")
    if which:
        candidates.append(Path(which))
    for c in candidates:
        if c.is_file() and os.access(c, os.X_OK):
            return c.resolve()
    raise DemoError(
        "quonc not found. Build it and point at the binary:\n"
        "  cargo build --release -p quonc\n"
        "  export QUONC=$PWD/target/release/quonc"
    )


def run_quonc(quonc: Path, source: Path, emit_flag: str) -> str:
    """Run `quonc` with an emit-to-stdout flag and return the captured output."""
    args = [
        str(quonc),
        "--target",
        str(NA_TARGET),
        "--na-backend",
        "zoned",
        emit_flag,
        "-",  # emit to stdout
        "-q",
        str(source),
    ]
    proc = subprocess.run(args, capture_output=True, text=True, cwd=REPO_ROOT)
    if proc.returncode != 0:
        raise DemoError(f"quonc failed for {source.name} ({emit_flag}):\n{proc.stderr}")
    return proc.stdout


def summarize_source(quonc: Path, source: Path, max_layers: int | None) -> None:
    src_abs = source.resolve()
    print(f"\n{'=' * 72}")
    print(f"# {src_abs.relative_to(REPO_ROOT)}")
    print("=" * 72)

    report_json = run_quonc(quonc, source, "--emit-resource-report")
    report = json.loads(report_json)
    print()
    print(quon_viz.summarize_na_report(report, title=f"Resource report — {source.stem}"))

    schedule_json = run_quonc(quonc, source, "--emit-na-schedule")
    schedule = json.loads(schedule_json)
    print()
    print(
        quon_viz.summarize_na_schedule(
            schedule, title=f"Schedule timeline — {source.stem}", max_layers=max_layers
        )
    )


def main(argv: list[str] | None = None) -> int:
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument(
        "sources",
        nargs="*",
        type=Path,
        help="NA .qn sources to summarize (default: test/na/bell.qn + qaoa_graph.qn)",
    )
    p.add_argument(
        "--max-layers",
        type=int,
        default=10,
        help="Cap the per-cycle schedule listing (default: 10; 0 = headline only)",
    )
    args = p.parse_args(argv)

    sources = args.sources or DEFAULT_SOURCES
    for src in sources:
        if not src.is_file():
            print(f"quon_viz demo: source not found: {src}", file=sys.stderr)
            return 1

    try:
        quonc = resolve_quonc()
    except DemoError as exc:
        print(str(exc), file=sys.stderr)
        return 1

    max_layers = args.max_layers or None
    for src in sources:
        try:
            summarize_source(quonc, src, max_layers)
        except (DemoError, json.JSONDecodeError, ValueError) as exc:
            print(f"quon_viz demo: failed to summarize {src}: {exc}", file=sys.stderr)
            return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
