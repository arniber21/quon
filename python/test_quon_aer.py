#!/usr/bin/env python3
"""Unit tests for the Quon -> Aer verification seam (issue #204).

Run:  python -m unittest python/test_quon_aer.py

These tests are pure-Python and always runnable; the handful that touch
`qiskit` behavior are skipped (not failed) when `qiskit` isn't importable,
mirroring `noisy_fidelity.py`'s own optional-dependency handling.
"""

from __future__ import annotations

import os
import subprocess
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import quon_aer  # noqa: E402

try:
    import qiskit  # noqa: F401

    HAS_QISKIT = True
except ImportError:
    HAS_QISKIT = False


class NormalizeBitIntConditionsTests(unittest.TestCase):
    """The named, public Qiskit-importer dialect adapter (was `_qiskit_qasm3_compat`)."""

    def test_rewrites_bit_equals_one(self) -> None:
        self.assertEqual(
            quon_aer.normalize_bit_int_conditions("if (c[0] == 1) { x q[1]; }"),
            "if (c[0] == true) { x q[1]; }",
        )

    def test_rewrites_bit_equals_zero(self) -> None:
        self.assertEqual(
            quon_aer.normalize_bit_int_conditions("if (c[2] == 0) { z q[0]; }"),
            "if (c[2] == false) { z q[0]; }",
        )

    def test_rewrites_multiple_conditions_on_one_line(self) -> None:
        src = "if (c[0] == 1) { x q[0]; } if (c[1] == 0) { z q[0]; }"
        expected = "if (c[0] == true) { x q[0]; } if (c[1] == false) { z q[0]; }"
        self.assertEqual(quon_aer.normalize_bit_int_conditions(src), expected)

    def test_leaves_whole_register_integer_comparisons_alone(self) -> None:
        # `c == 3` (no `[index]`) is a whole-bit-array integer comparison,
        # already accepted by qiskit_qasm3_import; must not be rewritten.
        src = "if (c == 3) { x q[0]; }"
        self.assertEqual(quon_aer.normalize_bit_int_conditions(src), src)

    def test_idempotent_on_already_boolean_input(self) -> None:
        src = "if (c[0] == true) { x q[1]; }"
        self.assertEqual(quon_aer.normalize_bit_int_conditions(src), src)

    def test_leaves_qasm_without_conditions_unchanged(self) -> None:
        src = "OPENQASM 3.0;\nqubit[2] q;\nbit[2] c;\nc[0] = measure q[0];\n"
        self.assertEqual(quon_aer.normalize_bit_int_conditions(src), src)


class NormalizeKeyAndClbitTests(unittest.TestCase):
    def test_normalize_key_strips_spaces(self) -> None:
        self.assertEqual(quon_aer.normalize_key("1 01"), "101")
        self.assertEqual(quon_aer.normalize_key("101"), "101")

    def test_clbit_reads_high_index_first(self) -> None:
        # Qiskit prints c[2] c[1] c[0] left-to-right for a 3-bit register.
        key = "110"
        self.assertEqual(quon_aer.clbit(key, 0, 3), 0)
        self.assertEqual(quon_aer.clbit(key, 1, 3), 1)
        self.assertEqual(quon_aer.clbit(key, 2, 3), 1)

    def test_clbit_tolerates_space_separated_registers(self) -> None:
        self.assertEqual(quon_aer.clbit("1 10", 0, 3), 0)
        self.assertEqual(quon_aer.clbit("1 10", 2, 3), 1)


class NormalizeCountsTests(unittest.TestCase):
    def test_normalizes_to_probabilities(self) -> None:
        counts = {"00": 75, "11": 25}
        self.assertEqual(quon_aer.normalize_counts(counts), {"00": 0.75, "11": 0.25})

    def test_strips_spaces_and_merges_duplicate_keys(self) -> None:
        # A synthetic multi-register-style counts dict; "1 01" and "101"
        # must land on the same normalized key. This is the exact failure
        # mode flagged in the plan review: verify_distribution's point-mass
        # equivalence to grover/ising/qft's `count/SHOTS` checks only holds
        # if normalize_counts keys on the stripped bitstring.
        counts = {"1 01": 90, "1 00": 10}
        self.assertEqual(quon_aer.normalize_counts(counts), {"101": 0.9, "100": 0.1})

    def test_empty_counts_do_not_divide_by_zero(self) -> None:
        self.assertEqual(quon_aer.normalize_counts({}), {})


