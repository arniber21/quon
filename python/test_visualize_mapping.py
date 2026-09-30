"""Smoke tests for python/visualize_mapping.py (issue #135)."""

from __future__ import annotations

import importlib.util
import json
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SCRIPT = ROOT / "visualize_mapping.py"
FIXTURE = ROOT / "testdata" / "toy_mapping_trace.json"
GOLDEN = ROOT / "testdata" / "toy_mapping_trace.ascii"
HTML_GOLDEN = ROOT / "testdata" / "toy_mapping_trace.html"


def load_viz_module():
    spec = importlib.util.spec_from_file_location("visualize_mapping", SCRIPT)
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    sys.modules["visualize_mapping"] = mod
    spec.loader.exec_module(mod)
    return mod


class MappingTraceTests(unittest.TestCase):
    def test_ascii_matches_golden(self) -> None:
        viz = load_viz_module()
        trace = viz.load_trace(json.loads(FIXTURE.read_text(encoding="utf-8")))
        self.assertEqual(viz.render_ascii(trace), GOLDEN.read_text(encoding="utf-8"))

    def test_unknown_field_is_rejected(self) -> None:
        viz = load_viz_module()
        data = json.loads(FIXTURE.read_text(encoding="utf-8"))
        data["extra"] = 1
        with self.assertRaises(ValueError) as caught:
            viz.load_trace(data)
        self.assertIn("unknown field", str(caught.exception))

    def test_wrong_kind_is_rejected(self) -> None:
        viz = load_viz_module()
        data = json.loads(FIXTURE.read_text(encoding="utf-8"))
        data["kind"] = "na_schedule_view"
        with self.assertRaises(ValueError) as caught:
            viz.load_trace(data)
        self.assertIn("kind", str(caught.exception))

    def test_html_matches_golden(self) -> None:
        viz = load_viz_module()
        trace = viz.load_trace(json.loads(FIXTURE.read_text(encoding="utf-8")))
        page = viz.render_html(trace)
        self.assertEqual(page, HTML_GOLDEN.read_text(encoding="utf-8"))
        self.assertIn("<select", page)
        self.assertNotIn("<script src=", page)


if __name__ == "__main__":
    unittest.main()
