#!/usr/bin/env python3
"""Visualize neutral-atom schedule JSON / interaction-graph DOT (issue #113).

Schedule JSON is the debug/visualization envelope from
``quonc --emit-na-schedule`` (``kind: na_schedule_view``). Graphviz DOT comes
from ``quonc --emit-na-graph``.

Examples::

  python/visualize_na_schedule.py schedule.json -o /tmp/bell --format svg
  python/visualize_na_schedule.py --graph graph.dot -o /tmp/bell-graph --format svg

Before/after comparison is deferred; ``meta.na_placer`` / ``meta.na_backend``
are present so a future ``--compare a.json b.json`` can share axes.
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any

SUPPORTED_SCHEMA_VERSION = 1
EXPECTED_KIND = "na_schedule_view"

# Zone face colors (light fills; edges drawn darker).
ZONE_COLORS = {
    "storage": "#cfe8ff",
    "entanglement": "#ffe0b2",
    "readout": "#d7f5d7",
}

# Distinct colors for simultaneous Move groups within a cycle.
MOVE_GROUP_COLORS = [
    "#d62728",
    "#1f77b4",
    "#2ca02c",
    "#9467bd",
    "#ff7f0e",
    "#8c564b",
]


def _die(msg: str, code: int = 1) -> None:
    print(f"visualize_na_schedule: {msg}", file=sys.stderr)
    raise SystemExit(code)


def load_schedule_view(path: Path) -> dict[str, Any]:
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        _die(f"failed to read schedule JSON {path}: {exc}")

    if not isinstance(data, dict):
        _die("schedule JSON must be an object (na_schedule_view envelope)")

    kind = data.get("kind")
    version = data.get("schema_version")
    if kind != EXPECTED_KIND:
        _die(f"expected kind={EXPECTED_KIND!r}, got {kind!r}")
    if version != SUPPORTED_SCHEMA_VERSION:
        _die(
            f"unsupported schema_version {version!r} "
            f"(supported major={SUPPORTED_SCHEMA_VERSION})"
        )
    if "layers" not in data or "zones" not in data:
        _die("schedule view missing required fields: layers, zones")
    return data


def site_positions(layout: dict[str, Any] | None) -> dict[int, tuple[float, float]]:
    if not layout:
        return {}
    out: dict[int, tuple[float, float]] = {}
    for site in layout.get("sites", []):
        sid = int(site["id"])
        pos = site["position"]
        out[sid] = (float(pos["x_um"]), float(pos["y_um"]))
    return out


def trap_site(trap: dict[str, Any]) -> int | None:
    if "Slm" in trap:
        return int(trap["Slm"]["site"])
    if "Aod" in trap:
        return int(trap["Aod"]["site"])
    return None


def initial_atom_sites(layout: dict[str, Any] | None) -> dict[int, int]:
    if not layout:
        return {}
    binding_map: dict[int, int] = {}
    for binding in layout.get("initial_bindings", []):
        atom = int(binding["atom"])
        site = trap_site(binding["trap"])
        if site is not None:
            binding_map[atom] = site
    return binding_map


def action_tag(action: dict[str, Any]) -> str:
    if len(action) != 1:
        return "Unknown"
    return next(iter(action.keys()))


def replay_through_cycle(
    atom_sites: dict[int, int],
    layers: list[dict[str, Any]],
    through_cycle: int,
) -> dict[int, int]:
    """Return atom→site after applying layers with ``cycle <= through_cycle``."""
    state = dict(atom_sites)
    for layer in layers:
        if int(layer["cycle"]) > through_cycle:
            break
        for action in layer.get("actions", []):
            tag = action_tag(action)
            body = action[tag]
            if tag == "Move":
                for move in body.get("moves", []):
                    state[int(move["atom"])] = int(move["to"])
            elif tag == "Transfer":
                state[int(body["atom"])] = int(body["site"])
    return state


def frame_title(view: dict[str, Any], cycle: int, tags: list[str]) -> str:
    meta = view.get("meta", {})
    metrics = view.get("metrics", {})
    placer = meta.get("na_placer", "?")
    backend = meta.get("na_backend", "?")
    stages = metrics.get("rydberg_stages", "?")
    rearr = metrics.get("rearrangement_steps", "?")
    total = metrics.get("total_time_us", "?")
    tag_s = ",".join(tags) if tags else "idle"
    return (
        f"cycle {cycle} [{tag_s}]  |  {backend}/{placer}  |  "
        f"rydberg={stages} rearr={rearr} total_us={total}"
    )


def draw_zones(ax: Any, zones: list[dict[str, Any]]) -> None:
    import matplotlib.patches as mpatches

    for zone in zones:
        kind = zone.get("kind", "storage")
        x0, y0 = zone["origin_um"]
        w = float(zone["width_um"])
        h = float(zone["height_um"])
        color = ZONE_COLORS.get(kind, "#eeeeee")
        rect = mpatches.Rectangle(
            (x0, y0),
            w,
            h,
            linewidth=1.0,
            edgecolor="#444444",
            facecolor=color,
            alpha=0.55,
            label=kind,
        )
        ax.add_patch(rect)
        ax.text(
            x0 + 0.02 * max(w, 1.0),
            y0 + 0.02 * max(h, 1.0),
            kind,
            fontsize=8,
            color="#333333",
            va="bottom",
        )


def axis_limits(
    zones: list[dict[str, Any]], points: list[tuple[float, float]]
) -> tuple[float, float, float, float] | None:
    xs: list[float] = []
    ys: list[float] = []
    for zone in zones:
        x0, y0 = zone["origin_um"]
        xs.extend([float(x0), float(x0) + float(zone["width_um"])])
        ys.extend([float(y0), float(y0) + float(zone["height_um"])])
    for x, y in points:
        xs.append(x)
        ys.append(y)
    if not xs or not ys:
        return None
    pad_x = max((max(xs) - min(xs)) * 0.05, 5.0)
    pad_y = max((max(ys) - min(ys)) * 0.05, 5.0)
    return (min(xs) - pad_x, max(xs) + pad_x, min(ys) - pad_y, max(ys) + pad_y)


def render_schedule_frames(
    view: dict[str, Any],
    out_prefix: Path,
    fmt: str,
) -> list[Path]:
    try:
        import matplotlib

        matplotlib.use("Agg")
        import matplotlib.pyplot as plt
    except ImportError as exc:
        _die(f"matplotlib is required for schedule frames ({exc})")

    layout = view.get("layout")
    sites = site_positions(layout)
    atom_sites0 = initial_atom_sites(layout)
    zones = view.get("zones", [])
    layers = view.get("layers", [])
    written: list[Path] = []

    if not layers:
        _die("schedule has no layers to render")

    for layer in layers:
        cycle = int(layer["cycle"])
        actions = layer.get("actions", [])
        tags = [action_tag(a) for a in actions]
        # Positions at the *start* of this cycle (before applying its actions).
        prev_cycle = cycle - 1
        state_before = (
            replay_through_cycle(atom_sites0, layers, prev_cycle)
            if prev_cycle >= 0
            else dict(atom_sites0)
        )
        state_after = replay_through_cycle(atom_sites0, layers, cycle)

        fig, ax = plt.subplots(figsize=(8, 8))
        draw_zones(ax, zones)

        # Atom markers after the cycle (final placement for this frame).
        points: list[tuple[float, float]] = []
        for atom, site in sorted(state_after.items()):
            pos = sites.get(site)
            if pos is None:
                continue
            points.append(pos)
            ax.scatter(
                [pos[0]],
                [pos[1]],
                s=60,
                c="#111111",
                zorder=5,
            )
            ax.annotate(
                f"a{atom}",
                pos,
                textcoords="offset points",
                xytext=(4, 4),
                fontsize=8,
            )

        # Move arrows (group-colored) and entangle highlights.
        move_group_idx = 0
        for action in actions:
            tag = action_tag(action)
            body = action[tag]
            if tag == "Move":
                color = MOVE_GROUP_COLORS[move_group_idx % len(MOVE_GROUP_COLORS)]
                move_group_idx += 1
                for move in body.get("moves", []):
                    frm = sites.get(int(move["from"]))
                    to = sites.get(int(move["to"]))
                    if frm is None or to is None:
                        continue
                    ax.annotate(
                        "",
                        xy=to,
                        xytext=frm,
                        arrowprops=dict(arrowstyle="->", color=color, lw=1.8),
                        zorder=4,
                    )
            elif tag in ("Entangle2", "EntangleN"):
                atoms = body.get("atoms", [])
                coords = []
                for atom in atoms:
                    site = state_after.get(int(atom))
                    if site is None:
                        continue
                    pos = sites.get(site)
                    if pos is not None:
                        coords.append(pos)
                if len(coords) >= 2:
                    xs = [c[0] for c in coords]
                    ys = [c[1] for c in coords]
                    ax.plot(xs + [xs[0]], ys + [ys[0]], color="#e91e63", lw=2.5, zorder=6)
                    ax.scatter(xs, ys, s=120, facecolors="none", edgecolors="#e91e63", lw=2, zorder=6)
            elif tag == "Transfer":
                atom = int(body["atom"])
                before = state_before.get(atom)
                after = state_after.get(atom)
                if before is None or after is None:
                    continue
                frm = sites.get(before)
                to = sites.get(after)
                if frm is None or to is None or frm == to:
                    # Same site (SLM↔AOD): mark with a diamond.
                    if to is not None:
                        ax.scatter(
                            [to[0]],
                            [to[1]],
                            s=90,
                            marker="D",
                            c="#0288d1",
                            zorder=5,
                        )
                    continue
                ax.annotate(
                    "",
                    xy=to,
                    xytext=frm,
                    arrowprops=dict(
                        arrowstyle="->",
                        color="#0288d1",
                        lw=1.4,
                        linestyle="dashed",
                    ),
                    zorder=4,
                )

        limits = axis_limits(zones, points)
        if limits is not None:
            ax.set_xlim(limits[0], limits[1])
            ax.set_ylim(limits[2], limits[3])
        ax.set_aspect("equal", adjustable="box")
        ax.set_xlabel("x (µm)")
        ax.set_ylabel("y (µm)")
        ax.set_title(frame_title(view, cycle, tags), fontsize=10)
        ax.grid(True, alpha=0.25)

        out_path = Path(f"{out_prefix}-cycle-{cycle:03d}.{fmt}")
        out_path.parent.mkdir(parents=True, exist_ok=True)
        fig.tight_layout()
        fig.savefig(out_path, dpi=140)
        plt.close(fig)
        written.append(out_path)

    return written


def render_graph_dot(dot_path: Path, out_prefix: Path, fmt: str) -> Path:
    try:
        import graphviz
    except ImportError as exc:
        _die(f"graphviz Python package is required for --graph ({exc})")

    try:
        source = dot_path.read_text(encoding="utf-8")
    except OSError as exc:
        _die(f"failed to read DOT file {dot_path}: {exc}")

    out_prefix.parent.mkdir(parents=True, exist_ok=True)
    # graphviz.Source.render writes OUT_PREFIX.FMT
    try:
        src = graphviz.Source(source)
        rendered = src.render(
            filename=str(out_prefix),
            format=fmt,
            cleanup=True,
        )
    except graphviz.ExecutableNotFound as exc:
        _die(
            f"system `dot` executable not found ({exc}); "
            "install Graphviz (e.g. brew install graphviz)"
        )
    except Exception as exc:  # noqa: BLE001 — surface render failures cleanly
        _die(f"graphviz render failed: {exc}")

    return Path(rendered)


def build_parser() -> argparse.ArgumentParser:
    p = argparse.ArgumentParser(
        description="Render NA schedule frames (matplotlib) or interaction-graph DOT (Graphviz).",
    )
    p.add_argument(
        "schedule",
        nargs="?",
        type=Path,
        help="na_schedule_view JSON from quonc --emit-na-schedule",
    )
    p.add_argument(
        "--graph",
        type=Path,
        metavar="DOT",
        help="Interaction-graph DOT from quonc --emit-na-graph",
    )
    p.add_argument(
        "-o",
        "--out",
        type=Path,
        default=Path("na-viz"),
        help="Output prefix (default: na-viz)",
    )
    p.add_argument(
        "--format",
        choices=("png", "svg"),
        default="svg",
        help="Image format (default: svg)",
    )
    return p


def main(argv: list[str] | None = None) -> int:
    args = build_parser().parse_args(argv)

    if args.graph is None and args.schedule is None:
        _die("provide SCHEDULE.json and/or --graph DOT")

    if args.schedule is not None:
        view = load_schedule_view(args.schedule)
        paths = render_schedule_frames(view, args.out, args.format)
        for path in paths:
            print(path)

    if args.graph is not None:
        path = render_graph_dot(args.graph, Path(f"{args.out}-graph"), args.format)
        print(path)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
