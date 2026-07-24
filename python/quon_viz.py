#!/usr/bin/env python3
"""Results & plotting helpers for Quon (issue #196).

A small Python-first presentation layer sitting *next to* (not inside) the
compiler: it consumes the artifacts `quonc` already emits and the Aer counts
`python/quon_aer.py` already produces, and renders them for humans.

Two audiences:

- **Qiskit migrants** get `plot_histogram` and `plot_bloch`, which mirror the
  ergonomics of `qiskit.visualization.plot_histogram` / `plot_bloch_multivector`
  (same keyword shape) but are safe to call headless — they default to the
  non-interactive ``Agg`` backend and never block on ``plt.show()``.
- **NA compiler users** get `metrics_table`, `summarize_na_report`, and
  `summarize_na_schedule`, which pretty-print the JSON envelopes from
  `quonc --emit-resource-report` and `quonc --emit-na-schedule` into readable
  tables/timelines instead of raw JSON.

Every table/timeline function accepts its report three ways, in preference
order: a parsed ``dict``/``list``, a filesystem path to a JSON file, or a JSON
string. The plotting functions take the in-memory object directly (you would
not read a counts dict off disk in a sample).

Only optional dependencies: ``matplotlib`` for plots, ``qiskit`` (already
required by the Aer seam) for the optional Bloch sphere. Table/timeline
functions are pure stdlib and import nothing heavy.
"""

from __future__ import annotations

import json
import os
import sys
from pathlib import Path
from typing import Any, Iterable, Mapping

__all__ = [
    "plot_histogram",
    "metrics_table",
    "summarize_na_report",
    "summarize_na_schedule",
    "plot_bloch",
    "load_json",
]

# ---------------------------------------------------------------------------
# JSON intake — accept dict | path | str
# ---------------------------------------------------------------------------


def load_json(report: Any) -> Any:
    """Normalize a report argument to a parsed Python object.

    Accepts an already-parsed ``dict``/``list`` (returned as-is), a
    ``str``/``Path`` pointing at a JSON file (read and parsed), or a ``str``
    that is itself JSON (parsed). A bare non-JSON string raises ``ValueError``
    so a mistaken ``metrics_table("not a path")`` fails loudly rather than
    silently formatting a filename.
    """
    if isinstance(report, Mapping) or isinstance(report, list):
        return report
    if isinstance(report, Path):
        return json.loads(report.read_text(encoding="utf-8"))
    if isinstance(report, str):
        # A plausible filesystem path that exists → read it.
        if os.path.isfile(report):
            return json.loads(Path(report).read_text(encoding="utf-8"))
        # Otherwise try to parse it as a JSON document.
        try:
            return json.loads(report)
        except json.JSONDecodeError as exc:
            raise ValueError(
                f"load_json: argument is neither an existing JSON file nor "
                f"valid JSON text: {report!r} ({exc})"
            ) from exc
    raise TypeError(
        f"load_json: expected dict/list/Path/str, got {type(report).__name__}"
    )


# ---------------------------------------------------------------------------
# plot_histogram — Aer counts → bar chart (qiskit-familiar ergonomics)
# ---------------------------------------------------------------------------


def _ensure_agg() -> None:
    """Force the non-interactive Agg backend unless a GUI backend is already
    loaded and the caller is in an interactive session. Headless sample
    scripts must never block on ``plt.show()``."""
    import matplotlib

    if os.environ.get("MPLBACKEND"):
        return  # respect an explicit caller choice
    backend = matplotlib.get_backend()
    if backend.lower() == "agg":
        return
    # A non-Agg backend may already be running interactively; only override
    # when there is no active figure manager (i.e. no display attached).
    try:
        matplotlib.use("Agg", force=False)
    except Exception:  # noqa: BLE001 — never fatal; plotting is best-effort
        pass


