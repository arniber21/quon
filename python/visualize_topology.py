#!/usr/bin/env python3
"""Standalone coupling-graph viewer for a fixed-target mapping (issue #136).

Reads a device JSON (`topology.edges`) and a ``mapping_trace`` v1 document
from ``quonc --emit-mapping-json``. Draws the coupling graph, overlays the
compile's layout on the physical nodes, and prints an ASCII view. Validation
rejects a two-qubit routing event whose physical pair is not a device edge.

No editor embedding and no third-party packages. Neutral-atom timelines are
out of scope: ``quonc`` does not emit ``--emit-na-json``.

Examples::

  python/visualize_topology.py backend/tests/fixtures/device_5q.json \\
    python/testdata/toy_mapping_trace.json --ascii
  python/visualize_topology.py device.json mapping.json --html /tmp/topo.html
  python/visualize_topology.py device.json mapping.json --validate
"""

from __future__ import annotations

import argparse
import html
import json
import math
import sys
from pathlib import Path
from typing import Any

SCHEMA_VERSION = 1
KIND = "mapping_trace"

TRACE_KEYS = {
    "schema_version",
    "kind",
    "summary",
    "meta",
    "topology",
    "initial_layout",
    "final_layout",
    "events",
    "stages",
}
META_KEYS = {"target_id"}
TOPOLOGY_KEYS = {"edges"}
ASSIGNMENT_KEYS = {"logical", "physical"}
STAGE_KEYS = {"id", "summary", "metrics"}
METRICS_KEYS = {"gate_count", "depth", "swap_count", "t_count"}
SWAP_KEYS = {"kind", "logical", "physical", "summary"}
INTERACTION_KEYS = {"kind", "gate", "logical", "physical", "summary"}
BRANCH_KEYS = {"kind", "arm", "summary", "events", "layout"}


def _die(msg: str, code: int = 1) -> None:
    print(f"visualize_topology: {msg}", file=sys.stderr)
    raise SystemExit(code)


def _require_object(value: Any, where: str) -> dict[str, Any]:
    if not isinstance(value, dict):
        raise ValueError(f"{where} must be a JSON object")
    return value


def _reject_unknown(obj: dict[str, Any], allowed: set[str], where: str) -> None:
    extra = sorted(set(obj) - allowed)
    if extra:
        raise ValueError(f"{where} has unknown field(s): {', '.join(extra)}")


def _require_fields(obj: dict[str, Any], required: set[str], where: str) -> None:
    missing = sorted(required - set(obj))
    if missing:
        raise ValueError(f"{where} is missing field(s): {', '.join(missing)}")


def _pair(value: Any, where: str) -> list[int]:
    if (
        not isinstance(value, list)
        or len(value) != 2
        or not all(isinstance(item, int) and not isinstance(item, bool) for item in value)
    ):
        raise ValueError(f"{where} must be a pair of integers")
    return [int(value[0]), int(value[1])]


def _int_field(obj: dict[str, Any], key: str, where: str) -> int:
    value = obj.get(key)
    if isinstance(value, bool) or not isinstance(value, int):
        raise ValueError(f"{where}.{key} must be an integer")
    return value


def _str_field(obj: dict[str, Any], key: str, where: str) -> str:
    value = obj.get(key)
    if not isinstance(value, str) or not value:
        raise ValueError(f"{where}.{key} must be a non-empty string")
    return value


def _validate_layout(rows: Any, where: str) -> None:
    if not isinstance(rows, list):
        raise ValueError(f"{where} must be an array")
    for index, row in enumerate(rows):
        assignment = _require_object(row, f"{where}[{index}]")
        _reject_unknown(assignment, ASSIGNMENT_KEYS, f"{where}[{index}]")
        _require_fields(assignment, ASSIGNMENT_KEYS, f"{where}[{index}]")
        _int_field(assignment, "logical", f"{where}[{index}]")
        _int_field(assignment, "physical", f"{where}[{index}]")


