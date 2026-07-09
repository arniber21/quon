#!/usr/bin/env python3
"""Extract circuit metrics from OpenQASM 3 / Quil text and quilc statistics.

Metrics (issue #116):
  - two_qubit_count: CX/CNOT/CZ/SWAP/ISWAP/CPHASE/PISWAP/...
  - depth: critical-path depth (layering by qubit occupancy), or quilc's
    reported "Compiled gate depth" when available
  - t_count: T / Tdg / DAGGER T occurrences in the *scanned* program text
"""

from __future__ import annotations

import re
from dataclasses import dataclass
from typing import Iterable, Optional


TWO_Q = {
    "cx",
    "cnot",
    "cz",
    "cy",
    "swap",
    "iswap",
    "cphase",
    "cp",
    "crz",
    "crx",
    "cry",
    "rzz",
    "rxx",
    "ryy",
    "piswap",
    "xy",
    "ccx",
    "cswap",
    "toffoli",
}

T_GATES = {"t", "tdg", "t†", "t_dg"}


@dataclass
class Metrics:
    two_qubit_count: int
    depth: int
    t_count: int
    gate_count: int
    swap_count: int
    source: str = ""


_QASM_GATE = re.compile(
    r"^\s*([A-Za-z_][\w.]*)\s*(?:\([^)]*\))?\s+(.+?)\s*;\s*$"
)
_QUIL_GATE = re.compile(
    r"^\s*(?:DAGGER\s+)?([A-Za-z_][\w]*)\s*(?:\([^)]*\))?\s*(.*)$"
)
_QUIL_STAT_DEPTH = re.compile(r"Compiled gate depth:\s*(\d+)", re.I)
_QUIL_STAT_MQ_DEPTH = re.compile(r"Compiled multiqubit gate depth:\s*(\d+)", re.I)
_QUIL_STAT_VOLUME = re.compile(r"Compiled gate volume:\s*(\d+)", re.I)
_QUIL_STAT_SWAPS = re.compile(r"SWAPs incurred[^:]*:\s*(\d+)", re.I)


def _qubit_ids(args: str) -> list[str]:
    """Pull qubit identifiers from an argument list."""
    # OpenQASM: q[0], q[1]  | Quil: 0 1  | mixed
    ids = re.findall(r"q\[(\d+)\]|\[(\d+)\]|\b(\d+)\b", args)
    out: list[str] = []
    for a, b, c in ids:
        out.append(a or b or c)
    # Also named qubits like q0
    if not out:
        out = re.findall(r"\b[A-Za-z_]\w*\b", args)
    return out


def _is_t_name(name: str, dagger_prefix: bool = False) -> bool:
    n = name.lower()
    if dagger_prefix and n == "t":
        return True
    return n in T_GATES or n in {"t†"}


def metrics_from_qasm(text: str) -> Metrics:
    """Parse OpenQASM 3 gate lines (stdgates-style)."""
    two_q = 0
    t_count = 0
    gate_count = 0
    swap_count = 0
    # qubit -> next free layer
    layers: dict[str, int] = {}
    depth = 0

    for raw in text.splitlines():
        line = raw.split("//")[0].strip()
        if not line or line.startswith(("OPENQASM", "include", "qubit", "bit", "const")):
            continue
        if "=" in line and "measure" in line.lower():
            continue
        if line.startswith(("measure", "reset", "barrier")):
            continue
        m = _QASM_GATE.match(line)
        if not m:
            continue
        name = m.group(1).split(".")[-1].lower()
        args = m.group(2)
        qids = _qubit_ids(args)
        if not qids:
            continue
        gate_count += 1
        if name in TWO_Q or name.startswith("c") and len(qids) >= 2:
            if name in TWO_Q or len(qids) >= 2:
                two_q += 1
        if name == "swap" or name == "cswap":
            swap_count += 1
        if _is_t_name(name):
            t_count += 1

        start = max(layers.get(q, 0) for q in qids)
        end = start + 1
        for q in qids:
            layers[q] = end
        depth = max(depth, end)

    return Metrics(
        two_qubit_count=two_q,
        depth=depth,
        t_count=t_count,
        gate_count=gate_count,
        swap_count=swap_count,
        source="qasm",
    )