def plot_histogram(
    counts: Mapping[str, float] | Mapping[str, int],
    *,
    figsize: tuple[float, float] | None = None,
    color: str | tuple[str, ...] | None = None,
    title: str | None = None,
    legend_keys: list[str] | None = None,
    bar_labels: bool = False,
    number_to_keep: int | None = None,
    sort: str | None = None,
    filename: str | os.PathLike[str] | None = None,
    dpi: int = 150,
) -> Any:
    """Render an Aer counts dict as a bar chart (mirrors qiskit ``plot_histogram``).

    Parameters mirror the qiskit names so a Qiskit user can drop this in:

    - ``counts`` — ``{"00": 2048, "11": 2048}`` (shots) or normalized probs.
    - ``figsize`` — ``(w, h)`` inches; default ``(max(7, 0.9 * n), 5)``.
    - ``color`` — single color or one per bitstring key.
    - ``title`` — axes title.
    - ``legend_keys`` — labels for an overlaid legend (use when plotting
      several distributions side by side; pass the same ``counts`` merged with
      ``legend_keys`` to label series — see the docs page for the recipe).
    - ``bar_labels`` — annotate each bar with its value.
    - ``number_to_keep`` — keep only the top-N most-probable bitstrings.
    - ``sort`` — ``"asc"`` / ``"desc"`` sort bars by value; ``None`` keeps
      insertion (qiskit default) order.
    - ``filename`` — if given, save the figure to this path (PNG unless the
      extension says otherwise) and close it. The function never calls
      ``plt.show()``, so it is safe to run headless.

    Returns the matplotlib ``Figure`` (or ``None`` if matplotlib is missing
    and ``filename`` was given — a missing dep is reported on stderr, not
    raised, so a sample script can still exit 0 on the parts that matter).
    """
    try:
        import matplotlib

        _ensure_agg()
        import matplotlib.pyplot as plt
    except ImportError as exc:
        msg = f"quon_viz.plot_histogram: matplotlib is required ({exc})"
        print(msg, file=sys.stderr)
        if filename is not None:
            print(
                f"quon_viz.plot_histogram: requested output {filename!r} not written",
                file=sys.stderr,
            )
        return None

    # Normalize to probabilities for a comparable y-axis, preserving keys.
    items: list[tuple[str, float]] = []
    total = 0.0
    for key, val in counts.items():
        v = float(val)
        items.append((str(key), v))
        total += v
    norm = [(k, (v / total if total > 0 else 0.0)) for k, v in items]

    if number_to_keep is not None and number_to_keep >= 0:
        norm = sorted(norm, key=lambda kv: kv[1], reverse=True)[:number_to_keep]
    if sort == "asc":
        norm.sort(key=lambda kv: kv[1])
    elif sort == "desc":
        norm.sort(key=lambda kv: kv[1], reverse=True)

    keys = [k for k, _ in norm]
    probs = [p for _, p in norm]
    n = len(keys)
    if n == 0:
        fig, ax = plt.subplots(figsize=(6, 4))
        ax.text(0.5, 0.5, "no counts", ha="center", va="center", transform=ax.transAxes)
        ax.set_axis_off()
        if title:
            ax.set_title(title)
        if filename is not None:
            fig.savefig(filename, dpi=dpi, bbox_inches="tight")
            plt.close(fig)
        return fig

    width = figsize[0] if figsize else max(7.0, 0.9 * n)
    height = figsize[1] if figsize else 5.0
    fig, ax = plt.subplots(figsize=(width, height))

    x = range(n)
    if color is None:
        bars = ax.bar(x, probs, width=0.7, edgecolor="#333333", linewidth=0.5)
    elif isinstance(color, str):
        bars = ax.bar(x, probs, width=0.7, color=color, edgecolor="#333333", linewidth=0.5)
    else:
        colors = list(color)
        bar_colors = [colors[i % len(colors)] for i in range(n)]
        bars = ax.bar(x, probs, width=0.7, color=bar_colors, edgecolor="#333333", linewidth=0.5)

    ax.set_xticks(list(x))
    ax.set_xticklabels(keys, rotation=45 if any(len(k) > 4 for k in keys) else 0, ha="right" if any(len(k) > 4 for k in keys) else "center")
    ax.set_ylabel("probability")
    ax.set_ylim(0, max(1.0, max(probs) * 1.12) if probs else 1.0)
    if title:
        ax.set_title(title)
    if bar_labels:
        for bar, p in zip(bars, probs):
            ax.text(
                bar.get_x() + bar.get_width() / 2,
                bar.get_height(),
                f"{p:.3f}",
                ha="center",
                va="bottom",
                fontsize=8,
            )
    if legend_keys:
        ax.legend(legend_keys)
    ax.grid(axis="y", alpha=0.25)
    fig.tight_layout()

    if filename is not None:
        Path(filename).parent.mkdir(parents=True, exist_ok=True)
        fig.savefig(filename, dpi=dpi, bbox_inches="tight")
        plt.close(fig)
        return None
    return fig


# ---------------------------------------------------------------------------
# metrics_table — generic pretty-printer for a flat/nested metrics dict
# ---------------------------------------------------------------------------


