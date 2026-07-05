"""
Qiskit Aer verification bridge — see issue #29, SPEC.md §9.2

Usage:
    quonc --emit-qasm program.qn | python python/quon_aer.py --shots 4096
    python python/quon_aer.py program.qn --shots 4096
"""

import argparse
import os
import re
import subprocess
import sys


def quonc_binary() -> str:
    """The quonc executable: the QUONC env var if set, else `quonc` on PATH."""
    return os.environ.get("QUONC", "quonc")


def compile_to_qasm(source_file: str, target: str | None = None) -> str:
    args = [quonc_binary(), "--emit-qasm"]
    if target is not None:
        args += ["--target", target]
    args.append(source_file)
    result = subprocess.run(args, capture_output=True, text=True, check=True)
    return result.stdout


def _qiskit_qasm3_compat(qasm_src: str) -> str:
    """Normalize spec-valid bit integer conditions for Qiskit's importer.

    quonc emits `if (c[i] == 1)` to match the OpenQASM integer-comparison
    convention used by the backend. qiskit_qasm3_import currently accepts
    indexed-bit conditions only as `bit == const bool`, while accepting integer
    comparisons for whole bit arrays. Keep quonc's output unchanged and adapt
    only this verification bridge.
    """

    def replace(match: re.Match[str]) -> str:
        bit_ref, value = match.groups()
        return f"{bit_ref} == {'true' if value == '1' else 'false'}"

    return re.sub(r"\b([A-Za-z_][A-Za-z0-9_]*\[\d+\])\s*==\s*([01])\b", replace, qasm_src)


def run(qasm_src: str, shots: int = 4096, seed: int | None = None) -> dict:
    from qiskit import qasm3
    from qiskit_aer import AerSimulator

    circuit = qasm3.loads(_qiskit_qasm3_compat(qasm_src))
    # quonc emits explicit `measure` statements into a `bit[m] c;` register, so
    # the loaded circuit already carries its measurements. Only fall back to
    # measure_all() for a purely unitary circuit with no classical bits.
    if circuit.num_clbits == 0:
        circuit.measure_all()
    sim = AerSimulator()
    # `seed` pins the sampler RNG so verification runs are reproducible; left as
    # None for the interactive CLI, where fresh randomness is fine.
    run_kwargs: dict = {"shots": shots}
    if seed is not None:
        run_kwargs["seed_simulator"] = seed
    job = sim.run(circuit, **run_kwargs)
    return job.result().get_counts()


def main():
    parser = argparse.ArgumentParser(description="Run a compiled Quon program on Qiskit Aer")
    parser.add_argument("source", nargs="?", help=".qn source file (omit to read QASM from stdin)")
    parser.add_argument("--shots", type=int, default=4096)
    parser.add_argument("--seed", type=int, default=None, help="pin the Aer sampler RNG")
    args = parser.parse_args()

    if args.source:
        qasm_src = compile_to_qasm(args.source)
    else:
        qasm_src = sys.stdin.read()

    counts = run(qasm_src, shots=args.shots, seed=args.seed)
    for bitstring, count in sorted(counts.items(), key=lambda x: -x[1]):
        print(f"{bitstring}: {count}")


if __name__ == "__main__":
    main()
