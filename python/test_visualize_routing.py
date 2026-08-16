"""Smoke tests for python/visualize_routing.py (issue #135)."""

from __future__ import annotations

import importlib.util
import sys
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
SCRIPT = ROOT / "visualize_routing.py"
FIXTURE_QASM = ROOT / "testdata" / "toy_routing.qasm"
FIXTURE_TARGET = ROOT / "testdata" / "toy_routing_target.json"


def load_viz_module():
    spec = importlib.util.spec_from_file_location("visualize_routing", SCRIPT)
    assert spec is not None and spec.loader is not None
    mod = importlib.util.module_from_spec(spec)
    sys.modules["visualize_routing"] = mod
    spec.loader.exec_module(mod)
    return mod


class GateParsingTests(unittest.TestCase):
    def test_parses_two_qubit_gates_and_skips_single_qubit_and_measure(self) -> None:
        viz = load_viz_module()
        num_qubits, ops = viz.parse_qasm_gates(FIXTURE_QASM.read_text(encoding="utf-8"))
        self.assertEqual(num_qubits, 3)
        self.assertEqual(
            [(op.name, op.qubits) for op in ops],
            [
                ("cx", (0, 1)),
                ("cx", (1, 0)),
                ("cx", (0, 1)),
                ("cx", (1, 2)),
            ],
        )


class SwapDetectionTests(unittest.TestCase):
    def test_detects_literal_swap_triple_and_leaves_other_gates_as_interactions(self) -> None:
        viz = load_viz_module()
        _, ops = viz.parse_qasm_gates(FIXTURE_QASM.read_text(encoding="utf-8"))
        events = viz.detect_events(ops)
        self.assertEqual(
            [(e.kind, e.gate, e.a, e.b) for e in events],
            [
                ("swap", "swap", 0, 1),
                ("interaction", "cx", 1, 2),
            ],
        )

    def test_non_swap_pattern_stays_as_separate_interactions(self) -> None:
        viz = load_viz_module()
        ops = [
            viz.GateOp("cx", (0, 1), 1),
            viz.GateOp("cx", (1, 2), 2),
            viz.GateOp("cx", (0, 1), 3),
        ]
        events = viz.detect_events(ops)
        self.assertEqual([e.kind for e in events], ["interaction", "interaction", "interaction"])


class LayoutTests(unittest.TestCase):
    def test_line_topology_gets_path_layout(self) -> None:
        viz = load_viz_module()
        pos = viz.layout_for(3, [(0, 1), (1, 2)])
        self.assertTrue(all(y == 0.0 for _, y in pos.values()))
        self.assertLess(pos[0][0], pos[1][0])
        self.assertLess(pos[1][0], pos[2][0])

    def test_branching_topology_falls_back_to_circular(self) -> None:
        viz = load_viz_module()
        # A star (node 0 connected to 1, 2, 3) is not a simple path.
        pos = viz.layout_for(4, [(0, 1), (0, 2), (0, 3)])
        self.assertFalse(all(y == 0.0 for _, y in pos.values()))


@unittest.skipUnless(
    importlib.util.find_spec("matplotlib") is not None,
    "matplotlib not installed",
)
class RenderTests(unittest.TestCase):
    def test_renders_one_frame_per_event(self) -> None:
        viz = load_viz_module()
        target = viz.load_target(FIXTURE_TARGET)
        edges = [(int(a), int(b)) for a, b in target["topology"]["edges"]]
        _, ops = viz.parse_qasm_gates(FIXTURE_QASM.read_text(encoding="utf-8"))
        events = viz.detect_events(ops)

        out_dir = ROOT / "testdata" / "_out"
        out_dir.mkdir(exist_ok=True)
        paths = viz.render(3, edges, events, target["id"], out_dir / "toy_routing", "svg")

        self.assertEqual(len(paths), len(events))
        for path in paths:
            self.assertTrue(path.is_file(), path)
            self.assertGreater(path.stat().st_size, 100)


if __name__ == "__main__":
    unittest.main()
