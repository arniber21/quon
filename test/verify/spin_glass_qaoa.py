#!/usr/bin/env python3
"""Dense weighted spin-glass QAOA QASM workload verification.

This verifier is intentionally compile/parse/shape-oriented rather than a
large Aer sampling test. The source is a p=10, 12-qubit weighted Ising QAOA
ansatz that should expand to thousands of OpenQASM statements after Rzz
decomposition. That makes it useful for stressing Quon's elaboration, pass
pipeline, physical lowering, scheduling, and OpenQASM emitter without turning
the default verifier suite into a slow statevector benchmark.

Run:  QUONC=target/debug/quonc python test/verify/spin_glass_qaoa.py
"""

import os
import re
import sys

REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
sys.path.insert(0, os.path.join(REPO_ROOT, "python"))

import quon_aer  # noqa: E402

SOURCE = os.path.join(REPO_ROOT, "test", "verify", "spin_glass_qaoa.qn")
EXPECTED_QUBITS = 12
EXPECTED_MEASUREMENTS = 12
MIN_QUANTUM_OPS = 2_000
EXPECTED_CX = 10 * 66 * 2
EXPECTED_RZ = 10 * (66 + 12)
EXPECTED_RX = 10 * 12
EXPECTED_H = 12


def op_counts(qasm: str) -> dict[str, int]:
    counts: dict[str, int] = {}
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

    quantum_ops = sum(counts.get(name, 0) for name in ("h", "rx", "rz", "cx"))
    print(
        "qasm shape: "
        f"{len(qasm.splitlines())} lines, {quantum_ops} quantum ops, "
        f"h={counts.get('h', 0)}, rx={counts.get('rx', 0)}, "
        f"rz={counts.get('rz', 0)}, cx={counts.get('cx', 0)}, "
        f"measure={counts.get('measure', 0)}"
    )

    failures = []
    if f"qubit[{EXPECTED_QUBITS}]" not in qasm:
        failures.append(f"expected qubit[{EXPECTED_QUBITS}] declaration")
    if counts.get("measure", 0) != EXPECTED_MEASUREMENTS:
        failures.append(f"expected {EXPECTED_MEASUREMENTS} measurements")
    if quantum_ops < MIN_QUANTUM_OPS:
        failures.append(f"expected at least {MIN_QUANTUM_OPS} emitted quantum ops")
    if counts.get("h", 0) != EXPECTED_H:
        failures.append(f"expected {EXPECTED_H} initial Hadamards")
    if counts.get("rx", 0) != EXPECTED_RX:
        failures.append(f"expected {EXPECTED_RX} mixer Rx rotations")
    if counts.get("rz", 0) != EXPECTED_RZ:
        failures.append(f"expected {EXPECTED_RZ} Rz rotations from fields and Rzz decompositions")
    if counts.get("cx", 0) != EXPECTED_CX:
        failures.append(f"expected {EXPECTED_CX} CNOTs from Rzz decompositions")

    # Qiskit's OpenQASM 3 importer is much faster than simulating this workload,
    # and still catches malformed output from the emitter. Goes through
    # quon_aer.load_circuit (dialect-normalize + qasm3.loads) rather than a
    # raw qasm3.loads(qasm) call, per issue #204: raw piping into Qiskit
    # without the compat rewrite is unsupported, even for a fixture that
    # happens not to need it today (no mid-circuit bit conditions).
    parsed_with_qiskit = False
    try:
        quon_aer.load_circuit(qasm)
        parsed_with_qiskit = True
    except quon_aer.SimulationDependencyMissingError as exc:
        print(f"NOTE: skipped OpenQASM importer parse check: {exc}")
    except Exception as exc:  # pragma: no cover - only used for CLI diagnostics
        failures.append(f"Qiskit failed to parse emitted OpenQASM 3: {exc}")

    if failures:
        for failure in failures:
            print(f"FAIL: {failure}")
        return 1

    if parsed_with_qiskit:
        print("PASS: dense spin-glass QAOA emitted a large, parseable OpenQASM workload")
    else:
        print("PASS: dense spin-glass QAOA emitted a large OpenQASM workload")
    return 0


if __name__ == "__main__":
    sys.exit(main())
