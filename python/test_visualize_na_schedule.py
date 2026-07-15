"""Smoke tests for python/visualize_na_schedule.py (issue #113)."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SCRIPT = ROOT / "visualize_na_schedule.py"
FIXTURE_JSON = ROOT / "testdata" / "toy_na_schedule_view.json"
FIXTURE_DOT = ROOT / "testdata" / "toy_interaction_graph.dot"


def load_viz_module():
    spec = importlib.util.spec_from_file_location("visualize_na_schedule", SCRIPT)
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    sys.modules["visualize_na_schedule"] = mod
    spec.loader.exec_module(mod)
    return mod


@unittest.skipUnless(
    importlib.util.find_spec("matplotlib") is not None,
    "matplotlib not installed",
)
class ScheduleFrameTests(unittest.TestCase):
    def test_renders_cycle_frames(self) -> None:
        viz = load_viz_module()
        out_dir = ROOT / "testdata" / "_out"
        out_dir.mkdir(exist_ok=True)
        prefix = out_dir / "toy"
        view = viz.load_schedule_view(FIXTURE_JSON)
        paths = viz.render_schedule_frames(view, prefix, "svg")
        self.assertEqual(len(paths), 3)
        for path in paths:
            self.assertTrue(path.is_file(), path)
            self.assertGreater(path.stat().st_size, 100)


@unittest.skipUnless(
    importlib.util.find_spec("graphviz") is not None,
    "graphviz Python package not installed",
)
class GraphDotTests(unittest.TestCase):
    def test_renders_dot(self) -> None:
        viz = load_viz_module()
        out_dir = ROOT / "testdata" / "_out"
        out_dir.mkdir(exist_ok=True)
        try:
            path = viz.render_graph_dot(FIXTURE_DOT, out_dir / "toy-graph", "svg")
        except SystemExit as exc:
            # System `dot` may be missing in CI; treat as skip rather than fail.
            self.skipTest(f"graphviz render unavailable: {exc}")
        self.assertTrue(path.is_file(), path)
        self.assertGreater(path.stat().st_size, 50)


class SchemaGuardTests(unittest.TestCase):
    def test_rejects_wrong_kind(self) -> None:
        viz = load_viz_module()
        bad = ROOT / "testdata" / "_out" / "bad.json"
        bad.parent.mkdir(exist_ok=True)
        bad.write_text('{"schema_version": 1, "kind": "nope", "layers": [], "zones": []}', encoding="utf-8")
        with self.assertRaises(SystemExit):
            viz.load_schedule_view(bad)


if __name__ == "__main__":
    unittest.main()
