#!/usr/bin/env python3
"""Smoke twin for `na_resource_study.ipynb` (issue #190).

Regenerates the notebook's neutral-atom resource-report comparison across
`--na-placer` modes for two *linked, canonical* fixtures already owned by
`test/na/` and registered in `samples/catalog.yaml` (`neutral-atom/bell-pair`,
`neutral-atom/qaoa-maxcut`): `bell.qn` (2 qubits, no placement freedom) and
`qaoa_graph.qn` (4-qubit dense interaction graph). This is the "NA resource
study (reports from #192 artifacts)" seed idea (#190).

Two of the four cells below (`bell.qn` / `qaoa_graph.qn`, both
`routing-agnostic`, the quonc default) reproduce numbers already checked in
as `#189` goldens at
`samples/visualization/goldens/na_schedule_metrics/*.resource_report.json`;
this script cross-checks against those files rather than re-deriving them
from scratch, per the "link, don't fork" rule. The other two cells
(`routing-aware`) are new — no golden is checked in for them, so this script
compiles them fresh every run (consistent with `samples/research/` being
regenerable narrative, not a second copy of `#189`'s static showcase).

Not a `test/na/` correctness oracle: schedule *legality* for both fixtures is
already gated by `--verify-na` in `just ci-rust`'s corpus sweep and by
`neutral-atom/*`'s `ci: smoke` catalog rows. This script instead answers the
notebook's question — "does routing-awareness change the *analytic resource
report*, and by how much?" — and is wired into `just ci-rust`'s python
verify loop as its own regression gate on this notebook's headline numbers
(see `samples/research/README.md`).

Run:
    QUONC=target/release/quonc python samples/research/na_resource_study_smoke.py
"""

from __future__ import annotations

import json
import os
import shutil
import subprocess
import sys
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
NA_TARGET = REPO_ROOT / "targets" / "neutral_atom" / "generic_rna_v0.json"
BELL_SOURCE = REPO_ROOT / "test" / "na" / "bell.qn"
QAOA_SOURCE = REPO_ROOT / "test" / "na" / "qaoa_graph.qn"

GOLDEN_DIR = REPO_ROOT / "samples" / "visualization" / "goldens" / "na_schedule_metrics"
BELL_GOLDEN = GOLDEN_DIR / "bell_zoned.resource_report.json"
QAOA_GOLDEN = GOLDEN_DIR / "qaoa_graph_zoned.resource_report.json"

PLACERS = ("routing-agnostic", "routing-aware")

# Exact headline numbers the notebook's "Resource table" quotes for the
# `routing-aware` `qaoa_graph.qn` cell (37/9/24) — no #189 golden covers
# `routing-aware` (only `routing-agnostic` was checked in), so this is the
# one cell that needs its own pinned expectation rather than a golden
# cross-check. This is analytic/deterministic `--emit-resource-report`
# output, not a sampled statistic, so an exact match is the right bar; if a
# placer-heuristic change legitimately moves these numbers, update both this
# dict and na_resource_study.ipynb's "Resource table" together.
QAOA_GRAPH_ROUTING_AWARE_EXPECTED = {
    "estimated_cycles": 37,
    "rearrangement_steps": 9,
    "trap_transfers": 24,
    "bottleneck": "rearrangement",
}

