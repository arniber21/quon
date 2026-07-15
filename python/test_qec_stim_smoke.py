#!/usr/bin/env python3
"""Stim structure smoke for QEC experiment emit (ADR-0022 / #255).

Run:  python -m unittest python/test_qec_stim_smoke.py

Validates that a minimal structure-only Stim circuit (matching the repetition
d=3 emit shape) parses, exposes detectors/observables, and samples
deterministically with a fixed seed when no noise channels are present.
"""

from __future__ import annotations

import unittest

try:
    import stim

    HAS_STIM = True
except ImportError:
    HAS_STIM = False


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


if __name__ == "__main__":
    unittest.main()
