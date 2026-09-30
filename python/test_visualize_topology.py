"""Smoke tests for python/visualize_topology.py (issue #136)."""

from __future__ import annotations

import importlib.util
import json
import subprocess
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent
SCRIPT = ROOT / "visualize_topology.py"
DEVICE = REPO / "backend" / "tests" / "fixtures" / "device_5q.json"
TRACE = ROOT / "testdata" / "toy_mapping_trace.json"
BAD = ROOT / "testdata" / "bad_routing_mapping_trace.json"
ASCII_GOLDEN = ROOT / "testdata" / "device_5q_mapping.ascii"
HTML_GOLDEN = ROOT / "testdata" / "device_5q_mapping.html"


def load_viz_module():
    spec = importlib.util.spec_from_file_location("visualize_topology", SCRIPT)
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    sys.modules["visualize_topology"] = mod
    spec.loader.exec_module(mod)
    return mod


class TopologyViewerTests(unittest.TestCase):
    def test_ascii_overlays_mapping_on_device_5q(self) -> None:
        viz = load_viz_module()
        device = viz.load_device(json.loads(DEVICE.read_text(encoding="utf-8")))
        trace = viz.load_trace(json.loads(TRACE.read_text(encoding="utf-8")))
        rendered = viz.render_ascii(device, trace)
        self.assertEqual(rendered, ASCII_GOLDEN.read_text(encoding="utf-8"))
        self.assertIn("q0[L0] — q1[L2]", rendered)
        self.assertIn("q0[L0] — q2[L1]", rendered)
        self.assertIn("validation: ok", rendered)
        self.assertEqual(viz.validate_routing(device, trace), [])

    def test_html_draws_coupling_graph(self) -> None:
        viz = load_viz_module()
        device = viz.load_device(json.loads(DEVICE.read_text(encoding="utf-8")))
        trace = viz.load_trace(json.loads(TRACE.read_text(encoding="utf-8")))
        page = viz.render_html(device, trace)
        self.assertEqual(page, HTML_GOLDEN.read_text(encoding="utf-8"))
        self.assertIn("<svg", page)
        self.assertIn("q0 L0", page)
        self.assertIn("q1 L2", page)
        self.assertNotIn("<script src=", page)

    def test_bad_routing_fixture_fails_with_readable_text(self) -> None:
        completed = subprocess.run(
            [sys.executable, str(SCRIPT), str(DEVICE), str(BAD), "--validate"],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("validation failed", completed.stderr)
        self.assertIn(
            "interaction cx on physical qubits (0, 3) is not a coupling edge of my_device",
            completed.stderr,
        )
        self.assertNotIn("validation ok", completed.stdout)

    def test_cli_ascii_matches_golden(self) -> None:
        completed = subprocess.run(
            [sys.executable, str(SCRIPT), str(DEVICE), str(TRACE), "--ascii"],
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(completed.returncode, 0, completed.stderr)
        self.assertEqual(completed.stdout, ASCII_GOLDEN.read_text(encoding="utf-8"))


if __name__ == "__main__":
    unittest.main()