def metrics_from_quil(text: str, prefer_quilc_depth: bool = True) -> Metrics:
    """Parse Quil instructions; optionally prefer quilc --print-statistics depth."""
    two_q = 0
    t_count = 0
    gate_count = 0
    swap_count = 0
    layers: dict[str, int] = {}
    layered_depth = 0

    quilc_depth: Optional[int] = None
    quilc_swaps: Optional[int] = None
    quilc_volume: Optional[int] = None

    for raw in text.splitlines():
        # statistics comments
        if "Compiled gate depth:" in raw:
            m = _QUIL_STAT_DEPTH.search(raw)
            if m:
                quilc_depth = int(m.group(1))
            continue
        if "SWAPs incurred" in raw:
            m = _QUIL_STAT_SWAPS.search(raw)
            if m:
                quilc_swaps = int(m.group(1))
            continue
        if "Compiled gate volume:" in raw:
            m = _QUIL_STAT_VOLUME.search(raw)
            if m:
                quilc_volume = int(m.group(1))
            continue
        if raw.lstrip().startswith("#"):
            continue

        line = raw.split("#")[0].strip()
        if not line:
            continue
        upper = line.upper()
        if upper.startswith(
            ("DECLARE", "HALT", "LABEL", "JUMP", "WAIT", "RESET", "MEASURE", "PRAGMA", "DEFCIRCUIT", "DEFGATE")
        ):
            continue

        dagger = upper.startswith("DAGGER ")
        body = line[7:].lstrip() if dagger else line
        m = _QUIL_GATE.match(body)
        if not m:
            continue
        name = m.group(1)
        args = m.group(2) or ""
        qids = _qubit_ids(args)
        if not qids and name.upper() not in {"HALT"}:
            # gates like I with no args shouldn't happen; skip non-gate
            continue
        if not qids:
            continue

        lname = name.lower()
        gate_count += 1
        if lname in TWO_Q or len(qids) >= 2:
            two_q += 1
        if lname == "swap":
            swap_count += 1
        if _is_t_name(lname, dagger_prefix=dagger):
            t_count += 1

        start = max(layers.get(q, 0) for q in qids)
        end = start + 1
        for q in qids:
            layers[q] = end
        layered_depth = max(layered_depth, end)

    depth = layered_depth
    if prefer_quilc_depth and quilc_depth is not None:
        depth = quilc_depth
    if quilc_swaps is not None:
        # quilc reports routing SWAPs separately; keep parsed swap_count as gate SWAPs
        pass
    if quilc_volume is not None and gate_count == 0:
        gate_count = quilc_volume

    return Metrics(
        two_qubit_count=two_q,
        depth=depth,
        t_count=t_count,
        gate_count=gate_count,
        swap_count=swap_count,
        source="quil",
    )


def parse_quilc_stats(text: str) -> dict:
    out = {}
    for pat, key in (
        (_QUIL_STAT_DEPTH, "gate_depth"),
        (_QUIL_STAT_MQ_DEPTH, "multiqubit_gate_depth"),
        (_QUIL_STAT_VOLUME, "gate_volume"),
        (_QUIL_STAT_SWAPS, "routing_swaps"),
    ):
        m = pat.search(text)
        if m:
            out[key] = int(m.group(1))
    return out


if __name__ == "__main__":
    import sys

    data = sys.stdin.read()
    if "OPENQASM" in data:
        m = metrics_from_qasm(data)
    else:
        m = metrics_from_quil(data)
    print(
        f"2q={m.two_qubit_count} depth={m.depth} t={m.t_count} "
        f"gates={m.gate_count} swaps={m.swap_count}"
    )