def _validate_events(events: list[Any], where: str) -> None:
    for index, event in enumerate(events):
        obj = _require_object(event, f"{where}[{index}]")
        kind = obj.get("kind")
        at = f"{where}[{index}]"
        if kind == "swap":
            _reject_unknown(obj, SWAP_KEYS, at)
            _require_fields(obj, SWAP_KEYS, at)
            _pair(obj["logical"], f"{at}.logical")
            _pair(obj["physical"], f"{at}.physical")
            _str_field(obj, "summary", at)
        elif kind == "interaction":
            _reject_unknown(obj, INTERACTION_KEYS, at)
            _require_fields(obj, INTERACTION_KEYS, at)
            _str_field(obj, "gate", at)
            _pair(obj["logical"], f"{at}.logical")
            _pair(obj["physical"], f"{at}.physical")
            _str_field(obj, "summary", at)
        elif kind == "branch":
            _reject_unknown(obj, BRANCH_KEYS, at)
            _require_fields(obj, BRANCH_KEYS, at)
            arm = _str_field(obj, "arm", at)
            if arm not in ("then", "else"):
                raise ValueError(f"{at}.arm must be 'then' or 'else'")
            _str_field(obj, "summary", at)
            _validate_layout(obj["layout"], f"{at}.layout")
            nested = obj["events"]
            if not isinstance(nested, list):
                raise ValueError(f"{at}.events must be an array")
            _validate_events(nested, f"{at}.events")
        else:
            raise ValueError(f"{at}.kind must be 'swap', 'interaction', or 'branch'")


def load_trace(data: Any) -> dict[str, Any]:
    """Validate a mapping_trace document. Raises ValueError on schema drift."""
    trace = _require_object(data, "mapping trace")
    _reject_unknown(trace, TRACE_KEYS, "mapping trace")
    _require_fields(trace, TRACE_KEYS, "mapping trace")
    if trace["schema_version"] != SCHEMA_VERSION:
        raise ValueError(
            f"unsupported schema_version {trace['schema_version']!r} (expected {SCHEMA_VERSION})"
        )
    if trace["kind"] != KIND:
        raise ValueError(f"unsupported kind {trace['kind']!r} (expected {KIND!r})")
    if not isinstance(trace["summary"], str) or not trace["summary"]:
        raise ValueError("summary must be a non-empty string")

    meta = _require_object(trace["meta"], "meta")
    _reject_unknown(meta, META_KEYS, "meta")
    _require_fields(meta, META_KEYS, "meta")
    _str_field(meta, "target_id", "meta")

    topology = _require_object(trace["topology"], "topology")
    _reject_unknown(topology, TOPOLOGY_KEYS, "topology")
    _require_fields(topology, TOPOLOGY_KEYS, "topology")
    edges = topology["edges"]
    if not isinstance(edges, list):
        raise ValueError("topology.edges must be an array")
    for index, edge in enumerate(edges):
        _pair(edge, f"topology.edges[{index}]")

    for name in ("initial_layout", "final_layout"):
        _validate_layout(trace[name], name)

    events = trace["events"]
    if not isinstance(events, list):
        raise ValueError("events must be an array")
    _validate_events(events, "events")

    stages = trace["stages"]
    if not isinstance(stages, list) or not stages:
        raise ValueError("stages must be a non-empty array")
    for index, stage in enumerate(stages):
        obj = _require_object(stage, f"stages[{index}]")
        where = f"stages[{index}]"
        _reject_unknown(obj, STAGE_KEYS, where)
        _require_fields(obj, STAGE_KEYS, where)
        _str_field(obj, "id", where)
        _str_field(obj, "summary", where)
        metrics = _require_object(obj["metrics"], f"{where}.metrics")
        _reject_unknown(metrics, METRICS_KEYS, f"{where}.metrics")
        _require_fields(metrics, METRICS_KEYS, f"{where}.metrics")
        for key in METRICS_KEYS:
            _int_field(metrics, key, f"{where}.metrics")
    return trace


def load_device(data: Any) -> dict[str, Any]:
    """Read the fields the coupling graph needs. Extra device fields are kept."""
    device = _require_object(data, "device")
    device_id = device.get("id")
    if not isinstance(device_id, str) or not device_id:
        raise ValueError("device.id must be a non-empty string")
    num_qubits = device.get("num_qubits")
    if isinstance(num_qubits, bool) or not isinstance(num_qubits, int) or num_qubits < 0:
        raise ValueError("device.num_qubits must be a non-negative integer")
    topology = _require_object(device.get("topology"), "device.topology")
    edges = topology.get("edges")
    if not isinstance(edges, list):
        raise ValueError("device.topology.edges must be an array")
    parsed: list[tuple[int, int]] = []
    for index, edge in enumerate(edges):
        left, right = _pair(edge, f"device.topology.edges[{index}]")
        if left == right:
            raise ValueError(f"device.topology.edges[{index}] is a self-loop")
        if left < 0 or right < 0 or left >= num_qubits or right >= num_qubits:
            raise ValueError(
                f"device.topology.edges[{index}] endpoint is outside {device_id} ({num_qubits} qubits)"
            )
        parsed.append((left, right) if left < right else (right, left))
    parsed = sorted(set(parsed))
    return {
        "id": device_id,
        "num_qubits": num_qubits,
        "edges": parsed,
    }


