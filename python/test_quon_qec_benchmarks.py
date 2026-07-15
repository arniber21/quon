#!/usr/bin/env python3
"""Unit + CI smoke / axis-coverage tests for the QEC benchmark harness (#254).

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
from unittest import mock

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
        self.assertIn("schedule-agnostic", text.lower())
        self.assertIn("ADR-0024", text)
        self.assertIn("separate", text.lower())
        self.assertIn("placer no-ops", text.lower())
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

    def test_axis_grid_covers_each_ablation_axis_and_cx(self) -> None:
        cells = bench.expand_grid(mode="axis", repo_root=REPO_ROOT)
        self.assertEqual(len(cells), 4)
        backends = {c.na_backend for c in cells}
        placers = {c.na_placer for c in cells}
        compact = {c.na_compact for c in cells}
        workloads = {c.workload for c in cells}
        self.assertEqual(backends, {"zoned"})
        self.assertEqual(placers, {"routing-agnostic", "routing-aware"})
        self.assertEqual(compact, {True, False})
        self.assertIn("repetition_d3_memory", workloads)
        self.assertIn("surface_d3_cx", workloads)

    def test_full_grid_covers_workloads_and_ablations(self) -> None:
        cells = bench.expand_grid(mode="full")
        workloads = {c.workload for c in cells}
        self.assertIn("repetition_d3_memory", workloads)
        self.assertIn("surface_d3_memory", workloads)
        self.assertIn("surface_d3_cx", workloads)
        backends = {c.na_backend for c in cells}
        placers = {c.na_placer for c in cells}
        compact = {c.na_compact for c in cells}
        self.assertEqual(backends, {"zoned"})
        self.assertEqual(placers, {"routing-agnostic", "routing-aware"})
        self.assertEqual(compact, {True, False})
        # 3 workloads × 2 placers × 1 backend × 2 compact
        self.assertEqual(len(cells), 12)

    def test_include_flat_extends_full_grid(self) -> None:
        cells = bench.expand_grid(mode="full", include_flat=True)
        backends = {c.na_backend for c in cells}
        self.assertEqual(backends, {"zoned", "flat"})
        # 3 × 2 × 2 × 2
        self.assertEqual(len(cells), 24)
        # flat×placer cells are still enumerated (placer no-ops on flat path).
        flat_placers = {c.na_placer for c in cells if c.na_backend == "flat"}
        self.assertEqual(flat_placers, {"routing-agnostic", "routing-aware"})


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

    def test_missing_required_int_hard_fails(self) -> None:
        cell = bench.GridCell(
            workload="repetition_d3_memory",
            source=REPO_ROOT / "examples/na_qec/repetition_d3_memory.qn",
            na_placer="routing-agnostic",
            na_backend="zoned",
            na_compact=True,
        )
        report = _fake_resource_report()
        del report["physical_atoms"]
        sample = sinter_harness.SampleResult(shots=16, logical_failures=0)
        with self.assertRaises(bench.BenchmarkError) as ctx:
            bench.join_cell(cell, report=report, sample=sample, seed=7)
        self.assertIn("physical_atoms", str(ctx.exception))
        self.assertIn("missing", str(ctx.exception).lower())

    def test_missing_error_budget_key_hard_fails(self) -> None:
        cell = bench.GridCell(
            workload="repetition_d3_memory",
            source=REPO_ROOT / "examples/na_qec/repetition_d3_memory.qn",
            na_placer="routing-agnostic",
            na_backend="zoned",
            na_compact=True,
        )
        report = _fake_resource_report()
        del report["error_budget"]["idle"]
        sample = sinter_harness.SampleResult(shots=16, logical_failures=0)
        with self.assertRaises(bench.BenchmarkError) as ctx:
            bench.join_cell(cell, report=report, sample=sample, seed=7)
        self.assertIn("idle", str(ctx.exception))

    def test_missing_error_budget_object_hard_fails(self) -> None:
        cell = bench.GridCell(
            workload="repetition_d3_memory",
            source=REPO_ROOT / "examples/na_qec/repetition_d3_memory.qn",
            na_placer="routing-agnostic",
            na_backend="zoned",
            na_compact=True,
        )
        report = _fake_resource_report()
        del report["error_budget"]
        sample = sinter_harness.SampleResult(shots=16, logical_failures=0)
        with self.assertRaises(bench.BenchmarkError) as ctx:
            bench.join_cell(cell, report=report, sample=sample, seed=7)
        self.assertIn("error_budget", str(ctx.exception))


class CompileArgvTests(unittest.TestCase):
    """Argv/flag regression tests for compile_cell (no quonc invoke)."""

    def _cell(
        self,
        *,
        backend: str = "zoned",
        placer: str = "routing-agnostic",
        compact: bool = True,
    ) -> bench.GridCell:
        return bench.GridCell(
            workload="repetition_d3_memory",
            source=REPO_ROOT / "examples/na_qec/repetition_d3_memory.qn",
            na_placer=placer,
            na_backend=backend,
            na_compact=compact,
        )

    def test_argv_includes_na_backend_placer_and_dual_emit(self) -> None:
        cell = self._cell(backend="flat", placer="routing-aware", compact=True)
        argv = bench.compile_cell_argv(
            cell,
            quonc=Path("/fake/quonc"),
            target=Path("/fake/target.json"),
            report_path=Path("/tmp/report.json"),
            experiment_json=Path("/tmp/cell.qec.json"),
        )
        self.assertEqual(argv[0], "/fake/quonc")
        self.assertIn("--na-backend", argv)
        self.assertEqual(argv[argv.index("--na-backend") + 1], "flat")
        self.assertIn("--na-placer", argv)
        self.assertEqual(argv[argv.index("--na-placer") + 1], "routing-aware")
        self.assertIn("--emit-resource-report", argv)
        self.assertIn("--emit-qec-experiment", argv)
        self.assertNotIn("--no-na-compact", argv)

    def test_argv_includes_no_na_compact_when_compaction_off(self) -> None:
        cell = self._cell(compact=False)
        argv = bench.compile_cell_argv(
            cell,
            quonc=Path("/fake/quonc"),
            target=Path("/fake/target.json"),
            report_path=Path("/tmp/report.json"),
            experiment_json=Path("/tmp/cell.qec.json"),
        )
        self.assertIn("--no-na-compact", argv)

@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching")
class HelpAndCliTests(unittest.TestCase):
    def test_help_documents_ci_smoke_axis_and_local_full(self) -> None:
        parser = bench.build_arg_parser()
        help_text = parser.format_help()
        self.assertIn("--mode", help_text)
        self.assertIn("smoke", help_text)
        self.assertIn("axis", help_text)
        self.assertIn("full", help_text)
        self.assertIn("CI", help_text)
        self.assertIn("#111", help_text)
        self.assertIn("threshold", help_text.lower())
        self.assertIn("schedule-agnostic", help_text.lower())
        self.assertIn("--dry-run-compile", help_text)
        self.assertIn("--sinter-csv", help_text)
        self.assertIn("--include-flat", help_text)


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching")
class SmokeWithoutQuoncTests(unittest.TestCase):
    """CSV pipeline smoke using pinned dual-emit + fake report (no quonc)."""

    def test_run_cell_from_artifacts_writes_sinter_csv(self) -> None:
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
            import shutil

            exp_json = out_dir / "cell.qec.json"
            exp_stim = out_dir / "cell.stim"
            shutil.copy(PINNED_REP_D3_R2_JSON, exp_json)
            shutil.copy(
                PINNED_REP_D3_R2_JSON.with_name("qec_rep_d3_r2.stim"),
                exp_stim,
            )
            doc = json.loads(exp_json.read_text())
            doc["stim_file"] = "cell.stim"
            exp_json.write_text(json.dumps(doc))

            sinter_path = out_dir / "sinter.csv"
            row, sinter_rows = bench.evaluate_artifacts(
                cell,
                report_path=report_path,
                experiment_json=exp_json,
                shots=16,
                seed=7,
                sinter_csv_path=sinter_path,
            )
            self.assertEqual(row.shots, 16)
            self.assertGreaterEqual(row.logical_failures, 0)
            self.assertEqual(row.physical_atoms, 5)
            self.assertTrue(sinter_path.is_file())
            sinter_text = sinter_path.read_text()
            self.assertIn("evidence_kind: sampled", sinter_text)
            self.assertEqual(len(sinter_rows), 1)
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

    def test_smoke_mode_compiles_and_writes_join_plus_sinter_csv(self) -> None:
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
            work = Path(tmp) / "work"
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
                    str(work),
                ]
            )
            self.assertEqual(code, 0)
            self.assertTrue(csv_path.is_file())
            sinter_csv = csv_path.with_name("smoke.sinter.csv")
            self.assertTrue(sinter_csv.is_file(), "must emit separate Sinter CSV")
            # Primaries kept (no cleanup by default).
            cell_dirs = list(work.iterdir())
            self.assertTrue(cell_dirs)
            report = next(work.rglob("resource_report.json"), None)
            qec_json = next(work.rglob("*.qec.json"), None)
            stim = next(work.rglob("*.stim"), None)
            cell_sinter = next(work.rglob("sinter.csv"), None)
            self.assertIsNotNone(report)
            self.assertIsNotNone(qec_json)
            self.assertIsNotNone(stim)
            self.assertIsNotNone(cell_sinter)
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


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching")
class AxisCoverageIntegrationTests(unittest.TestCase):
    """Compile+Sinter per supported ablation axis + CX — gates full grid."""

    def test_axis_mode_compiles_each_axis_and_cx(self) -> None:
        quonc = _resolve_quonc_for_ci()
        if quonc is None:
            if os.environ.get("CI"):
                self.fail(
                    "quonc required for #254 axis-coverage gate "
                    "(build target/release/quonc or set QUONC); "
                    "full grid must not land unproven"
                )
            self.skipTest("quonc not built; skip local axis-coverage integration")

        target = REPO_ROOT / "targets/neutral_atom/generic_rna_v0.json"
        with tempfile.TemporaryDirectory() as tmp:
            csv_path = Path(tmp) / "axis.csv"
            work = Path(tmp) / "work"
            code = bench.main(
                [
                    "--mode",
                    "axis",
                    "--repo-root",
                    str(REPO_ROOT),
                    "--quonc",
                    str(quonc),
                    "--target",
                    str(target),
                    "--shots",
                    "8",
                    "--seed",
                    "7",
                    "--csv",
                    str(csv_path),
                    "--work-dir",
                    str(work),
                ]
            )
            self.assertEqual(
                code,
                0,
                "axis coverage must succeed on supported zoned axes + CX",
            )
            self.assertTrue(csv_path.is_file())
            self.assertTrue(csv_path.with_name("axis.sinter.csv").is_file())
            lines = [ln for ln in csv_path.read_text().splitlines() if not ln.startswith("#")]
            records = list(csv.DictReader(lines))
            self.assertEqual(len(records), 4)
            backends = {r["na_backend"] for r in records}
            placers = {r["na_placer"] for r in records}
            compact = {r["na_compact"] for r in records}
            workloads = {r["workload"] for r in records}
            self.assertEqual(backends, {"zoned"})
            self.assertEqual(placers, {"routing-agnostic", "routing-aware"})
            self.assertEqual(compact, {"true", "false"})
            self.assertIn("surface_d3_cx", workloads)
            # Primaries retained for every cell.
            self.assertEqual(len(list(work.glob("*/resource_report.json"))), 4)
            self.assertEqual(len(list(work.glob("*/sinter.csv"))), 4)


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching")
class FlatFailClosedTests(unittest.TestCase):
    """Flat QEC on generic_rna_v0 must fail closed with a clear error (not silent)."""

    def test_flat_cell_fails_closed_with_clear_error(self) -> None:
        quonc = _resolve_quonc_for_ci()
        if quonc is None:
            if os.environ.get("CI"):
                self.fail(
                    "quonc required for #254 flat fail-closed gate "
                    "(build target/release/quonc or set QUONC)"
                )
            self.skipTest("quonc not built; skip flat fail-closed check")

        target = REPO_ROOT / "targets/neutral_atom/generic_rna_v0.json"
        cell = bench.GridCell(
            workload="repetition_d3_memory",
            source=REPO_ROOT / "examples/na_qec/repetition_d3_memory.qn",
            na_placer="routing-agnostic",
            na_backend="flat",
            na_compact=True,
        )
        with tempfile.TemporaryDirectory() as tmp:
            with self.assertRaises(bench.BenchmarkError) as ctx:
                bench.compile_cell(
                    cell,
                    quonc=quonc,
                    target=target,
                    work_dir=Path(tmp),
                )
            msg = str(ctx.exception).lower()
            self.assertIn("flat", msg)
            self.assertTrue(
                "geometry" in msg or "rydberg" in msg or "unsupported" in msg
                or "failed" in msg,
                msg,
            )


class DryRunCompileTests(unittest.TestCase):
    def test_dry_run_compile_full_mode_with_mock_quonc_path(self) -> None:
        """Flag validation for the full Cartesian without needing a real binary."""
        with mock.patch.object(bench, "resolve_quonc", return_value=Path("/fake/quonc")):
            with tempfile.TemporaryDirectory() as tmp:
                code = bench.main(
                    [
                        "--mode",
                        "full",
                        "--repo-root",
                        str(REPO_ROOT),
                        "--dry-run-compile",
                        "--work-dir",
                        tmp,
                    ]
                )
                self.assertEqual(code, 0)

    def test_dry_run_compile_axis_mode(self) -> None:
        with mock.patch.object(bench, "resolve_quonc", return_value=Path("/fake/quonc")):
            with tempfile.TemporaryDirectory() as tmp:
                code = bench.main(
                    [
                        "--mode",
                        "axis",
                        "--repo-root",
                        str(REPO_ROOT),
                        "--dry-run-compile",
                        "--work-dir",
                        tmp,
                    ]
                )
                self.assertEqual(code, 0)


if __name__ == "__main__":
    unittest.main()
