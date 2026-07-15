#!/usr/bin/env python3
"""Unit + CI smoke tests for the QEC benchmark ablation harness (#254 / ADR-0023).

Run:  python -m unittest python/test_quon_qec_benchmarks.py
"""

from __future__ import annotations

import csv
import io
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

try:
    import pymatching  # noqa: F401
    import sinter  # noqa: F401
    import stim  # noqa: F401

    HAS_STIM_STACK = True
except ImportError:
    HAS_STIM_STACK = False

# Fail hard under CI if the Stim stack is missing (do not skip green).
if not HAS_STIM_STACK and os.environ.get("CI"):
    raise ImportError(
        "stim/sinter/pymatching required in CI for #254 benchmark tests "
        "(python/requirements.txt / just setup-python / ADR-0022)"
    )

import quon_qec_benchmarks as bench  # noqa: E402
import quon_qec_sinter as sinter_harness  # noqa: E402

REPO_ROOT = Path(__file__).resolve().parents[1]
TESTDATA = Path(__file__).resolve().parent / "testdata"
PINNED_REP_D3_R2_JSON = TESTDATA / "qec_rep_d3_r2.qec.json"


def _fake_resource_report(**overrides) -> dict:
    base = {
        "evidence_kind": "analytic",
        "evidence_disclaimer": "analytic only",
        "estimated_cycles": 12,
        "rydberg_stages": 4,
        "rearrangement_steps": 3,
        "rearrangement_time_us": 150,
        "trap_transfers": 2,
        "transfer_time_us": 30,
        "measurement_rounds": 2,
        "reset_rounds": 2,
        "physical_atoms": 5,
        "logical_qubits": 1,
        "distance": 3,
        "memory_rounds": 2,
        "bottleneck": "rydberg",
        "error_budget": {
            "rydberg": 0.008,
            "measurement": 0.04,
            "reset": 0.06,
            "movement": 0.012,
            "transfer": 0.01,
            "idle": 1e-6,
        },
    }
    base.update(overrides)
    return base


class CsvShapeTests(unittest.TestCase):
    def test_csv_columns_match_locked_contract(self) -> None:
        required = {
            "experiment_class",
            "methodology_anchor",
            "workload",
            "na_placer",
            "na_backend",
            "na_compact",
            "estimated_cycles",
            "rydberg_stages",
            "rearrangement_time_us",
            "trap_transfers",
            "measurement_rounds",
            "physical_atoms",
            "error_budget_rydberg",
            "error_budget_measurement",
            "error_budget_reset",
            "error_budget_movement",
            "error_budget_transfer",
            "error_budget_idle",
            "logical_failures",
            "logical_failure_rate",
            "evidence_kind_analytic",
            "evidence_kind_sampled",
        }
        self.assertTrue(required.issubset(set(bench.CSV_COLUMNS)))

    def test_write_csv_emits_banner_and_row(self) -> None:
        row = bench.BenchmarkRow(
            workload="repetition_d3_memory",
            source="examples/na_qec/repetition_d3_memory.qn",
            na_placer="routing-agnostic",
            na_backend="zoned",
            na_compact=True,
            distance=3,
            memory_rounds=2,
            shots=32,
            seed=7,
            estimated_cycles=12,
            rydberg_stages=4,
            rearrangement_steps=3,
            rearrangement_time_us=150,
            trap_transfers=2,
            transfer_time_us=30,
            measurement_rounds=2,
            reset_rounds=2,
            physical_atoms=5,
            logical_qubits=1,
            bottleneck="rydberg",
            error_budget={
                "rydberg": 0.008,
                "measurement": 0.04,
                "reset": 0.06,
                "movement": 0.012,
                "transfer": 0.01,
                "idle": 1e-6,
            },
            logical_failures=1,
            logical_failure_rate=1 / 32,
        )
        buf = io.StringIO()
        bench.write_csv(buf, [row])
        text = buf.getvalue()
        self.assertIn("experiment_class: qec_compiler_ablation", text)
        self.assertIn("#111", text)
        self.assertIn("not a threshold claim", text.lower())
        # Skip comment banner lines for DictReader.
        lines = [ln for ln in text.splitlines() if not ln.startswith("#")]
        records = list(csv.DictReader(lines))
        self.assertEqual(len(records), 1)
        rec = records[0]
        self.assertEqual(rec["experiment_class"], bench.EXPERIMENT_CLASS)
        self.assertEqual(rec["evidence_kind_analytic"], "analytic")
        self.assertEqual(rec["evidence_kind_sampled"], "sampled")
        self.assertEqual(rec["estimated_cycles"], "12")
        self.assertEqual(rec["logical_failures"], "1")
        self.assertEqual(rec["na_compact"], "true")


class GridTests(unittest.TestCase):
    def test_smoke_grid_is_single_cell(self) -> None:
        cells = bench.expand_grid(mode="smoke")
        self.assertEqual(len(cells), 1)
        cell = cells[0]
        self.assertEqual(cell.workload, "repetition_d3_memory")
        self.assertEqual(cell.na_backend, "zoned")
        self.assertEqual(cell.na_placer, "routing-agnostic")
        self.assertTrue(cell.na_compact)

    def test_full_grid_covers_workloads_and_ablations(self) -> None:
        cells = bench.expand_grid(mode="full")
        workloads = {c.workload for c in cells}
        self.assertIn("repetition_d3_memory", workloads)
        self.assertIn("surface_d3_memory", workloads)
        self.assertIn("surface_d3_cx", workloads)
        backends = {c.na_backend for c in cells}
        placers = {c.na_placer for c in cells}
        compact = {c.na_compact for c in cells}
        self.assertEqual(backends, {"zoned", "flat"})
        self.assertEqual(placers, {"routing-agnostic", "routing-aware"})
        self.assertEqual(compact, {True, False})
        # 3 workloads × 2 × 2 × 2
        self.assertEqual(len(cells), 24)


