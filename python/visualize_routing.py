#!/usr/bin/env python3
"""Visualize fixed-target qubit routing: device topology + gate-by-gate trace (issue #135).

Consumes the OpenQASM 3 emitted by ``quonc --target ... --emit-qasm`` (already
physical-qubit-indexed — ADR-0034: SSA wiring is the canonical layout identity,
so the QASM *is* the routed circuit) plus the target device JSON's
``topology.edges``. Renders one frame per two-qubit event on the device
connectivity graph: a genuine interaction (native 2-qubit gate) or a detected
SWAP.

SWAP detection reads the gate trace directly rather than trusting
``metrics.json``'s ``swap_count``: physical passes decompose a routed SWAP
into three consecutive CX ops on the same pair before metrics collection runs,
so ``swap_count`` is 0 even when SABRE inserted a SWAP (see
samples/visualization's ``dense-swap-mismatch`` entry, #135). This script
recognizes that literal ``cx a,b; cx b,a; cx a,b`` triple in the instruction
stream and reports it as one SWAP event — pass ``--metrics-json`` to also
print the mismatch against the (possibly zero) counter.

Examples::

  quonc program.qn --target targets/ibm/fake_manila_v2.json --emit-qasm > /tmp/out.qasm
  python/visualize_routing.py /tmp/out.qasm --target targets/ibm/fake_manila_v2.json \
    -o /tmp/route --format svg

  # Cross-check against the (possibly misleading) metrics counter:
  quonc program.qn --target targets/ibm/fake_manila_v2.json --emit-qasm \
    --metrics-json /tmp/out.metrics.json > /tmp/out.qasm
  python/visualize_routing.py /tmp/out.qasm --target targets/ibm/fake_manila_v2.json \
    --metrics-json /tmp/out.metrics.json -o /tmp/route
"""

from __future__ import annotations

import argparse
import json
import math
import re
import sys
from pathlib import Path
from typing import Any, NamedTuple

# Gate names treated as barriers/measurement — never routing events, even
# though some (barrier) can take multiple qubit operands.
NON_ROUTING_GATES = {"barrier", "reset"}

GATE_RE = re.compile(
    r"^\s*([a-zA-Z_][\w]*)\s*(?:\([^)]*\))?\s+((?:q\[\d+\]\s*,?\s*)+);"
)
QUBIT_REF_RE = re.compile(r"q\[(\d+)\]")
QUBIT_DECL_RE = re.compile(r"qubit\[(\d+)\]\s+(\w+)\s*;")


class GateOp(NamedTuple):
    name: str
    qubits: tuple[int, ...]
    line_no: int


class Event(NamedTuple):
    kind: str  # "swap" | "interaction"
    gate: str  # gate name (or "swap" for a detected/native swap)
    a: int
    b: int


def _die(msg: str, code: int = 1) -> None:
    print(f"visualize_routing: {msg}", file=sys.stderr)
    raise SystemExit(code)


def load_target(path: Path) -> dict[str, Any]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        _die(f"failed to read target JSON {path}: {exc}")
    topology = data.get("topology")
    if not isinstance(topology, dict) or "edges" not in topology:
        _die(f"target {path} has no topology.edges (Fixed targets only)")
    return data


def parse_qasm_gates(text: str) -> tuple[int, list[GateOp]]:
    """Return (declared qubit count, ordered gate operand list)."""
    num_qubits = 0
    ops: list[GateOp] = []
    for i, raw in enumerate(text.splitlines(), start=1):
        line = raw.split("//", 1)[0].strip()
        if not line:
            continue
        decl = QUBIT_DECL_RE.match(line)
        if decl:
            num_qubits = int(decl.group(1))
            continue
        m = GATE_RE.match(line)
        if not m:
            continue
        name = m.group(1)
        if name in NON_ROUTING_GATES:
            continue
        qubits = tuple(int(q) for q in QUBIT_REF_RE.findall(m.group(2)))
        if len(qubits) < 2:
            continue  # single-qubit gate: no routing/connectivity content
        ops.append(GateOp(name=name, qubits=qubits, line_no=i))
    return num_qubits, ops