def _fmt_value(value: Any) -> str:
    if isinstance(value, bool):
        return str(value)
    if isinstance(value, (int,)):
        return f"{value:,}"
    if isinstance(value, float):
        if 0 < abs(value) < 1e-3 or abs(value) >= 1e6:
            return f"{value:.6g}"
        return f"{value:.4g}"
    if value is None:
        return "—"
    return str(value)


def _humanize(key: str) -> str:
    out = key.replace("_", " ").replace(".", " ")
    return out[0].upper() + out[1:] if out else out


def metrics_table(
    metrics: Any,
    *,
    title: str | None = None,
    file: Any = None,
    indent: int = 0,
) -> str:
    """Pretty-print a metrics dict as an aligned two-column table.

    ``metrics`` may be a parsed ``dict``, a path to JSON, or a JSON string
    (see :func:`load_json`). Top-level scalar values are printed as
    ``Metric ............ value``; nested ``dict`` values become an indented
    subsection. ``list`` values are rendered comma-separated (or, for short
    numeric series, inline). The rendered text is returned and, if ``file``
    is given (an open text stream), also written there.

    This is the generic formatter underneath :func:`summarize_na_report`; call
    it directly on any flat metrics dict (e.g. ``{"depth": 12, "swaps": 3}``).
    """
    data = load_json(metrics) if not isinstance(metrics, Mapping) else metrics
    lines: list[str] = []
    pad = "  " * indent
    if title:
        lines.append(f"{pad}{title}")
        lines.append(f"{pad}{'-' * max(len(title), 20)}")

    if not isinstance(data, Mapping):
        lines.append(f"{pad}{_fmt_value(data)}")
        text = "\n".join(lines)
        if file is not None:
            file.write(text + "\n")
        return text

    # Compute label width over scalars only (nested dicts get their own block).
    scalar_keys = [k for k, v in data.items() if not isinstance(v, Mapping)]
    label_w = max((_len_human(k) for k in scalar_keys), default=0)
    label_w = max(label_w, 12)

    for key, value in data.items():
        label = _humanize(key)
        if isinstance(value, Mapping):
            lines.append(f"{pad}{label}:")
            lines.append(metrics_table(value, indent=indent + 1))
        else:
            rendered = value if isinstance(value, list) else _fmt_value(value)
            if isinstance(rendered, list):
                rendered = ", ".join(_fmt_value(v) for v in rendered)
            lines.append(f"{pad}{label:<{label_w}}  {rendered}")

    text = "\n".join(lines)
    if file is not None:
        file.write(text + "\n")
    return text


def _len_human(key: str) -> int:
    return len(_humanize(key))


# ---------------------------------------------------------------------------
# summarize_na_report — pretty-print --emit-resource-report output
# ---------------------------------------------------------------------------


# Ordered metric groups for a readable report (order ≈ how the README's
# headline tables present them).
_REPORT_TOP_KEYS = [
    ("resource", ["rydberg_stages", "rearrangement_steps", "rearrangement_time_us",
                  "trap_transfers", "transfer_time_us", "entangle2_count",
                  "entangle_n_count", "measurement_rounds", "reset_rounds",
                  "wait_time_us", "total_time_us"]),
    ("qubits", ["logical_qubits", "physical_atoms", "estimated_cycles", "bottleneck"]),
    ("single-qubit gates", ["local_h_count", "local_rz_count", "local_u3_count",
                            "local_gate_time_us", "global_ry_count", "global_ry_time_us"]),
    ("fidelity", ["gate_fidelity_product", "estimated_fidelity"]),
]


def _row(label: str, value: Any, label_w: int) -> str:
    return f"  {label:<{label_w}}  {_fmt_value(value)}"


