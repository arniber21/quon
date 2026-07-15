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

import quon_qec_sinter as harness  # noqa: E402

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

ERROR_MODEL = {
    "rydberg": 0.01,
    "measurement": 0.02,
    "reset": 0.03,
    "movement": 0.004,
    "transfer": 0.005,
    "idle_per_us": 1e-6,
}


def _write_experiment(dir_path: Path, *, stim_text: str = MINIMAL_REP_STIM) -> Path:
    stim_name = "rep_d3.stim"
    json_path = dir_path / "rep_d3.qec.json"
    (dir_path / stim_name).write_text(stim_text)
    doc = {
        "schema_version": 1,
        "kind": "qec_experiment",
        "family": "repetition",
        "code_family": "repetition_code_toy",
        "distance": 3,
        "rounds": 1,
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
        "error_model": dict(ERROR_MODEL),
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


@unittest.skipUnless(HAS_STIM_STACK, "requires stim/sinter/pymatching (python/requirements.txt)")
class SampleAndCsvTests(unittest.TestCase):
    def test_sample_is_deterministic_with_fixed_seed(self) -> None:
        import stim as stim_mod

        noisy = harness.annotate_noise(stim_mod.Circuit(MINIMAL_REP_STIM), ERROR_MODEL)
        a = harness.sample_logical_failures(noisy, shots=32, seed=7)
        b = harness.sample_logical_failures(noisy, shots=32, seed=7)
        self.assertEqual(a.logical_failures, b.logical_failures)
        self.assertEqual(a.shots, 32)
        self.assertAlmostEqual(a.logical_failure_rate, a.logical_failures / 32)

    def test_csv_row_columns(self) -> None:
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
        buf.seek(0)
        reader = csv.DictReader(buf)
        self.assertEqual(list(reader.fieldnames or []), harness.CSV_COLUMNS)
        got = next(reader)
        self.assertEqual(got["distance"], "3")
        self.assertEqual(got["rounds"], "1")
        self.assertEqual(got["shots"], "32")
        self.assertEqual(got["rydberg"], "0.01")
        self.assertEqual(got["logical_failures"], "2")
        self.assertEqual(got["logical_failure_rate"], str(2 / 32))

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
            self.assertNotIn("threshold", text.lower())


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
        self.assertNotIn("below the threshold", help_text.lower())


if __name__ == "__main__":
    unittest.main()
