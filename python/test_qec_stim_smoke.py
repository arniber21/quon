#!/usr/bin/env python3
"""Stim structure + tiny Sinter harness smoke (ADR-0022 / #255 / #253).

Run:  python -m unittest python/test_qec_stim_smoke.py

Validates that a minimal structure-only Stim circuit (matching the repetition
d=3 emit shape) parses, exposes detectors/observables, and samples
deterministically with a fixed seed when no noise channels are present.

Also runs a tiny fixed-seed noisy sample through `quon_qec_sinter` so CI
covers the Python noise-annotation + decode path (ADR-0024), including the
pinned #255 rounds=2 dual-emit pair under python/testdata/.
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

try:
    import stim

    HAS_STIM = True
except ImportError:
    HAS_STIM = False

try:
    import pymatching  # noqa: F401
    import sinter  # noqa: F401

    import quon_qec_sinter as harness

    HAS_HARNESS = True
except ImportError:
    HAS_HARNESS = False

# Fail hard under CI if the Stim stack is missing (do not skip green).
if (not HAS_STIM or not HAS_HARNESS) and os.environ.get("CI"):
    raise ImportError(
        "stim/sinter/pymatching required in CI for QEC smoke tests "
        "(python/requirements.txt / just setup-python / ADR-0022)"
    )


# Minimal structure-only circuit mirroring quon_qec emit_stim_structure for d=3,
# 1 memory round (kept tiny for unit-test speed).
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

_SMOKE_ERROR_MODEL = {
    "rydberg": 0.01,
    "measurement": 0.02,
    "reset": 0.03,
    "movement": 0.004,
    "transfer": 0.005,
    "idle_per_us": 1e-6,
}

# Higher-noise golden circuit (non-trivial under compose movement+idle).
_GOLDEN_ERROR_MODEL = {
    "rydberg": 0.05,
    "measurement": 0.05,
    "reset": 0.05,
    "movement": 0.02,
    "transfer": 0.02,
    "idle_per_us": 1e-4,
}
# Band for MINIMAL_REP_STIM + _GOLDEN_ERROR_MODEL, shots=32, seed=7. The exact
# fixed-seed count is architecture-sensitive (stim's vectorized sampling varies
# with SIMD width: 4 on arm64, 2 on x86_64 CI), so pin a band — nonzero proves
# the scaled error model injects noise; well under half the shots proves the
# decoder beats chance. Same-machine determinism is asserted separately in
# test_noisy_sample_is_deterministic.
GOLDEN_SMOKE_FAILURES_MIN = 1
GOLDEN_SMOKE_FAILURES_MAX = 8

TESTDATA = Path(__file__).resolve().parent / "testdata"
PINNED_REP_D3_R2_JSON = TESTDATA / "qec_rep_d3_r2.qec.json"
PINNED_SURFACE_D3_R2_STIM = TESTDATA / "qec_surface_d3_r2.stim"


@unittest.skipUnless(HAS_STIM, "requires stim (python/requirements.txt / ADR-0022)")
class StimStructureSmokeTests(unittest.TestCase):
    def test_circuit_parses(self) -> None:
        circuit = stim.Circuit(MINIMAL_REP_STIM)
        self.assertGreater(circuit.num_qubits, 0)

    def test_detector_and_observable_counts(self) -> None:
        circuit = stim.Circuit(MINIMAL_REP_STIM)
        self.assertEqual(circuit.num_detectors, 4)
        self.assertEqual(circuit.num_observables, 1)

    def test_noiseless_sample_is_deterministic(self) -> None:
        circuit = stim.Circuit(MINIMAL_REP_STIM)
        a = circuit.compile_sampler(seed=7).sample(shots=16)
        b = circuit.compile_sampler(seed=7).sample(shots=16)
        self.assertTrue((a == b).all())

    def test_surface_x_extraction_requires_h_sandwich(self) -> None:
        """Fault-inject: strip X-check H → first-round X MRs become vacuous.

        Under |0⟩^n Z-memory, X-check measurements with a proper H sandwich are
        random (not used as first-round detectors). Without mid/after H,
        CX(check→data) leaves checks at |0⟩ so X-check MRs are always 0 —
        vacuous extraction that must fail the randomness check.
        """
        self.assertTrue(PINNED_SURFACE_D3_R2_STIM.is_file(), PINNED_SURFACE_D3_R2_STIM)
        stim_text = PINNED_SURFACE_D3_R2_STIM.read_text()
        self.assertIn("H 9 11 14 16", stim_text)

        # Measurement order: MR×8 (r0) + MR×8 (r1) + MZ×9. X-checks at
        # atoms 9,11,14,16 → indices 0,2,5,7 within each MR block.
        x_idx = [0, 2, 5, 7]

        good = stim.Circuit(stim_text)
        good_m = good.compile_sampler(seed=0).sample(shots=128)
        good_x = good_m[:, x_idx]
        self.assertTrue(
            good_x.any() and not good_x.all(),
            "with H sandwich, first-round X-check MRs must be non-constant",
        )

        stripped_lines = [
            line
            for line in stim_text.splitlines()
            if not (line.startswith("H ") and "9" in line and "11" in line)
        ]
        stripped = "\n".join(stripped_lines) + "\n"
        self.assertNotIn("H 9 11 14 16", stripped)
        bad = stim.Circuit(stripped)
        bad_m = bad.compile_sampler(seed=0).sample(shots=128)
        bad_x = bad_m[:, x_idx]
        self.assertFalse(
            bad_x.any(),
            "without X-check H, first-round X MRs must be vacuously all-zero",
        )


@unittest.skipUnless(
    HAS_STIM and HAS_HARNESS,
    "requires stim/sinter/pymatching (python/requirements.txt / ADR-0022)",
)
class SinterHarnessSmokeTests(unittest.TestCase):
    """Tiny-shot deterministic check for the #253 harness (CI)."""

    def test_noisy_sample_is_deterministic(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            stim_path = root / "rep_d3.stim"
            json_path = root / "rep_d3.qec.json"
            stim_path.write_text(MINIMAL_REP_STIM)
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
                            "atoms": [0, 1, 2, 3, 4],
                            "data_atoms": [0, 2, 4],
                            "check_atoms": [1, 3],
                            "stabilizers": [],
                        },
                        "measurement_schedule": [],
                        "logical_observables": [],
                        "atom_site_map": [],
                        "error_model": dict(_SMOKE_ERROR_MODEL),
                        "na_refs": [],
                        "stim_file": "rep_d3.stim",
                    }
                )
            )
            rows_a = harness.run_experiments(
                [json_path], shots_list=[16], seed=7, error_scales=[1.0]
            )
            rows_b = harness.run_experiments(
                [json_path], shots_list=[16], seed=7, error_scales=[1.0]
            )
            self.assertEqual(len(rows_a), 1)
            self.assertEqual(rows_a[0].logical_failures, rows_b[0].logical_failures)
            self.assertEqual(rows_a[0].shots, 16)

    def test_golden_logical_failures_fixed_seed(self) -> None:
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            stim_path = root / "rep_d3.stim"
            json_path = root / "rep_d3.qec.json"
            stim_path.write_text(MINIMAL_REP_STIM)
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
                            "atoms": [0, 1, 2, 3, 4],
                            "data_atoms": [0, 2, 4],
                            "check_atoms": [1, 3],
                            "stabilizers": [],
                        },
                        "measurement_schedule": [],
                        "logical_observables": [],
                        "atom_site_map": [],
                        "error_model": dict(_GOLDEN_ERROR_MODEL),
                        "na_refs": [],
                        "stim_file": "rep_d3.stim",
                    }
                )
            )
            rows = harness.run_experiments(
                [json_path], shots_list=[32], seed=7, error_scales=[1.0]
            )
            self.assertGreaterEqual(
                rows[0].logical_failures, GOLDEN_SMOKE_FAILURES_MIN
            )
            self.assertLessEqual(rows[0].logical_failures, GOLDEN_SMOKE_FAILURES_MAX)

    def test_pinned_rounds2_dual_emit_smoke(self) -> None:
        """CI exercises the real #255 rounds=2 dual-emit pair."""
        self.assertTrue(PINNED_REP_D3_R2_JSON.is_file(), PINNED_REP_D3_R2_JSON)
        rows = harness.run_experiments(
            [PINNED_REP_D3_R2_JSON], shots_list=[32], seed=7, error_scales=[1.0]
        )
        self.assertEqual(rows[0].rounds, 2)
        self.assertEqual(rows[0].distance, 3)
        self.assertEqual(rows[0].logical_failures, 0)


if __name__ == "__main__":
    unittest.main()
