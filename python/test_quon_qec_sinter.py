#!/usr/bin/env python3
"""Unit tests for the Quon QEC Stim/Sinter harness (#253 / ADR-0024).

Run:  python -m unittest python/test_quon_qec_sinter.py
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
    import stim  # noqa: F401
    import sinter  # noqa: F401
    import pymatching  # noqa: F401

    HAS_STIM_STACK = True
except ImportError:
    HAS_STIM_STACK = False

# Fail hard under CI if the Stim stack is missing (do not skip green).
if not HAS_STIM_STACK and os.environ.get("CI"):
    raise ImportError(
        "stim/sinter/pymatching required in CI for #253 harness tests "
        "(python/requirements.txt / just setup-python)"
    )

import quon_qec_sinter as harness  # noqa: E402

TESTDATA = Path(__file__).resolve().parent / "testdata"
PINNED_REP_D3_R2_JSON = TESTDATA / "qec_rep_d3_r2.qec.json"

# Structure-only circuit matching quon_qec emit_stim_structure for d=3, 1 round.
MINIMAL_REP_STIM = """
QUBIT_COORDS(0, 0) 0
QUBIT_COORDS(1, 0) 1
QUBIT_COORDS(2, 0) 2
QUBIT_COORDS(3, 0) 3
QUBIT_COORDS(4, 0) 4
R 0 1 2 3 4
TICK
CX 0 1 2 3
TICK
CX 2 1 4 3
TICK
MR 1 3
DETECTOR(0, 0) rec[-2]
DETECTOR(1, 0) rec[-1]
TICK
MZ 0 2 4
DETECTOR(0, 1) rec[-5] rec[-3] rec[-2]
DETECTOR(1, 1) rec[-4] rec[-2] rec[-1]
OBSERVABLE_INCLUDE(0) rec[-3] rec[-2] rec[-1]
"""

# Higher-noise model for a non-trivial fixed-seed golden (not the CI smoke rates).
GOLDEN_ERROR_MODEL = {
    "rydberg": 0.05,
    "measurement": 0.05,
    "reset": 0.05,
    "movement": 0.02,
    "transfer": 0.02,
    "idle_per_us": 1e-4,
}
# Band for MINIMAL_REP_STIM + GOLDEN_ERROR_MODEL, shots=32, seed=7. The exact
# fixed-seed count is architecture-sensitive (stim's vectorized sampling varies
# with SIMD width: 4 on arm64, differs on x86_64 CI), so pin a band — nonzero
# proves noise injection, well under half the shots proves decoding beats
# chance. Same-machine determinism is asserted in the deterministic tests.
GOLDEN_MINIMAL_FAILURES_MIN = 1
GOLDEN_MINIMAL_FAILURES_MAX = 8

ERROR_MODEL = {
    "rydberg": 0.01,
    "measurement": 0.02,
    "reset": 0.03,
    "movement": 0.004,
    "transfer": 0.005,
    "idle_per_us": 1e-6,
}


def _write_experiment(
    dir_path: Path,
    *,
    stim_text: str = MINIMAL_REP_STIM,
    error_model: dict | None = None,
    distance: int = 3,
    rounds: int = 1,
) -> Path:
    stim_name = "rep_d3.stim"
    json_path = dir_path / "rep_d3.qec.json"
    (dir_path / stim_name).write_text(stim_text)
    doc = {
        "schema_version": 1,
        "kind": "qec_experiment",
        "family": "repetition",
        "code_family": "repetition_code_toy",
        "distance": distance,
        "rounds": rounds,
        "logical_ids": [0],
        "check_graph": {
            "atoms": [0, 1, 2, 3, 4],
            "data_atoms": [0, 2, 4],
            "check_atoms": [1, 3],
            "stabilizers": [],
        },
        "measurement_schedule": [],
        "logical_observables": [],
        "atom_site_map": [],
        "error_model": dict(error_model or ERROR_MODEL),
        "na_refs": [],
        "stim_file": stim_name,
    }
    json_path.write_text(json.dumps(doc))
    return json_path


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching (python/requirements.txt)")
class LoadExperimentTests(unittest.TestCase):
    def test_loads_json_and_sibling_stim(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            json_path = _write_experiment(Path(tmp))
            experiment, circuit = harness.load_experiment(json_path)
            self.assertEqual(experiment["distance"], 3)
            self.assertEqual(experiment["rounds"], 1)
            self.assertEqual(experiment["error_model"]["rydberg"], 0.01)
            self.assertGreater(circuit.num_qubits, 0)
            self.assertEqual(circuit.num_detectors, 4)
            self.assertEqual(circuit.num_observables, 1)

    def test_rejects_missing_sibling_stim(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            json_path = Path(tmp) / "orphan.qec.json"
            json_path.write_text(
                json.dumps(
                    {
                        "schema_version": 1,
                        "kind": "qec_experiment",
                        "family": "repetition",
                        "code_family": "repetition_code_toy",
                        "distance": 3,
                        "rounds": 1,
                        "logical_ids": [0],
                        "check_graph": {
                            "atoms": [],
                            "data_atoms": [],
                            "check_atoms": [],
                            "stabilizers": [],
                        },
                        "measurement_schedule": [],
                        "logical_observables": [],
                        "atom_site_map": [],
                        "error_model": dict(ERROR_MODEL),
                        "na_refs": [],
                        "stim_file": "missing.stim",
                    }
                )
            )
            with self.assertRaises(harness.ExperimentLoadError):
                harness.load_experiment(json_path)

    def test_rejects_wrong_kind(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            json_path = _write_experiment(Path(tmp))
            doc = json.loads(json_path.read_text())
            doc["kind"] = "not_an_experiment"
            json_path.write_text(json.dumps(doc))
            with self.assertRaises(harness.ExperimentLoadError):
                harness.load_experiment(json_path)

    def test_rejects_invalid_distance_rounds_types(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for key, bad in (("distance", 0), ("distance", 1.5), ("rounds", "2"), ("rounds", True)):
                json_path = _write_experiment(root)
                doc = json.loads(json_path.read_text())
                doc[key] = bad
                json_path.write_text(json.dumps(doc))
                with self.assertRaises(harness.ExperimentLoadError):
                    harness.load_experiment(json_path)

    def test_rejects_non_finite_or_out_of_range_error_model(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for bad in (-0.1, 1.1, float("nan"), float("inf"), "0.01"):
                em = dict(ERROR_MODEL)
                em["rydberg"] = bad
                json_path = _write_experiment(root, error_model=em)
                with self.assertRaises(harness.HarnessError):
                    harness.load_experiment(json_path)

    def test_rejects_stim_illegal_rydberg_at_load(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            em = dict(ERROR_MODEL)
            em["rydberg"] = 0.99  # > 15/16, still in [0,1]
            json_path = _write_experiment(Path(tmp), error_model=em)
            with self.assertRaises(harness.HarnessError) as ctx:
                harness.load_experiment(json_path)
            self.assertIn("Stim-illegal", str(ctx.exception))


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching (python/requirements.txt)")
class AnnotateNoiseTests(unittest.TestCase):
    def test_structure_circuit_has_no_noise_ops(self) -> None:
        import stim as stim_mod

        circuit = stim_mod.Circuit(MINIMAL_REP_STIM)
        for inst in circuit.flattened():
            self.assertNotIn(
                inst.name,
                {
                    "DEPOLARIZE1",
                    "DEPOLARIZE2",
                    "X_ERROR",
                    "Z_ERROR",
                    "Y_ERROR",
                    "PAULI_CHANNEL_1",
                    "PAULI_CHANNEL_2",
                },
            )

    def test_annotation_inserts_noise_channels(self) -> None:
        import stim as stim_mod

        noisy = harness.annotate_noise(stim_mod.Circuit(MINIMAL_REP_STIM), ERROR_MODEL)
        names = {inst.name for inst in noisy.flattened()}
        self.assertIn("DEPOLARIZE2", names)
        self.assertIn("X_ERROR", names)
        self.assertIn("DEPOLARIZE1", names)

    def test_annotation_preserves_detectors_and_observables(self) -> None:
        import stim as stim_mod

        base = stim_mod.Circuit(MINIMAL_REP_STIM)
        noisy = harness.annotate_noise(base, ERROR_MODEL)
        self.assertEqual(noisy.num_detectors, base.num_detectors)
        self.assertEqual(noisy.num_observables, base.num_observables)

    def test_zero_rates_leave_structure_only(self) -> None:
        import stim as stim_mod

        zeros = {k: 0.0 for k in ERROR_MODEL}
        noisy = harness.annotate_noise(stim_mod.Circuit(MINIMAL_REP_STIM), zeros)
        for inst in noisy.flattened():
            self.assertNotIn(
                inst.name,
                {"DEPOLARIZE1", "DEPOLARIZE2", "X_ERROR", "Z_ERROR", "Y_ERROR"},
            )

    def test_measurement_noise_x_for_mz_z_for_mx(self) -> None:
        import stim as stim_mod

        mz = stim_mod.Circuit("R 0\nMZ 0")
        mx = stim_mod.Circuit("R 0\nMX 0")
        em = {k: 0.0 for k in ERROR_MODEL}
        em["measurement"] = 0.1
        mz_noisy = harness.annotate_noise(mz, em)
        mx_noisy = harness.annotate_noise(mx, em)
        mz_names = [inst.name for inst in mz_noisy.flattened()]
        mx_names = [inst.name for inst in mx_noisy.flattened()]
        self.assertIn("X_ERROR", mz_names)
        self.assertNotIn("Z_ERROR", mz_names)
        self.assertIn("Z_ERROR", mx_names)
        self.assertNotIn("X_ERROR", mx_names)

    def test_flattens_repeat_blocks(self) -> None:
        import stim as stim_mod

        circuit = stim_mod.Circuit(
            """
            R 0 1
            TICK
            REPEAT 2 {
              CX 0 1
              TICK
            }
            MZ 0 1
            """
        )
        em = {k: 0.0 for k in ERROR_MODEL}
        em["rydberg"] = 0.01
        em["movement"] = 0.02
        # Must not raise AttributeError on CircuitRepeatBlock.
        noisy = harness.annotate_noise(circuit, em)
        cx_count = sum(1 for inst in noisy.flattened() if inst.name == "CX")
        depol2 = sum(1 for inst in noisy.flattened() if inst.name == "DEPOLARIZE2")
        self.assertEqual(cx_count, 2)
        self.assertEqual(depol2, 2)

    def test_composes_movement_and_idle_into_one_depolarize1_per_tick(self) -> None:
        import stim as stim_mod

        circuit = stim_mod.Circuit("R 0\nTICK\nMZ 0")
        em = {k: 0.0 for k in ERROR_MODEL}
        em["movement"] = 0.1
        em["idle_per_us"] = 0.1
        noisy = harness.annotate_noise(circuit, em, tick_us=1.0)
        ops = list(noisy.flattened())
        # Find TICK then exactly one DEPOLARIZE1 before MZ / measurement noise.
        tick_idx = next(i for i, op in enumerate(ops) if op.name == "TICK")
        following = [op.name for op in ops[tick_idx + 1 :]]
        depol_count = following.count("DEPOLARIZE1")
        self.assertEqual(depol_count, 1)
        # Composed p = 1 - (1-0.1)*(1-0.1) = 0.19
        depol = next(op for op in ops[tick_idx + 1 :] if op.name == "DEPOLARIZE1")
        self.assertAlmostEqual(depol.gate_args_copy()[0], 0.19)


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching (python/requirements.txt)")
class ScaleAndSampleTests(unittest.TestCase):
    def test_scale_clamps_to_stim_maxima_not_one(self) -> None:
        scaled = harness.scale_error_model(ERROR_MODEL, 1000.0)
        self.assertLessEqual(scaled["rydberg"], harness.STIM_DEPOLARIZE2_MAX)
        self.assertEqual(scaled["rydberg"], harness.STIM_DEPOLARIZE2_MAX)
        self.assertLessEqual(scaled["movement"], harness.STIM_DEPOLARIZE1_MAX)
        self.assertEqual(scaled["movement"], harness.STIM_DEPOLARIZE1_MAX)
        self.assertLessEqual(scaled["transfer"], harness.STIM_DEPOLARIZE1_MAX)
        self.assertNotEqual(scaled["rydberg"], 1.0)
        self.assertNotEqual(scaled["movement"], 1.0)

    def test_large_scale_does_not_crash_dem(self) -> None:
        import stim as stim_mod

        scaled = harness.scale_error_model(ERROR_MODEL, 1000.0)
        noisy = harness.annotate_noise(stim_mod.Circuit(MINIMAL_REP_STIM), scaled)
        # Must build DEM without ValueError.
        sample = harness.sample_logical_failures(noisy, shots=8, seed=7)
        self.assertEqual(sample.shots, 8)

    def test_sample_is_deterministic_with_fixed_seed(self) -> None:
        import stim as stim_mod

        noisy = harness.annotate_noise(stim_mod.Circuit(MINIMAL_REP_STIM), ERROR_MODEL)
        a = harness.sample_logical_failures(noisy, shots=32, seed=7)
        b = harness.sample_logical_failures(noisy, shots=32, seed=7)
        self.assertEqual(a.logical_failures, b.logical_failures)
        self.assertEqual(a.shots, 32)
        self.assertAlmostEqual(a.logical_failure_rate, a.logical_failures / 32)

    def test_golden_logical_failures_pinned_noisy_circuit(self) -> None:
        import stim as stim_mod

        noisy = harness.annotate_noise(
            stim_mod.Circuit(MINIMAL_REP_STIM), GOLDEN_ERROR_MODEL
        )
        sample = harness.sample_logical_failures(noisy, shots=32, seed=7)
        self.assertGreaterEqual(sample.logical_failures, GOLDEN_MINIMAL_FAILURES_MIN)
        self.assertLessEqual(sample.logical_failures, GOLDEN_MINIMAL_FAILURES_MAX)

    def test_main_catches_value_error_as_exit_one(self) -> None:
        # Force a HarnessError path via invalid tick_us through CLI.
        rc = harness.main(
            [
                str(PINNED_REP_D3_R2_JSON),
                "--shots",
                "8",
                "--seed",
                "7",
                "--tick-us",
                "0",
            ]
        )
        self.assertEqual(rc, 1)


class CsvEmitTests(unittest.TestCase):
    """CSV labeling does not need Stim — keep ungated for local AFK agents."""

    def test_csv_row_columns_and_sampled_label(self) -> None:
        row = harness.ResultRow(
            distance=3,
            rounds=1,
            shots=32,
            error_model=dict(ERROR_MODEL),
            logical_failures=2,
            logical_failure_rate=2 / 32,
        )
        buf = io.StringIO()
        harness.write_csv(buf, [row])
        text = buf.getvalue()
        self.assertTrue(
            text.startswith("# evidence_kind: sampled\n"),
            "CSV artifact must open with sampled evidence banner",
        )
        self.assertIn("not a threshold claim", text.lower())
        self.assertNotIn("below the threshold", text.lower())
        # Skip `#` comment banner lines before the CSV header.
        data_lines = [ln for ln in text.splitlines() if not ln.startswith("#")]
        reader = csv.DictReader(io.StringIO("\n".join(data_lines) + "\n"))
        self.assertEqual(list(reader.fieldnames or []), harness.CSV_COLUMNS)
        got = next(reader)
        self.assertEqual(got["evidence_kind"], "sampled")
        self.assertEqual(got["distance"], "3")
        self.assertEqual(got["rounds"], "1")
        self.assertEqual(got["shots"], "32")
        self.assertEqual(got["rydberg"], "0.01")
        self.assertEqual(got["logical_failures"], "2")
        self.assertEqual(got["logical_failure_rate"], str(2 / 32))


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching (python/requirements.txt)")
class SampleAndCsvTests(unittest.TestCase):
    def test_run_experiment_end_to_end_csv(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            json_path = _write_experiment(Path(tmp))
            rows = harness.run_experiments(
                [json_path],
                shots_list=[16],
                seed=7,
                error_scales=[1.0],
            )
            self.assertEqual(len(rows), 1)
            self.assertEqual(rows[0].distance, 3)
            self.assertEqual(rows[0].shots, 16)
            out = Path(tmp) / "out.csv"
            with out.open("w", newline="") as f:
                harness.write_csv(f, rows)
            text = out.read_text()
            self.assertIn("logical_failures", text)
            self.assertIn("evidence_kind", text)
            self.assertIn("sampled", text.lower())
            # Anti-threshold disclaimer is allowed; positive claims are not.
            self.assertIn("not a threshold claim", text.lower())
            self.assertNotIn("below the threshold", text.lower())
            self.assertNotIn("below threshold", text.lower())

    def test_pinned_rounds2_emit_integration(self) -> None:
        """Real #255 dual-emit shape (d=3, rounds=2) must load and sample."""
        self.assertTrue(PINNED_REP_D3_R2_JSON.is_file(), PINNED_REP_D3_R2_JSON)
        rows = harness.run_experiments(
            [PINNED_REP_D3_R2_JSON],
            shots_list=[32],
            seed=7,
            error_scales=[1.0],
        )
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0].distance, 3)
        self.assertEqual(rows[0].rounds, 2)
        self.assertEqual(rows[0].shots, 32)
        # Deterministic golden for pinned emit + target error_model + seed 7.
        self.assertEqual(rows[0].logical_failures, 0)