def detect_events(ops: list[GateOp]) -> list[Event]:
    """Collapse literal SWAP-decomposition triples; pass through other 2q ops.

    A routed SWAP on a fixed target lowers to ``cx a,b; cx b,a; cx a,b``
    (three consecutive CX on the same unordered pair, alternating direction)
    *before* metrics run — this is why ``metrics.json``'s ``swap_count`` reads
    0 on a circuit that genuinely needed routing (#135's documented mismatch).
    Detecting the pattern here, on the actual gate trace, is the fix.
    """
    events: list[Event] = []
    i = 0
    n = len(ops)
    while i < n:
        op = ops[i]
        if len(op.qubits) != 2:
            # 3+ qubit native op (rare/unsupported downstream) — pass through
            # as its own event on its first two qubits so it's still visible.
            events.append(Event("interaction", op.name, op.qubits[0], op.qubits[1]))
            i += 1
            continue
        a, b = op.qubits
        if (
            op.name == "cx"
            and i + 2 < n
            and ops[i + 1].name == "cx"
            and ops[i + 2].name == "cx"
            and ops[i + 1].qubits == (b, a)
            and ops[i + 2].qubits == (a, b)
        ):
            events.append(Event("swap", "swap", a, b))
            i += 3
            continue
        events.append(Event("interaction", op.name, a, b))
        i += 1
    return events


def circular_layout(num_qubits: int) -> dict[int, tuple[float, float]]:
    pos: dict[int, tuple[float, float]] = {}
    if num_qubits <= 0:
        return pos
    for q in range(num_qubits):
        theta = 2 * math.pi * q / num_qubits - math.pi / 2
        pos[q] = (math.cos(theta), math.sin(theta))
    return pos


def path_order(num_qubits: int, edges: list[tuple[int, int]]) -> list[int] | None:
    """If the topology is a single simple path, return qubits in path order."""
    if num_qubits <= 0:
        return None
    adj: dict[int, list[int]] = {q: [] for q in range(num_qubits)}
    for u, v in edges:
        if u not in adj or v not in adj:
            return None
        adj[u].append(v)
        adj[v].append(u)
    if any(len(neighbors) > 2 for neighbors in adj.values()):
        return None  # branching — not a simple path/ring
    endpoints = [q for q, neighbors in adj.items() if len(neighbors) == 1]
    if len(endpoints) != 2:
        return None  # 0 endpoints = ring, or a disconnected graph
    order = [endpoints[0]]
    prev = None
    cur = endpoints[0]
    while len(order) < num_qubits:
        nxt = [n for n in adj[cur] if n != prev]
        if not nxt:
            return None
        prev, cur = cur, nxt[0]
        order.append(cur)
    return order if len(set(order)) == num_qubits else None


def layout_for(num_qubits: int, edges: list[tuple[int, int]]) -> dict[int, tuple[float, float]]:
    order = path_order(num_qubits, edges)
    if order is None:
        return circular_layout(num_qubits)
    pos: dict[int, tuple[float, float]] = {}
    span = max(num_qubits - 1, 1)
    for i, q in enumerate(order):
        pos[q] = (2 * i / span - 1, 0.0)
    return pos


def render(
    num_qubits: int,
    edges: list[tuple[int, int]],
    events: list[Event],
    target_id: str,
    out_prefix: Path,
    fmt: str,
) -> list[Path]:
    try:
        import matplotlib

        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError as exc:
        _die(f"matplotlib is required to render frames ({exc})")

    pos = layout_for(num_qubits, edges)
    is_line = all(y == 0.0 for _, y in pos.values())
    figsize = (8, 3) if is_line else (6, 6)
    written: list[Path] = []
    swap_count = 0
    interaction_count = 0

    for step, ev in enumerate(events):
        if ev.kind == "swap":
            swap_count += 1
        else:
            interaction_count += 1

        fig, ax = plt.subplots(figsize=figsize)

        for u, v in edges:
            pu, pv = pos.get(u), pos.get(v)
            if pu is None or pv is None:
                continue
            ax.plot([pu[0], pv[0]], [pu[1], pv[1]], color="#cccccc", lw=1.5, zorder=1)

        pa, pb = pos.get(ev.a), pos.get(ev.b)
        if pa is not None and pb is not None:
            color = "#e91e63" if ev.kind == "swap" else "#1f77b4"
            style = "dashed" if ev.kind == "swap" else "solid"
            ax.plot(
                [pa[0], pb[0]],
                [pa[1], pb[1]],
                color=color,
                lw=3.5,
                linestyle=style,
                zorder=3,
            )

        active = {ev.a, ev.b}
        for q, (x, y) in pos.items():
            is_active = q in active
            ax.scatter(
                [x],
                [y],
                s=420,
                c="#111111" if is_active else "white",
                edgecolors="#111111",
                linewidths=1.5,
                zorder=4,
            )
            ax.annotate(
                f"q{q}",
                (x, y),
                color="white" if is_active else "#111111",
                ha="center",
                va="center",
                fontsize=9,
                fontweight="bold",
                zorder=5,
            )

        label = f"SWAP q{ev.a},q{ev.b}" if ev.kind == "swap" else f"{ev.gate} q{ev.a},q{ev.b}"
        ax.set_title(
            f"step {step} [{label}]  |  target={target_id}  |  "
            f"swaps={swap_count} interactions={interaction_count}",
            fontsize=10,
        )
        ax.set_xlim(-1.4, 1.4)
        ax.set_ylim(-1.4, 1.4)
        if all(y == 0.0 for _, y in pos.values()):
            ax.set_ylim(-0.4, 0.4)
        ax.set_aspect("equal", adjustable="box")
        ax.axis("off")

        out_path = Path(f"{out_prefix}-step-{step:03d}.{fmt}")
        out_path.parent.mkdir(parents=True, exist_ok=True)
        fig.tight_layout()
        fig.savefig(out_path, dpi=140)
        plt.close(fig)
        written.append(out_path)

    return written