def summarize_na_report(
    report: Any,
    *,
    title: str = "NA resource report",
    file: Any = None,
) -> str:
    """Pretty-print a neutral-atom resource report as a readable summary.

    ``report`` is the JSON document emitted by ``quonc --emit-resource-report``
    (JSON form), accepted as a dict, a path, or a JSON string. The summary
    groups the flat fields into Resource / Qubits / Single-qubit gates /
    Fidelity blocks, then appends the ``error_budget`` and
    ``temporal_atom_metrics`` sub-objects, and the evidence disclaimer.
    """
    data = load_json(report)
    if not isinstance(data, Mapping):
        raise ValueError("summarize_na_report: report must be a JSON object")

    lines: list[str] = []
    lines.append(f"{title}")
    lines.append("=" * max(len(title), 24))

    meta = {k: v for k, v in data.items() if not isinstance(v, Mapping)}
    for heading, keys in _REPORT_TOP_KEYS:
        present = [(k, meta[k]) for k in keys if k in meta]
        if not present:
            continue
        lines.append("")
        lines.append(f"  {heading}")
        label_w = max(_len_human(k) for k, _ in present)
        for k, v in present:
            lines.append(_row(_humanize(k), v, label_w))

    for sub_key in ("error_budget", "temporal_atom_metrics"):
        sub = data.get(sub_key)
        if isinstance(sub, Mapping):
            lines.append("")
            lines.append(f"  {_humanize(sub_key)}")
            label_w = max(_len_human(k) for k in sub)
            for k, v in sub.items():
                rendered = v if not isinstance(v, list) else ", ".join(_fmt_value(x) for x in v)
                lines.append(_row(_humanize(k), rendered, label_w))

    disclaimer = data.get("evidence_disclaimer")
    if disclaimer:
        lines.append("")
        lines.append(f"  note: {disclaimer}")

    text = "\n".join(lines)
    if file is not None:
        file.write(text + "\n")
    return text


# ---------------------------------------------------------------------------
# summarize_na_schedule — compact timeline of --emit-na-schedule layers
# ---------------------------------------------------------------------------


def _action_summary(action: Mapping[str, Any]) -> str:
    """One compact token per action, e.g. ``Move(2)``, ``Entangle2(a0,a1)``."""
    if len(action) != 1:
        return "Unknown"
    tag, body = next(iter(action.items()))
    if tag == "Move":
        n = len(body.get("moves", [])) if isinstance(body, Mapping) else 0
        return f"Move({n})"
    if tag == "Transfer":
        if isinstance(body, Mapping):
            atom = body.get("atom", "?")
            direction = body.get("direction", "")
            return f"Transfer(a{atom},{direction})"
        return "Transfer"
    if tag in ("Entangle2", "EntangleN"):
        atoms = body.get("atoms", []) if isinstance(body, Mapping) else []
        joined = ",".join(f"a{a}" for a in atoms)
        return f"{tag}({joined})"
    if tag == "LocalGate":
        if isinstance(body, Mapping):
            atom = body.get("atom", "?")
            gate = body.get("gate", {})
            gname = next(iter(gate.keys()), "?") if isinstance(gate, Mapping) else "?"
            return f"{gname}(a{atom})"
        return "LocalGate"
    if tag == "GlobalRy":
        return "GlobalRy"
    if tag in ("Measure", "Reset", "Reuse", "Wait"):
        return tag
    return tag


def summarize_na_schedule(
    schedule: Any,
    *,
    title: str = "NA schedule timeline",
    max_layers: int | None = None,
    file: Any = None,
) -> str:
    """Render a compact per-cycle timeline from an ``na_schedule_view`` envelope.

    ``schedule`` is the JSON document emitted by ``quonc --emit-na-schedule``,
    accepted as a dict, a path, or a JSON string. One line per cycle::

        cycle 00 │ LocalRz(a0)              │ 1µs
        cycle 01 │ GlobalRy                  │ 1µs
        cycle 05 │ Transfer(a0,SlmToAod) ... │ 15µs

    A header summarizes zones and the headline metrics; ``max_layers`` truncates
    the per-cycle listing (the metrics block always reflects the full schedule).
    """
    data = load_json(schedule)
    if not isinstance(data, Mapping):
        raise ValueError("summarize_na_schedule: schedule must be a JSON object")
    layers = data.get("layers", []) or []
    zones = data.get("zones", []) or []
    metrics = data.get("metrics", {}) or {}
    meta = data.get("meta", {}) or {}

    lines: list[str] = []
    lines.append(f"{title}")
    lines.append("=" * max(len(title), 24))
    lines.append(
        f"  backend={meta.get('na_backend', '?')}  placer={meta.get('na_placer', '?')}  "
        f"target={meta.get('target_id', '?')}"
    )
    zone_kinds: dict[str, int] = {}
    for z in zones:
        kind = z.get("kind", "?") if isinstance(z, Mapping) else "?"
        zone_kinds[kind] = zone_kinds.get(kind, 0) + 1
    if zone_kinds:
        lines.append("  zones: " + ", ".join(f"{k}×{v}" for k, v in zone_kinds.items()))
    head = (
        f"  cycles={metrics.get('estimated_cycles', len(layers))}  "
        f"layers={len(layers)}  rydberg={metrics.get('rydberg_stages', '?')}  "
        f"rearr={metrics.get('rearrangement_steps', '?')}  "
        f"transfers={metrics.get('trap_transfers', '?')}  "
        f"total={metrics.get('total_time_us', '?')}µs"
    )
    lines.append(head)

    if layers:
        lines.append("")
        header = f"  {'cycle':<6} │ {'actions':<28} │ dur"
        lines.append(header)
        lines.append("  " + "-" * (len(header) - 2))
        shown = layers if max_layers is None else layers[:max_layers]
        for layer in shown:
            cycle = layer.get("cycle", "?")
            actions = layer.get("actions", []) or []
            summary = ", ".join(_action_summary(a) for a in actions) or "idle"
            if len(summary) > 28:
                summary = summary[:25] + "..."
            dur = layer.get("duration_us")
            # The envelope does not always carry a per-layer duration; sum
            # the per-action durations when present.
            if dur is None:
                d = 0
                for a in actions:
                    body = next(iter(a.values()), {}) if isinstance(a, Mapping) else {}
                    if isinstance(body, Mapping) and "duration_us" in body:
                        d += int(body["duration_us"])
                dur = d if d else ""
            dur_s = f"{dur}µs" if dur != "" else "—"
            lines.append(f"  {str(cycle):<6} │ {summary:<28} │ {dur_s}")
        if max_layers is not None and len(layers) > max_layers:
            lines.append(f"  … {len(layers) - max_layers} more cycle(s) (max_layers={max_layers})")

    text = "\n".join(lines)
    if file is not None:
        file.write(text + "\n")
    return text