class HellingerFidelityTests(unittest.TestCase):
    def test_identical_distributions_have_fidelity_one(self) -> None:
        p = {"00": 0.5, "11": 0.5}
        self.assertAlmostEqual(quon_aer.hellinger_fidelity(p, p), 1.0)

    def test_disjoint_supports_have_fidelity_zero(self) -> None:
        p = {"00": 1.0}
        q = {"11": 1.0}
        self.assertAlmostEqual(quon_aer.hellinger_fidelity(p, q), 0.0)

    def test_point_mass_reduces_to_plain_probability(self) -> None:
        """The algebraic identity `verify_distribution` relies on to subsume
        grover.py/ising.py/qft.py's `count/SHOTS >= threshold` checks:
        hellinger_fidelity(p, {key: 1.0}) == p[key]."""
        p = {"11": 0.92, "10": 0.05, "01": 0.02, "00": 0.01}
        for key, prob in p.items():
            self.assertAlmostEqual(
                quon_aer.hellinger_fidelity(p, {key: 1.0}), prob, places=9
            )

    def test_point_mass_reduction_survives_space_separated_counts(self) -> None:
        """End-to-end version of the identity above starting from raw counts
        with Qiskit's space-separated multi-register key formatting."""
        counts = {"1 11": 4096 - 41, "0 00": 41}
        observed = quon_aer.normalize_counts(counts)
        fidelity = quon_aer.hellinger_fidelity(observed, {"111": 1.0})
        self.assertAlmostEqual(fidelity, (4096 - 41) / 4096, places=9)


class QuoncNotFoundErrorTests(unittest.TestCase):
    def test_compile_to_qasm_raises_actionable_error_for_missing_binary(self) -> None:
        with mock.patch.dict(os.environ, {"QUONC": "definitely-not-a-real-quonc-binary"}):
            with self.assertRaises(quon_aer.QuoncNotFoundError) as ctx:
                quon_aer.compile_to_qasm("program.qn")
        message = str(ctx.exception)
        self.assertIn("definitely-not-a-real-quonc-binary", message)
        self.assertIn("QUONC", message)
        self.assertIn("cargo build --release -p quonc", message)

    def test_compile_to_qasm_raises_actionable_error_on_nonzero_exit(self) -> None:
        with mock.patch(
            "quon_aer.subprocess.run",
            side_effect=subprocess.CalledProcessError(
                returncode=1, cmd=["quonc"], stderr="type error: ..."
            ),
        ):
            with self.assertRaises(quon_aer.QuoncCompileError) as ctx:
                quon_aer.compile_to_qasm("bad.qn")
        self.assertIn("bad.qn", str(ctx.exception))
        self.assertIn("type error", str(ctx.exception))


class SimulationDependencyMissingErrorTests(unittest.TestCase):
    def test_message_names_exact_pip_package_and_both_spellings(self) -> None:
        err = quon_aer.SimulationDependencyMissingError("qiskit-qasm3-import")
        message = str(err)
        self.assertIn("qiskit-qasm3-import", message)
        self.assertIn("qiskit_qasm3_import", message)
        self.assertIn("pip install -r python/requirements.txt", message)

    @unittest.skipUnless(HAS_QISKIT, "requires qiskit to exercise the real import path")
    def test_load_circuit_wraps_missing_qasm3_importer(self) -> None:
        from qiskit.exceptions import MissingOptionalLibraryError

        with mock.patch(
            "qiskit.qasm3.loads",
            side_effect=MissingOptionalLibraryError(
                libname="qiskit_qasm3_import", name="loading from OpenQASM 3"
            ),
        ):
            with self.assertRaises(quon_aer.SimulationDependencyMissingError):
                quon_aer.load_circuit("OPENQASM 3.0; qubit[1] q;")

    @unittest.skipUnless(HAS_QISKIT, "requires qiskit to exercise the real import path")
    def test_load_circuit_does_not_swallow_unrelated_import_errors(self) -> None:
        """An ImportError unrelated to the qasm3 importer must propagate
        unchanged, not get relabeled as a missing-dependency message."""
        with mock.patch(
            "qiskit.qasm3.loads",
            side_effect=ImportError("something unrelated exploded"),
        ):
            with self.assertRaises(ImportError) as ctx:
                quon_aer.load_circuit("OPENQASM 3.0; qubit[1] q;")
        self.assertNotIsInstance(ctx.exception, quon_aer.SimulationDependencyMissingError)
        self.assertIn("something unrelated exploded", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
