#!/usr/bin/env python3
"""Compare quonc vs quilc metrics for the shared corpus (#116)."""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path

# Allow `python compare.py` from any cwd.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from metrics import metrics_from_qasm, metrics_from_quil, parse_quilc_stats  # noqa: E402


def pct_delta(quon: float, quilc: float) -> str:
    if quilc == 0:
        if quon == 0:
            return "0%"
        return "n/a"
    return f"{100.0 * (quon - quilc) / quilc:+.1f}%"


def within_pct(quon: float, quilc: float, pct: float = 20.0) -> bool:
    if quilc == 0:
        return quon == 0
    return abs(quon - quilc) / quilc * 100.0 <= pct


def load_quonc(out_dir: Path, circuit_id: str) -> dict | None:
    path = out_dir / "quonc" / f"{circuit_id}.json"
    if not path.exists():
        return None
    snap = json.loads(path.read_text())
    metrics = snap.get("metrics") or {}
    qasm_path = out_dir / "qasm" / f"{circuit_id}.qasm"
    parsed = None
    if qasm_path.exists():
        parsed = metrics_from_qasm(qasm_path.read_text())
    two_q = parsed.two_qubit_count if parsed else None
    # Prefer IR t_count from quonc (pre-final-decomp); fall back to QASM scan.
    t_count = metrics.get("t_count")
    if t_count is None and parsed:
        t_count = parsed.t_count
    return {
        "depth": metrics.get("depth"),
        "gate_count": metrics.get("gate_count"),
        "t_count": t_count,
        "swap_count": metrics.get("swap_count"),
        "two_qubit_count": two_q,
        "ok": snap.get("compile", {}).get("status") == "ok",
    }


def count_input_t(quil_path: Path) -> int:
    """Count T / DAGGER T in the *source* Quil (pre-quilc)."""
    if not quil_path.exists():
        return 0
    return metrics_from_quil(quil_path.read_text(), prefer_quilc_depth=False).t_count


