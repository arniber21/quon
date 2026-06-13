"""
Qiskit Aer verification bridge — see issue #29, SPEC.md §9.2

Usage:
    quonc --emit-qasm program.qn | python python/quon_aer.py --shots 4096
    python python/quon_aer.py program.qn --shots 4096
"""

import argparse
import subprocess
import sys


def compile_to_qasm(source_file: str) -> str:
    result = subprocess.run(
        ["quonc", "--emit-qasm", source_file],
        capture_output=True, text=True, check=True,
    )
    return result.stdout


def run(qasm_src: str, shots: int = 4096) -> dict:
    from qiskit import qasm3
    from qiskit_aer import AerSimulator

    circuit = qasm3.loads(qasm_src)
    circuit.measure_all()
    sim = AerSimulator()
    job = sim.run(circuit, shots=shots)
    return job.result().get_counts()


def main():
    parser = argparse.ArgumentParser(description="Run a compiled Quon program on Qiskit Aer")
    parser.add_argument("source", nargs="?", help=".qn source file (omit to read QASM from stdin)")
    parser.add_argument("--shots", type=int, default=4096)
    args = parser.parse_args()

    if args.source:
        qasm_src = compile_to_qasm(args.source)
    else:
        qasm_src = sys.stdin.read()

    counts = run(qasm_src, shots=args.shots)
    for bitstring, count in sorted(counts.items(), key=lambda x: -x[1]):
        print(f"{bitstring}: {count}")


if __name__ == "__main__":
    main()