def print_summary(
    events: list[Event],
    edges: list[tuple[int, int]],
    metrics_path: Path | None,
) -> None:
    swaps = [e for e in events if e.kind == "swap"]
    interactions = [e for e in events if e.kind == "interaction"]
    edges_used = {tuple(sorted((e.a, e.b))) for e in events}
    edges_available = {tuple(sorted(e)) for e in edges}

    print("visualize_routing summary")
    print(f"  two-qubit events   : {len(events)}")
    print(f"  interactions       : {len(interactions)}")
    print(f"  swaps (from trace) : {len(swaps)}")
    print(
        f"  device edges used  : {len(edges_used & edges_available)}/{len(edges_available)}"
    )
    off_topology = edges_used - edges_available
    if off_topology:
        print(f"  WARNING: {len(off_topology)} event pair(s) not in target topology: {sorted(off_topology)}")

    if metrics_path is not None:
        try:
            declared = json.loads(metrics_path.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as exc:
            print(f"  (could not read --metrics-json {metrics_path}: {exc})")
            return
        declared_swaps = declared.get("metrics", declared).get("swap_count")
        if declared_swaps is None:
            print("  (--metrics-json has no swap_count field)")
        elif declared_swaps != len(swaps):
            print(
                f"  MISMATCH: metrics.json swap_count={declared_swaps} but the gate "
                f"trace shows {len(swaps)} swap(s) — metrics counts literal SWAP ops, "
                "which physical passes already decompose to CX triples before metrics "
                "run (#135). Trust the trace, not the counter."
            )
        else:
            print(f"  metrics.json swap_count={declared_swaps} matches the trace.")


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description="Render fixed-target routing: device topology + gate-by-gate SWAP/interaction trace.",
    )
    p.add_argument("qasm", type=Path, help="OpenQASM 3 from quonc --emit-qasm (Fixed target)")
    p.add_argument("--target", type=Path, required=True, help="Target device JSON (topology.edges)")
    p.add_argument("--metrics-json", type=Path, default=None, help="Optional quonc --metrics-json to cross-check swap_count")
    p.add_argument("-o", "--out", type=Path, default=Path("routing-viz"), help="Output prefix (default: routing-viz)")
    p.add_argument("--format", choices=("png", "svg"), default="svg", help="Image format (default: svg)")
    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)

    try:
        qasm_text = args.qasm.read_text(encoding="utf-8")
    except OSError as exc:
        _die(f"failed to read QASM {args.qasm}: {exc}")

    target = load_target(args.target)
    edges = [(int(a), int(b)) for a, b in target["topology"]["edges"]]
    target_id = target.get("id", args.target.stem)

    declared_qubits, ops = parse_qasm_gates(qasm_text)
    num_qubits = max(declared_qubits, target.get("num_qubits", declared_qubits))
    if declared_qubits and target.get("num_qubits") and declared_qubits != target["num_qubits"]:
        print(
            f"visualize_routing: warning: QASM declares {declared_qubits} qubits, "
            f"target has {target['num_qubits']}",
            file=sys.stderr,
        )

    events = detect_events(ops)
    if not events:
        _die("no two-qubit events found in QASM — nothing to render")

    paths = render(num_qubits, edges, events, target_id, args.out, args.format)
    for path in paths:
        print(path)
    print_summary(events, edges, args.metrics_json)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
