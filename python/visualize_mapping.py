#!/usr/bin/env python3
"""Standalone mapping viewer for ``quonc --emit-mapping-json`` (issue #135).

Reads a ``mapping_trace`` v1 document and prints an ASCII timeline, or writes
a self-contained HTML page with a stage selector. Unknown JSON fields are
rejected. No editor embedding and no third-party packages.

Examples::

  quonc program.qn --target targets/ibm/fake_manila_v2.json \\
    --emit-mapping-json mapping.json
  python/visualize_mapping.py mapping.json --ascii
  python/visualize_mapping.py mapping.json --html /tmp/mapping.html
"""

from __future__ import annotations

import argparse
import html
import json
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


def _die(msg: str, code: int = 1) -> None:
    print(f"visualize_mapping: {msg}", file=sys.stderr)
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
        rows = trace[name]
        if not isinstance(rows, list):
            raise ValueError(f"{name} must be an array")
        for index, row in enumerate(rows):
            assignment = _require_object(row, f"{name}[{index}]")
            _reject_unknown(assignment, ASSIGNMENT_KEYS, f"{name}[{index}]")
            _require_fields(assignment, ASSIGNMENT_KEYS, f"{name}[{index}]")
            _int_field(assignment, "logical", f"{name}[{index}]")
            _int_field(assignment, "physical", f"{name}[{index}]")

    events = trace["events"]
    if not isinstance(events, list):
        raise ValueError("events must be an array")
    for index, event in enumerate(events):
        obj = _require_object(event, f"events[{index}]")
        kind = obj.get("kind")
        where = f"events[{index}]"
        if kind == "swap":
            _reject_unknown(obj, SWAP_KEYS, where)
            _require_fields(obj, SWAP_KEYS, where)
        elif kind == "interaction":
            _reject_unknown(obj, INTERACTION_KEYS, where)
            _require_fields(obj, INTERACTION_KEYS, where)
            _str_field(obj, "gate", where)
        else:
            raise ValueError(f"{where}.kind must be 'swap' or 'interaction'")
        _pair(obj["logical"], f"{where}.logical")
        _pair(obj["physical"], f"{where}.physical")
        _str_field(obj, "summary", where)

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


def _layout_line(rows: list[dict[str, Any]]) -> str:
    if not rows:
        return "(empty)"
    return " ".join(f"{row['logical']}→{row['physical']}" for row in rows)


def _event_line(index: int, event: dict[str, Any]) -> str:
    logical = event["logical"]
    physical = event["physical"]
    if event["kind"] == "swap":
        head = (
            f"  {index:03d} swap logical=[{logical[0]}, {logical[1]}] "
            f"physical=[{physical[0]}, {physical[1]}]"
        )
    else:
        head = (
            f"  {index:03d} interaction {event['gate']} "
            f"logical=[{logical[0]}, {logical[1]}] "
            f"physical=[{physical[0]}, {physical[1]}]"
        )
    return f"{head}\n      {event['summary']}"


def render_ascii(trace: dict[str, Any]) -> str:
    """Terminal timeline. Stable for the checked-in golden."""
    lines = [
        f"mapping_trace v{trace['schema_version']}  target={trace['meta']['target_id']}",
        f"summary: {trace['summary']}",
        f"initial layout: {_layout_line(trace['initial_layout'])}",
        f"final layout: {_layout_line(trace['final_layout'])}",
        "stages:",
    ]
    for stage in trace["stages"]:
        metrics = stage["metrics"]
        lines.append(
            f"  {stage['id']}  gate_count={metrics['gate_count']} "
            f"depth={metrics['depth']} swap_count={metrics['swap_count']} "
            f"t_count={metrics['t_count']}"
        )
        lines.append(f"    {stage['summary']}")
    lines.append("events:")
    if not trace["events"]:
        lines.append("  (none)")
    else:
        for index, event in enumerate(trace["events"]):
            lines.append(_event_line(index, event))
    swaps = sum(1 for event in trace["events"] if event["kind"] == "swap")
    lines.append(f"swap insertions: {swaps}")
    return "\n".join(lines) + "\n"