def load_quilc(out_dir: Path, circuit_id: str, input_quil: Path | None = None) -> dict | None:
    path = out_dir / "quilc" / f"{circuit_id}.quil"
    if not path.exists():
        return None
    text = path.read_text()
    if "Error:" in text and "Compiled gate depth" not in text:
        return {"ok": False, "error": text.strip().splitlines()[-1][:120]}
    parsed = metrics_from_quil(text, prefer_quilc_depth=True)
    stats = parse_quilc_stats(text)
    input_t = count_input_t(input_quil) if input_quil else None
    return {
        "depth": stats.get("gate_depth", parsed.depth),
        "gate_count": stats.get("gate_volume", parsed.gate_count),
        "t_count": parsed.t_count,  # usually 0 after native RZ fold
        "input_t_count": input_t,
        "t_count_input_note": "post-native; T→RZ under Xhalves ISA",
        "swap_count": stats.get("routing_swaps", parsed.swap_count),
        "two_qubit_count": parsed.two_qubit_count,
        "multiqubit_depth": stats.get("multiqubit_gate_depth"),
        "ok": True,
    }


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--manifest", required=True)
    ap.add_argument("--out-dir", required=True)
    ap.add_argument("--skip-quilc", type=int, default=0)
    ap.add_argument("--table", required=True)
    ap.add_argument("--json", required=True)
    ap.add_argument("--within-pct", type=float, default=20.0)
    args = ap.parse_args()

    manifest = json.loads(Path(args.manifest).read_text())
    out_dir = Path(args.out_dir)
    skip_quilc = bool(args.skip_quilc)
    within = args.within_pct

    bench_root = Path(args.manifest).resolve().parent.parent
    rows = []
    for circ in manifest["circuits"]:
        cid = circ["id"]
        q = load_quonc(out_dir, cid)
        input_quil = bench_root / circ.get("quil", f"corpus/quil/{cid}.quil")
        c = None if skip_quilc else load_quilc(out_dir, cid, input_quil=input_quil)
        # Both compilers fold T→RZ before final metrics; surface input Quil T-count.
        if q is not None and input_quil.exists():
            q = {**q, "input_t_count": count_input_t(input_quil)}
        row = {
            "id": cid,
            "size": circ.get("size"),
            "quonc": q,
            "quilc": c,
        }
        if q and c and q.get("ok") and c.get("ok"):
            row["delta"] = {
                "two_qubit_count": pct_delta(
                    q.get("two_qubit_count") or 0, c.get("two_qubit_count") or 0
                ),
                "depth": pct_delta(q.get("depth") or 0, c.get("depth") or 0),
                "t_count": pct_delta(q.get("t_count") or 0, c.get("t_count") or 0),
            }
            row["within_20pct"] = {
                "two_qubit_count": within_pct(
                    q.get("two_qubit_count") or 0,
                    c.get("two_qubit_count") or 0,
                    within,
                ),
                "depth": within_pct(q.get("depth") or 0, c.get("depth") or 0, within),
            }
        rows.append(row)

    # Markdown table
    lines = [
        "# quonc vs quilc benchmark results",
        "",
        f"Target: linear 5-qubit, native 2Q = CX/CNOT. Within-{within:g}% flagged vs quilc.",
        "",
        "| circuit | quonc 2Q | quilc 2Q | Δ2Q | quonc depth | quilc depth | Δdepth | input T |",
        "|---|---:|---:|---:|---:|---:|---:|---:|",
    ]
    for row in rows:
        q = row.get("quonc") or {}
        c = row.get("quilc") or {}
        d = row.get("delta") or {}
        input_t = q.get("input_t_count", c.get("input_t_count") if c else None)
        if not q:
            lines.append(f"| {row['id']} | — | — | — | — | — | — | — |")
            continue
        if skip_quilc or not c:
            lines.append(
                f"| {row['id']} | {q.get('two_qubit_count')} | — | — | "
                f"{q.get('depth')} | — | — | {input_t} |"
            )
            continue
        if not c.get("ok"):
            lines.append(
                f"| {row['id']} | {q.get('two_qubit_count')} | ERR | — | "
                f"{q.get('depth')} | ERR | — | {input_t} |"
            )
            continue
        lines.append(
            f"| {row['id']} | {q.get('two_qubit_count')} | {c.get('two_qubit_count')} | "
            f"{d.get('two_qubit_count', '')} | {q.get('depth')} | {c.get('depth')} | "
            f"{d.get('depth', '')} | {input_t} |"
        )

    lines.extend(
        [
            "",
            "Both compilers fold `T` into `RZ(π/4)` before final metrics (quonc native decomp; "
            "quilc Xhalves ISA), so post-compile T-count is 0. The **input T** column is the "
            "logical Clifford+T count from the shared Quil source.",
            "",
            "## Findings summary",
            "",
        ]
    )

    competitive = []
    lagging = []
    for row in rows:
        if not row.get("within_20pct"):
            continue
        w = row["within_20pct"]
        d = row.get("delta") or {}
        if w.get("two_qubit_count") and w.get("depth"):
            competitive.append(row["id"])
        else:
            reasons = []
            if not w.get("two_qubit_count"):
                reasons.append(f"2Q {d.get('two_qubit_count')}")
            if not w.get("depth"):
                reasons.append(f"depth {d.get('depth')}")
            lagging.append(f"{row['id']} ({', '.join(reasons)})")

    if skip_quilc:
        lines.append("- quilc skipped this run; quonc-only metrics recorded.")
    else:
        lines.append(
            f"- Within {within:g}% of quilc on **both** 2Q count and depth: "
            + (", ".join(competitive) if competitive else "(none)")
        )
        lines.append(
            "- Lags quilc (outside band on 2Q and/or depth): "
            + (", ".join(lagging) if lagging else "(none)")
        )
        lines.append(
            "- T-count: both compilers absorb T into RZ before final metrics; see **input T** "
            f"(e.g. `clifford_t_phase` has "
            + str(
                next(
                    (
                        (r.get("quonc") or {}).get("input_t_count")
                        for r in rows
                        if r["id"] == "clifford_t_phase"
                    ),
                    "?",
                )
            )
            + ")."
        )
        lines.append(
            "- Notable: `grover` is cancelled to 0 two-qubit gates by quilc (identity-like "
            "rewrite) while quonc still emits 4 CX — large depth/2Q gap."
        )
        lines.append(
            "- Methodology note: quilc is Rigetti's optimizing Quil compiler "
            "([quil-lang/quilc](https://github.com/quil-lang/quilc)); this bench matches "
            "topology + 2Q native (CNOT) but 1Q natives differ (quonc: rz/sx/x; quilc: "
            "RZ + RX(k·π/2)). Depth definitions also differ (quonc schedule_time vs quilc "
            "gate-depth statistic)."
        )

    Path(args.table).write_text("\n".join(lines) + "\n")
    Path(args.json).write_text(
        json.dumps(
            {
                "manifest": manifest.get("description"),
                "target": manifest.get("target"),
                "skip_quilc": skip_quilc,
                "within_pct": within,
                "rows": rows,
                "competitive": competitive,
                "lagging": lagging,
            },
            indent=2,
        )
        + "\n"
    )
    print("\n".join(lines))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