class JoinRowTests(unittest.TestCase):
    def test_join_report_and_sample(self) -> None:
        cell = bench.GridCell(
            workload="repetition_d3_memory",
            source=REPO_ROOT / "examples/na_qec/repetition_d3_memory.qn",
            na_placer="routing-agnostic",
            na_backend="zoned",
            na_compact=True,
        )
        report = _fake_resource_report()
        sample = sinter_harness.SampleResult(shots=32, logical_failures=2)
        row = bench.join_cell(cell, report=report, sample=sample, seed=7)
        self.assertEqual(row.estimated_cycles, 12)
        self.assertEqual(row.rearrangement_time_us, 150)
        self.assertEqual(row.trap_transfers, 2)
        self.assertEqual(row.physical_atoms, 5)
        self.assertEqual(row.logical_failures, 2)
        self.assertEqual(row.logical_failure_rate, 2 / 32)
        self.assertEqual(row.error_budget["rydberg"], 0.008)


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching")
class HelpAndCliTests(unittest.TestCase):
    def test_help_documents_ci_smoke_and_local_full(self) -> None:
        parser = bench.build_arg_parser()
        help_text = parser.format_help()
        self.assertIn("--mode", help_text)
        self.assertIn("smoke", help_text)
        self.assertIn("full", help_text)
        self.assertIn("CI", help_text)
        self.assertIn("#111", help_text)
        self.assertIn("threshold", help_text.lower())


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching")
class SmokeWithoutQuoncTests(unittest.TestCase):
    """CSV pipeline smoke using pinned dual-emit + fake report (no quonc)."""

    def test_run_cell_from_artifacts(self) -> None:
        report = _fake_resource_report()
        cell = bench.GridCell(
            workload="repetition_d3_memory",
            source=REPO_ROOT / "examples/na_qec/repetition_d3_memory.qn",
            na_placer="routing-agnostic",
            na_backend="zoned",
            na_compact=True,
        )
        with tempfile.TemporaryDirectory() as tmp:
            out_dir = Path(tmp)
            report_path = out_dir / "report.json"
            report_path.write_text(json.dumps(report))
            # Reuse pinned experiment pair by copying into the workdir.
            import shutil

            exp_json = out_dir / "cell.qec.json"
            exp_stim = out_dir / "cell.stim"
            shutil.copy(PINNED_REP_D3_R2_JSON, exp_json)
            shutil.copy(
                PINNED_REP_D3_R2_JSON.with_name("qec_rep_d3_r2.stim"),
                exp_stim,
            )
            # Fix stim_file field to sibling name.
            doc = json.loads(exp_json.read_text())
            doc["stim_file"] = "cell.stim"
            exp_json.write_text(json.dumps(doc))

            row = bench.evaluate_artifacts(
                cell,
                report_path=report_path,
                experiment_json=exp_json,
                shots=16,
                seed=7,
            )
            self.assertEqual(row.shots, 16)
            self.assertGreaterEqual(row.logical_failures, 0)
            self.assertEqual(row.physical_atoms, 5)
            buf = io.StringIO()
            bench.write_csv(buf, [row])
            lines = [ln for ln in buf.getvalue().splitlines() if not ln.startswith("#")]
            records = list(csv.DictReader(lines))
            self.assertEqual(set(records[0].keys()), set(bench.CSV_COLUMNS))


def _resolve_quonc_for_ci() -> Path | None:
    return bench.resolve_quonc(REPO_ROOT)


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching")
class CiSmokeIntegrationTests(unittest.TestCase):
    """One-cell compile+Sinter smoke when quonc is available (required under CI)."""

    def test_smoke_mode_compiles_and_writes_csv(self) -> None:
        quonc = _resolve_quonc_for_ci()
        if quonc is None:
            if os.environ.get("CI"):
                self.fail(
                    "quonc required for #254 CI smoke "
                    "(build target/release/quonc or set QUONC)"
                )
            self.skipTest("quonc not built; skip local integration smoke")

        target = REPO_ROOT / "targets/neutral_atom/generic_rna_v0.json"
        self.assertTrue(target.is_file())
        with tempfile.TemporaryDirectory() as tmp:
            csv_path = Path(tmp) / "smoke.csv"
            code = bench.main(
                [
                    "--mode",
                    "smoke",
                    "--repo-root",
                    str(REPO_ROOT),
                    "--quonc",
                    str(quonc),
                    "--target",
                    str(target),
                    "--shots",
                    "16",
                    "--seed",
                    "7",
                    "--csv",
                    str(csv_path),
                    "--work-dir",
                    tmp,
                ]
            )
            self.assertEqual(code, 0)
            self.assertTrue(csv_path.is_file())
            text = csv_path.read_text()
            self.assertIn(bench.EXPERIMENT_CLASS, text)
            lines = [ln for ln in text.splitlines() if not ln.startswith("#")]
            records = list(csv.DictReader(lines))
            self.assertEqual(len(records), 1)
            rec = records[0]
            self.assertEqual(rec["workload"], "repetition_d3_memory")
            self.assertEqual(rec["evidence_kind_analytic"], "analytic")
            self.assertEqual(rec["evidence_kind_sampled"], "sampled")
            self.assertGreater(int(rec["estimated_cycles"]), 0)
            self.assertGreater(int(rec["physical_atoms"]), 0)
            self.assertIn("logical_failures", rec)
            self.assertIn("logical_failure_rate", rec)


if __name__ == "__main__":
    unittest.main()