class WilsonAndFingerprintTests(unittest.TestCase):
    """Deterministic helpers for the sampled-evidence JSON (no Stim needed)."""

    def test_wilson_interval_zero_failures(self) -> None:
        low, high = harness.wilson_interval(0, 32)
        self.assertAlmostEqual(low, 0.0)
        self.assertGreaterEqual(low, 0.0)
        self.assertGreater(high, 0.0)
        self.assertLessEqual(high, 1.0)

    def test_wilson_interval_brackets_point_estimate(self) -> None:
        low, high = harness.wilson_interval(4, 32)
        p = 4 / 32
        self.assertLessEqual(low, p)
        self.assertGreaterEqual(high, p)
        self.assertGreaterEqual(low, 0.0)
        self.assertLessEqual(high, 1.0)

    def test_wilson_interval_zero_shots(self) -> None:
        self.assertEqual(harness.wilson_interval(0, 0), (0.0, 0.0))

    def test_wilson_interval_rejects_bad_level(self) -> None:
        with self.assertRaises(harness.HarnessError):
            harness.wilson_interval(1, 8, level=1.5)

    def test_experiment_sha256_matches_manual(self) -> None:
        import hashlib

        with tempfile.TemporaryDirectory() as tmp:
            json_path = _write_experiment(Path(tmp))
            expected = hashlib.sha256(json_path.read_bytes()).hexdigest()
            self.assertEqual(harness.experiment_sha256(json_path), expected)


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching (python/requirements.txt)")
class SampledDocumentTests(unittest.TestCase):
    def test_build_sampled_document_shape(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            json_path = _write_experiment(Path(tmp))
            doc = harness.build_sampled_document(
                [json_path],
                shots_list=[16],
                seed=7,
                decoder="pymatching",
                confidence_level=0.95,
            )
            self.assertEqual(doc["schema_version"], harness.SAMPLED_SCHEMA_VERSION)
            self.assertEqual(doc["evidence_kind"], "sampled")
            self.assertIn("not a threshold claim", doc["disclaimer"].lower())
            self.assertEqual(doc["decoder"], "pymatching")
            self.assertEqual(doc["seed"], 7)
            self.assertEqual(doc["confidence_level"], 0.95)
            self.assertEqual(len(doc["experiments"]), 1)
            exp = doc["experiments"][0]
            self.assertEqual(exp["distance"], 3)
            self.assertEqual(exp["rounds"], 1)
            self.assertEqual(exp["family"], "repetition")
            self.assertEqual(exp["code_family"], "repetition_code_toy")
            self.assertEqual(
                exp["experiment_sha256"], harness.experiment_sha256(json_path)
            )
            self.assertEqual(len(exp["results"]), 1)
            result = exp["results"][0]
            self.assertEqual(result["shots"], 16)
            self.assertEqual(result["error_scale"], 1.0)
            self.assertIn("rydberg", result["noise_model"])
            self.assertIn("logical_failures", result)
            self.assertIn("logical_failure_rate", result)
            ci = result["confidence_interval"]
            self.assertEqual(ci["method"], "wilson")
            self.assertEqual(ci["level"], 0.95)
            self.assertLessEqual(ci["low"], ci["high"])

    def test_cli_json_output_writes_document(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            json_path = _write_experiment(Path(tmp))
            out = Path(tmp) / "sampled.json"
            rc = harness.main(
                [str(json_path), "--shots", "16", "--seed", "7", "--json", str(out)]
            )
            self.assertEqual(rc, 0)
            self.assertTrue(out.is_file())
            doc = json.loads(out.read_text())
            self.assertEqual(doc["evidence_kind"], "sampled")
            self.assertEqual(len(doc["experiments"]), 1)


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching (python/requirements.txt)")
class HelpAndCliTests(unittest.TestCase):
    def test_help_documents_sweeps_and_local_runs(self) -> None:
        parser = harness.build_arg_parser()
        help_text = parser.format_help()
        self.assertIn("Distance / round sweeps", help_text)
        self.assertIn("quonc", help_text)
        self.assertIn("--shots", help_text)
        self.assertIn("--scale-errors", help_text)
        self.assertIn("Local larger runs", help_text)
        self.assertIn("not a threshold claim", help_text.lower())
        self.assertIn("Analytic vs sampled", help_text)
        self.assertIn("ResourceReport", help_text)
        self.assertIn("--emit-resource-report", help_text)
        self.assertIn("no merged summary", help_text.lower())
        self.assertIn("tick-us", help_text.lower())
        self.assertIn("proxy", help_text.lower())
        self.assertIn("3/4", help_text)
        self.assertIn("15/16", help_text)
        self.assertNotIn("below the threshold", help_text.lower())


if __name__ == "__main__":
    unittest.main()
