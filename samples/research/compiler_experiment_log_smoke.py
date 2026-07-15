#!/usr/bin/env python3
"""Smoke twin for `compiler_experiment_log.ipynb` (issue #190).

Regenerates the notebook's compiler-experiment-log table: compiles the
*linked, canonical* `test/verify/qaoa.qn` fixture (already owned by
`visualization/dense-swap-mismatch`, catalog `ci: smoke` — see
`samples/catalog.yaml`) under a small grid of `--target` / `--sabre-lookahead`
choices and reports each cell's `--metrics-json` depth / gate count, mirroring
the "targets/opts -> depth & 2Q counts" seed idea (#190).

Not a `test/verify/` correctness oracle (this circuit's compiled semantics
are already gated by `test/verify/qaoa.py` in `just ci-rust`); this script
answers a different question — "how much does target topology / SABRE
tuning move the headline metrics?" — and is deliberately not wired into
that same CI loop (see `samples/research/README.md`).

Run:
    QUONC=target/release/quonc python samples/research/compiler_experiment_log_smoke.py
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE = REPO_ROOT / "test" / "verify" / "qaoa.qn"
IBM_TARGET = REPO_ROOT / "targets" / "ibm" / "fake_manila_v2.json"


@dataclass(frozen=True)
class Cell:
    label: str
    target: Path | None
    sabre_lookahead: int | None  # None = quonc default (20)


CELLS: tuple[Cell, ...] = (
    Cell("unconstrained (no --target)", target=None, sabre_lookahead=None),
    Cell("fake_manila_v2, --sabre-lookahead 1", target=IBM_TARGET, sabre_lookahead=1),
    Cell("fake_manila_v2, default lookahead (20)", target=IBM_TARGET, sabre_lookahead=None),
)


class SmokeError(Exception):
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
    raise SmokeError(
        "quonc not found. Build it and point at the binary:\n"
        "  cargo build --release -p quonc\n"
        "  export QUONC=$PWD/target/release/quonc"
    )


def run_cell(quonc: Path, cell: Cell) -> dict:
    args = [str(quonc)]
    if cell.target is not None:
        args += ["--target", str(cell.target)]
    if cell.sabre_lookahead is not None:
        args += ["--sabre-lookahead", str(cell.sabre_lookahead)]
    args += ["--metrics-json", "-", "-q", str(SOURCE)]
    proc = subprocess.run(args, capture_output=True, text=True, cwd=REPO_ROOT)
    if proc.returncode != 0:
        raise SmokeError(f"quonc failed for cell {cell.label!r}: {proc.stderr}")
    doc = json.loads(proc.stdout)
    return doc["metrics"]


def main() -> int:
    quonc = resolve_quonc()
    rows: list[tuple[Cell, dict]] = []
    for cell in CELLS:
        metrics = run_cell(quonc, cell)
        rows.append((cell, metrics))
        print(
            f"{cell.label:45s} depth={metrics['depth']:>4d} "
            f"gate_count={metrics['gate_count']:>4d}"
        )

    unconstrained = rows[0][1]
    lookahead_1 = rows[1][1]
    lookahead_default = rows[2][1]

    # Headline claim 1: routing onto a constrained line topology strictly
    # costs depth/gates relative to the unconstrained (no-target) bound —
    # topology constraints are real, not a rounding artifact.
    if not (unconstrained["depth"] < lookahead_1["depth"]):
        print(
            "FAIL: expected unconstrained depth "
            f"({unconstrained['depth']}) < routed depth ({lookahead_1['depth']})"
        )
        return 1

    # Headline claim 2: widening SABRE's lookahead window from 1 to the
    # default (20) never makes depth/gate count *worse* on this fixture.
    if lookahead_default["depth"] > lookahead_1["depth"]:
        print(
            "FAIL: default lookahead depth "
            f"({lookahead_default['depth']}) > lookahead=1 depth ({lookahead_1['depth']})"
        )
        return 1

    # Note (mirrors samples/visualization/refresh_goldens.sh's comment):
    # `swap_count` only counts a literal SWAP op, already decomposed to
    # CNOTs by the time metrics run, so it stays 0 here even though routing
    # clearly inserted a SWAP network (visible in gate_count growth) — do
    # not read swap_count==0 as "no routing happened."
    for cell, metrics in rows:
        if metrics["swap_count"] != 0:
            print(
                f"FAIL: unexpected nonzero swap_count for {cell.label!r} "
                "(the notebook's premise is that this metric stays 0 post-decomposition)"
            )
            return 1

    print(
        "\nPASS: topology constraint and SABRE lookahead both move depth/gate_count "
        "as the notebook's log claims"
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SmokeError as exc:
        print(f"FAIL: {exc}")
        sys.exit(1)