# ---------------------------------------------------------------------------
# plot_bloch — optional Bloch sphere (qiskit-backed)
# ---------------------------------------------------------------------------


def plot_bloch(
    statevector: Any,
    *,
    figsize: tuple[float, float] | None = None,
    title: str | None = None,
    filename: str | os.PathLike[str] | None = None,
    dpi: int = 150,
) -> Any:
    """Plot single-qubit statevectors on the Bloch sphere (optional).

    Thin wrapper over ``qiskit.visualization.plot_bloch_multivector`` that is
    safe headless: it forces the ``Agg`` backend and, when ``filename`` is
    given, saves the figure and returns ``None`` instead of opening a window.
    ``qiskit`` (already a verification dependency) provides the renderer; this
    function is intentionally **optional** — if the visualization submodule is
    unavailable it prints a notice and returns ``None`` rather than failing.
    """
    try:
        import matplotlib

        _ensure_agg()
    except ImportError as exc:
        print(f"quon_viz.plot_bloch: matplotlib is required ({exc})", file=sys.stderr)
        return None
    try:
        from qiskit.visualization import plot_bloch_multivector
    except ImportError as exc:
        print(
            f"quon_viz.plot_bloch: qiskit.visualization is unavailable ({exc}); "
            "plot_bloch is an optional helper",
            file=sys.stderr,
        )
        return None

    kwargs: dict[str, Any] = {"figsize": figsize}
    if title is not None:
        kwargs["title"] = title
    fig = plot_bloch_multivector(statevector, **kwargs)
    if filename is not None:
        Path(filename).parent.mkdir(parents=True, exist_ok=True)
        fig.savefig(filename, dpi=dpi, bbox_inches="tight")
        import matplotlib.pyplot as plt

        plt.close(fig)
        return None
    return fig


# ---------------------------------------------------------------------------
# CLI entry — quick pretty-print of a report/schedule file
# ---------------------------------------------------------------------------


def _cli_main(argv: list[str] | None = None) -> int:
    import argparse

    p = argparse.ArgumentParser(
        prog="quon_viz",
        description="Pretty-print Quon neutral-atom report/schedule JSON.",
    )
    sub = p.add_subparsers(dest="cmd", required=True)
    pr = sub.add_parser("report", help="summarize --emit-resource-report JSON")
    pr.add_argument("path", help="path to the report JSON (or '-' for stdin)")
    ps = sub.add_parser("schedule", help="summarize --emit-na-schedule JSON")
    ps.add_argument("path", help="path to the schedule JSON (or '-' for stdin)")
    ps.add_argument("--max-layers", type=int, default=None)

    args = p.parse_args(argv)
    raw = sys.stdin.read() if args.path == "-" else Path(args.path).read_text(encoding="utf-8")
    if args.cmd == "report":
        print(summarize_na_report(json.loads(raw)))
    else:
        print(summarize_na_schedule(json.loads(raw), max_layers=args.max_layers))
    return 0


if __name__ == "__main__":
    sys.exit(_cli_main())
