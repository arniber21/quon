#!/usr/bin/env python3
"""Toy TSP sketch -- compile/shape verification (NOT a TSP solver) (#191).

This verifier is intentionally structural, like `spin_glass_qaoa.py`: the
source is a SCHEMATIC -- a small QAOA-shaped cost Hamiltonian on 4 qubits of
the same shape a TSP-to-Ising reformulation would emit (weighted Rzz pair
couplings + Rz local fields) -- not a real TSP solver. The checker confirms
the circuit compiles, emits a properly-shaped, parseable OpenQASM workload
with the expected gate counts, and that Qiskit can ingest it. It makes NO claim
about solving TSP; tour decoding, constraint enforcement, and 2-opt are
classical and live outside Quon (documented in the .qn).

Run:  QUONC=target/debug/quonc python test/verify/tsp_sketch.py
"""

import os
import re
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SOURCE = os.path.join(REPO_ROOT, "samples", "applications", "tsp_sketch.qn")
EXPECTED_QUBITS = 4
EXPECTED_MEASUREMENTS = 4
EXPECTED_H = 4            # one Hadamard per qubit (initial superposition)
EXPECTED_RX = 4           # one mixer Rx per qubit
EXPECTED_RZ = 4 + 6       # four local constraint-penalty fields + six Rzz-decomposition Rzs
EXPECTED_CX = 6 * 2       # six weighted Rzz edges, each -> CNOT + Rz + CNOT


def op_counts(qasm):
    counts = {}
    for raw in qasm.splitlines():
        line = raw.strip()
        if not line or line.startswith("//"):
            continue
        if " measure " in f" {line} ":
            counts["measure"] = counts.get("measure", 0) + 1
        match = re.match(r"([A-Za-z_][A-Za-z0-9_]*)\b", line)
        if match:
            counts[match.group(1)] = counts.get(match.group(1), 0) + 1
    return counts


def main() -> int:
    qasm = quon_aer.compile_to_qasm(SOURCE)
    counts = op_counts(qasm)
    quantum_ops = sum(counts.get(n, 0) for n in ("h", "rx", "rz", "cx"))
    print(f"qasm shape: {len(qasm.splitlines())} lines, {quantum_ops} quantum ops, "
          f"h={counts.get('h', 0)}, rx={counts.get('rx', 0)}, rz={counts.get('rz', 0)}, "
          f"cx={counts.get('cx', 0)}, measure={counts.get('measure', 0)}")

    failures = []
    if f"qubit[{EXPECTED_QUBITS}]" not in qasm:
        failures.append(f"expected qubit[{EXPECTED_QUBITS}] declaration")
    if counts.get("measure", 0) != EXPECTED_MEASUREMENTS:
        failures.append(f"expected {EXPECTED_MEASUREMENTS} measurements")
    if counts.get("h", 0) != EXPECTED_H:
        failures.append(f"expected {EXPECTED_H} initial Hadamards")
    if counts.get("rx", 0) != EXPECTED_RX:
        failures.append(f"expected {EXPECTED_RX} mixer Rx rotations")
    if counts.get("rz", 0) != EXPECTED_RZ:
        failures.append(f"expected {EXPECTED_RZ} Rz rotations (fields + Rzz decompositions)")
    if counts.get("cx", 0) != EXPECTED_CX:
        failures.append(f"expected {EXPECTED_CX} CNOTs from Rzz decompositions")

    parsed = False
    try:
        quon_aer.load_circuit(qasm)
        parsed = True
    except quon_aer.SimulationDependencyMissingError as exc:
        print(f"NOTE: skipped OpenQASM importer parse check: {exc}")
    except Exception as exc:  # pragma: no cover - CLI diagnostics only
        failures.append(f"Qiskit failed to parse emitted OpenQASM 3: {exc}")

    if failures:
        for f in failures:
            print(f"FAIL: {f}")
        return 1
    if parsed:
        print("PASS: TSP sketch emitted a properly-shaped, parseable OpenQASM workload")
    else:
        print("PASS: TSP sketch emitted a properly-shaped OpenQASM workload")
    return 0


if __name__ == "__main__":
    sys.exit(main())