def coupling_edges(device: dict[str, Any]) -> set[tuple[int, int]]:
    return set(device["edges"])


def layout_rows(trace: dict[str, Any]) -> tuple[str, list[dict[str, Any]]]:
    """Prefer the post-routing layout. Fall back when a diverging branch cleared it."""
    if trace["final_layout"]:
        return "final", trace["final_layout"]
    return "initial", trace["initial_layout"]


def physical_owners(rows: list[dict[str, Any]]) -> dict[int, int]:
    return {int(row["physical"]): int(row["logical"]) for row in rows}


def _edge_label(pair: tuple[int, int]) -> str:
    return f"({pair[0]}, {pair[1]})"


def validate_routing(device: dict[str, Any], trace: dict[str, Any]) -> list[str]:
    """Human-readable violations. Empty means the mapping sits on the device."""
    errors: list[str] = []
    device_id = device["id"]
    num_qubits = device["num_qubits"]
    edges = coupling_edges(device)

    def check_layout(name: str, rows: list[dict[str, Any]]) -> None:
        seen: dict[int, int] = {}
        for row in rows:
            logical = int(row["logical"])
            physical = int(row["physical"])
            if physical < 0 or physical >= num_qubits:
                errors.append(
                    f"{name}: physical qubit {physical} is outside {device_id} ({num_qubits} qubits)"
                )
                continue
            previous = seen.get(physical)
            if previous is not None:
                errors.append(
                    f"{name}: physical qubit {physical} is assigned to logical qubits {previous} and {logical}"
                )
            else:
                seen[physical] = logical

    check_layout("initial_layout", trace["initial_layout"])
    check_layout("final_layout", trace["final_layout"])

    def walk(events: list[dict[str, Any]], where: str) -> None:
        for index, event in enumerate(events):
            at = f"{where}[{index}]"
            if event["kind"] == "branch":
                check_layout(f"{at}.layout", event["layout"])
                walk(event["events"], f"{at}.events")
                continue
            left, right = int(event["physical"][0]), int(event["physical"][1])
            for qubit in (left, right):
                if qubit < 0 or qubit >= num_qubits:
                    errors.append(
                        f"{at}: physical qubit {qubit} is outside {device_id} ({num_qubits} qubits)"
                    )
            pair = (left, right) if left < right else (right, left)
            if left == right or pair not in edges:
                gate = event["gate"] if event["kind"] == "interaction" else "swap"
                errors.append(
                    f"{at}: {event['kind']} {gate} on physical qubits ({left}, {right}) "
                    f"is not a coupling edge of {device_id}"
                )

    walk(trace["events"], "events")
    return errors


def _layout_line(rows: list[dict[str, Any]]) -> str:
    if not rows:
        return "(empty)"
    ordered = sorted(rows, key=lambda row: int(row["logical"]))
    return " ".join(f"{row['logical']}→{row['physical']}" for row in ordered)


def render_ascii(device: dict[str, Any], trace: dict[str, Any]) -> str:
    """Terminal coupling graph with the compile's layout overlaid."""
    which, rows = layout_rows(trace)
    owners = physical_owners(rows)
    lines = [
        f"quontopo  device={device['id']}  qubits={device['num_qubits']}  "
        f"trace={trace['meta']['target_id']}",
        f"summary: {trace['summary']}",
        f"{which} layout: {_layout_line(rows)}",
        "coupling:",
    ]
    if not device["edges"]:
        lines.append("  (no edges)")
    else:
        for left, right in device["edges"]:
            left_label = f"L{owners[left]}" if left in owners else "."
            right_label = f"L{owners[right]}" if right in owners else "."
            lines.append(f"  q{left}[{left_label}] — q{right}[{right_label}]")
    lines.append("nodes:")
    for qubit in range(device["num_qubits"]):
        if qubit in owners:
            lines.append(f"  q{qubit}  logical {owners[qubit]}")
        else:
            lines.append(f"  q{qubit}  (unoccupied)")
    violations = validate_routing(device, trace)
    if violations:
        lines.append("validation: failed")
        for violation in violations:
            lines.append(f"  {violation}")
    else:
        lines.append("validation: ok")
    return "\n".join(lines) + "\n"