# Fields that must stay comparable across placer modes for these fixtures
# (all analytic, ADR-0020 `evidence_kind: analytic`).
COMPARE_FIELDS = (
    "estimated_cycles",
    "rearrangement_steps",
    "rearrangement_time_us",
    "trap_transfers",
    "physical_atoms",
    "logical_qubits",
    "bottleneck",
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


def resource_report(quonc: Path, source: Path, placer: str) -> dict:
    args = [
        str(quonc),
        "--target",
        str(NA_TARGET),
        "--na-backend",
        "zoned",
        "--na-placer",
        placer,
        "--verify-na",
        "--emit-resource-report",
        "-",
        "-q",
        str(source),
    ]
    proc = subprocess.run(args, capture_output=True, text=True, cwd=REPO_ROOT)
    if proc.returncode != 0:
        raise SmokeError(f"quonc failed for {source.name} ({placer}): {proc.stderr}")
    return json.loads(proc.stdout)


def load_golden(path: Path) -> dict:
    if not path.is_file():
        raise SmokeError(f"missing #189 golden artifact: {path}")
    return json.loads(path.read_text())


def main() -> int:
    quonc = resolve_quonc()

    reports: dict[tuple[str, str], dict] = {}
    for source in (BELL_SOURCE, QAOA_SOURCE):
        for placer in PLACERS:
            reports[(source.name, placer)] = resource_report(quonc, source, placer)
            r = reports[(source.name, placer)]
            print(
                f"{source.name:16s} {placer:16s} "
                f"cycles={r['estimated_cycles']:>3d} "
                f"rearrange_steps={r['rearrangement_steps']:>2d} "
                f"transfers={r['trap_transfers']:>3d} "
                f"bottleneck={r['bottleneck']}"
            )

    # Cross-check the routing-agnostic (default) cells against the #189
    # checked-in goldens, rather than re-deriving them from scratch.
    bell_golden = load_golden(BELL_GOLDEN)
    qaoa_golden = load_golden(QAOA_GOLDEN)
    bell_agnostic = reports[("bell.qn", "routing-agnostic")]
    qaoa_agnostic = reports[("qaoa_graph.qn", "routing-agnostic")]

    for label, live, golden in (
        ("bell.qn", bell_agnostic, bell_golden),
        ("qaoa_graph.qn", qaoa_agnostic, qaoa_golden),
    ):
        for field in COMPARE_FIELDS:
            if live[field] != golden[field]:
                print(
                    f"FAIL: {label} routing-agnostic {field} = {live[field]!r} "
                    f"drifted from #189 golden {golden[field]!r} ({golden})"
                )
                return 1
    print("\nPASS: routing-agnostic cells match the checked-in #189 goldens exactly")

    # Headline claim: bell.qn (2 qubits, one edge) has no placement freedom,
    # so every analytic field is identical across placer modes.
    bell_aware = reports[("bell.qn", "routing-aware")]
    for field in COMPARE_FIELDS:
        if bell_agnostic[field] != bell_aware[field]:
            print(
                f"FAIL: expected bell.qn to be placer-invariant on {field}, "
                f"got {bell_agnostic[field]!r} (routing-agnostic) vs "
                f"{bell_aware[field]!r} (routing-aware)"
            )
            return 1
    print("PASS: bell.qn's resource report is identical across placer modes")

    # Headline claim: for this dense, already-low-degree qaoa_graph.qn
    # (Delta~=3), routing-awareness does NOT reduce estimated_cycles — it
    # costs at least as many cycles as routing-agnostic. This is the
    # counterintuitive result the notebook narrates.
    qaoa_aware = reports[("qaoa_graph.qn", "routing-aware")]
    if qaoa_aware["estimated_cycles"] < qaoa_agnostic["estimated_cycles"]:
        print(
            "FAIL: expected routing-aware estimated_cycles >= routing-agnostic "
            f"for qaoa_graph.qn, got {qaoa_aware['estimated_cycles']} < "
            f"{qaoa_agnostic['estimated_cycles']} — update the notebook's narrative, "
            "this claim no longer holds"
        )
        return 1

    # Exact-number check: the notebook's "Resource table" quotes 37/9/24 for
    # this cell specifically (not just "routing-aware costs more, direction
    # unspecified") — no #189 golden exists for `routing-aware`, so pin it
    # here instead of leaving it as an unchecked prose claim.
    for field, expected in QAOA_GRAPH_ROUTING_AWARE_EXPECTED.items():
        if qaoa_aware[field] != expected:
            print(
                f"FAIL: qaoa_graph.qn routing-aware {field} = {qaoa_aware[field]!r}, "
                f"but the notebook's Resource table claims {expected!r} — update "
                "both na_resource_study.ipynb and this script's "
                "QAOA_GRAPH_ROUTING_AWARE_EXPECTED"
            )
            return 1

    print(
        "PASS: qaoa_graph.qn's routing-aware placer costs "
        f"{qaoa_aware['estimated_cycles']} cycles vs routing-agnostic's "
        f"{qaoa_agnostic['estimated_cycles']} — routing-awareness doesn't pay off "
        "on this low-degree graph, matching the notebook's Resource table exactly"
    )
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except SmokeError as exc:
        print(f"FAIL: {exc}")
        sys.exit(1)