def render_html(trace: dict[str, Any]) -> str:
    """Self-contained HTML with a stage selector. No external assets."""
    stage_options = []
    stage_sections = []
    for index, stage in enumerate(trace["stages"]):
        selected = " selected" if index == 0 else ""
        stage_id = html.escape(stage["id"], quote=True)
        stage_options.append(f'<option value="{index}"{selected}>{stage_id}</option>')
        metrics = stage["metrics"]
        hidden = "" if index == 0 else " hidden"
        stage_sections.append(
            f'<section class="stage" data-index="{index}"{hidden}>'
            f"<h2>{html.escape(stage['id'])}</h2>"
            f"<p>{html.escape(stage['summary'])}</p>"
            "<ul>"
            f"<li>gate_count: {metrics['gate_count']}</li>"
            f"<li>depth: {metrics['depth']}</li>"
            f"<li>swap_count: {metrics['swap_count']}</li>"
            f"<li>t_count: {metrics['t_count']}</li>"
            "</ul></section>"
        )
    event_rows = []
    for index, event in enumerate(trace["events"]):
        kind = html.escape(event["kind"])
        gate = html.escape(event.get("gate", "swap"))
        logical = html.escape(str(event["logical"]))
        physical = html.escape(str(event["physical"]))
        summary = html.escape(event["summary"])
        event_rows.append(
            "<tr>"
            f"<td>{index}</td><td>{kind}</td><td>{gate}</td>"
            f"<td>{logical}</td><td>{physical}</td><td>{summary}</td>"
            "</tr>"
        )
    if not event_rows:
        event_rows.append('<tr><td colspan="6">(no routing events)</td></tr>')
    title = html.escape(trace["meta"]["target_id"])
    summary = html.escape(trace["summary"])
    return f"""<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="utf-8">
<title>mapping trace — {title}</title>
<style>
body {{ font: 15px/1.45 sans-serif; margin: 1.5rem; color: #14202b; }}
h1 {{ font-size: 1.25rem; }}
table {{ border-collapse: collapse; width: 100%; }}
td, th {{ border: 1px solid #d5dde5; padding: 0.35rem 0.5rem; text-align: left; vertical-align: top; }}
th {{ background: #f4f7fa; }}
.swap {{ background: #fff4f6; }}
</style>
</head>
<body>
<h1>mapping_trace v{trace["schema_version"]} — {title}</h1>
<p>{summary}</p>
<label>Stage <select id="stage">{"".join(stage_options)}</select></label>
{"".join(stage_sections)}
<h2>Events</h2>
<table>
<thead><tr><th>#</th><th>kind</th><th>gate</th><th>logical</th><th>physical</th><th>summary</th></tr></thead>
<tbody>
{"".join(event_rows)}
</tbody>
</table>
<script>
const select = document.getElementById("stage");
const sections = document.querySelectorAll(".stage");
select.addEventListener("change", () => {{
  sections.forEach((section) => {{
    section.hidden = section.dataset.index !== select.value;
  }});
}});
</script>
</body>
</html>
"""


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Render a quonc mapping_trace JSON document (ASCII or HTML).",
    )
    parser.add_argument("trace", type=Path, help="mapping_trace JSON from quonc --emit-mapping-json")
    parser.add_argument("--ascii", action="store_true", help="print the ASCII timeline")
    parser.add_argument("--html", type=Path, default=None, help="write a self-contained HTML file")
    return parser


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    if not args.ascii and args.html is None:
        args.ascii = True
    try:
        data = json.loads(args.trace.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        _die(f"failed to read {args.trace}: {exc}")
    try:
        trace = load_trace(data)
    except ValueError as exc:
        _die(str(exc))
    if args.ascii:
        sys.stdout.write(render_ascii(trace))
    if args.html is not None:
        args.html.parent.mkdir(parents=True, exist_ok=True)
        args.html.write_text(render_html(trace), encoding="utf-8")
        print(args.html, file=sys.stderr)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