def _positions(num_qubits: int) -> dict[int, tuple[float, float]]:
    pos: dict[int, tuple[float, float]] = {}
    if num_qubits == 0:
        return pos
    if num_qubits == 1:
        return {0: (200.0, 200.0)}
    for qubit in range(num_qubits):
        theta = 2 * math.pi * qubit / num_qubits - math.pi / 2
        pos[qubit] = (
            round(200 + 140 * math.cos(theta), 2),
            round(200 + 140 * math.sin(theta), 2),
        )
    return pos


def render_html(device: dict[str, Any], trace: dict[str, Any]) -> str:
    """Self-contained HTML coupling graph. No external assets."""
    which, rows = layout_rows(trace)
    owners = physical_owners(rows)
    pos = _positions(device["num_qubits"])
    edge_svg = []
    for left, right in device["edges"]:
        x1, y1 = pos[left]
        x2, y2 = pos[right]
        edge_svg.append(
            f'<line x1="{x1}" y1="{y1}" x2="{x2}" y2="{y2}" stroke="#8aa0b4" stroke-width="3"/>'
        )
    node_svg = []
    for qubit in range(device["num_qubits"]):
        x, y = pos[qubit]
        label = f"q{qubit}"
        if qubit in owners:
            label = f"q{qubit} L{owners[qubit]}"
        node_svg.append(
            f'<circle cx="{x}" cy="{y}" r="22" fill="#14202b"/>'
            f'<text x="{x}" y="{y + 4}" text-anchor="middle" fill="#ffffff" '
            f'font-size="11">{html.escape(label)}</text>'
        )
    title = html.escape(f"{device['id']} — {trace['meta']['target_id']}")
    summary = html.escape(trace["summary"])
    layout = html.escape(f"{which} layout: {_layout_line(rows)}")
    violations = validate_routing(device, trace)
    if violations:
        status = "validation failed: " + "; ".join(violations)
    else:
        status = "validation: ok"
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>quontopo — {title}</title>
<style>
body {{ font: 15px/1.45 sans-serif; margin: 1.5rem; color: #14202b; }}
h1 {{ font-size: 1.25rem; }}
svg {{ background: #f4f7fa; border: 1px solid #d5dde5; }}
</style>
</head>
<body>
<h1>quontopo — {title}</h1>
<p>{summary}</p>
<p>{layout}</p>
<p>{html.escape(status)}</p>
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 400 400" width="420" height="420" role="img" aria-label="coupling graph">
{"".join(edge_svg)}
{"".join(node_svg)}
</svg>
</body>
</html>
"""


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Draw a device coupling graph and overlay a mapping_trace layout.",
    )
    parser.add_argument("device", type=Path, help="fixed-target device JSON")
    parser.add_argument("trace", type=Path, help="mapping_trace JSON from quonc --emit-mapping-json")
    parser.add_argument("--ascii", action="store_true", help="print the ASCII coupling graph")
    parser.add_argument("--html", type=Path, default=None, help="write a self-contained HTML file")
    parser.add_argument(
        "--validate",
        action="store_true",
        help="check routing events against coupling edges and exit non-zero on a violation",
    )
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if not args.ascii and args.html is None and not args.validate:
        args.ascii = True
    try:
        device_data = json.loads(args.device.read_text(encoding="utf-8"))
        trace_data = json.loads(args.trace.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        _die(f"failed to read input: {exc}")
    try:
        device = load_device(device_data)
        trace = load_trace(trace_data)
    except ValueError as exc:
        _die(str(exc))
    violations = validate_routing(device, trace)
    if args.validate or violations:
        if violations:
            print("visualize_topology: validation failed", file=sys.stderr)
            for violation in violations:
                print(f"visualize_topology: {violation}", file=sys.stderr)
            return 1
        if args.validate:
            print("visualize_topology: validation ok")
    if args.ascii:
        sys.stdout.write(render_ascii(device, trace))
    if args.html is not None:
        args.html.parent.mkdir(parents=True, exist_ok=True)
        args.html.write_text(render_html(device, trace), encoding="utf-8")
        print(args.html, file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
