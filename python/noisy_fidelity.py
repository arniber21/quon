#!/usr/bin/env python3
"""Before/after noisy Hellinger fidelity for the PRD verify corpus (issue #117).

Compares each program under:
  1. all-to-all topology + the same noise model (no routing pressure)
  2. the full IBM target (topology + noise; SABRE may insert SWAPs)

Both are scored with Hellinger fidelity against an ideal (noise-free) Aer run
of the all-to-all compilation. Report-only — does not fail CI.

Requires qiskit + qiskit-aer (+ qiskit-qasm3-import). If unavailable, see
`targets/ibm/sample_fidelity_results.md` for a checked-in sample table.

Usage:
    QUONC=target/debug/quonc python python/noisy_fidelity.py
    python python/noisy_fidelity.py --target targets/ibm/fake_manila_v2.json
"""

from __future__ import annotations

import argparse
import json
import math
import subprocess
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT / "python"))

PROGRAMS = [
    "bell",
    "teleport",
    "bernstein_vazirani",
    "grover",
    "qft",
    "ising",
    "qaoa",
    "shor",
]


def hellinger_fidelity(p: dict[str, float], q: dict[str, float]) -> float:
    keys = set(p) | set(q)
    return sum(math.sqrt(p.get(k, 0.0) * q.get(k, 0.0)) for k in keys) ** 2


def normalize(counts: dict[str, int]) -> dict[str, float]:
    total = sum(counts.values()) or 1
    return {k: v / total for k, v in counts.items()}


def load_target(path: Path) -> dict:
    return json.loads(path.read_text())


def all_to_all_with_noise(ibm: dict, path: Path) -> None:
    """Write a temporary all-to-all target that reuses the IBM noise model."""
    n = int(ibm["num_qubits"])
    edges = [[i, j] for i in range(n) for j in range(i + 1, n)]
    payload = {
        "id": f"{ibm.get('id', 'ibm')}_all_to_all_noise",
        "num_qubits": n,
        "topology": {"edges": edges},
        "native_gates": list(ibm.get("native_gates", ["cx", "rz", "sx", "x"])),
        "noise": ibm.get("noise", {}),
        "meas_latency_us": ibm.get("meas_latency_us", 0.0),
        "supports_mid_circuit_meas": ibm.get("supports_mid_circuit_meas", True),
        "supports_feed_forward": ibm.get("supports_feed_forward", True),
    }
    path.write_text(json.dumps(payload, indent=2) + "\n")


def build_noise_model(target: dict):
    """Construct an Aer NoiseModel from Quon NoiseDescriptor fields."""
    from qiskit_aer.noise import NoiseModel, ReadoutError, depolarizing_error

    noise = NoiseModel()
    desc = target.get("noise") or {}

    for gate, per_q in (desc.get("single_qubit_fidelity") or {}).items():
        for q_str, fid in per_q.items():
            err = max(0.0, min(1.0, 1.0 - float(fid)))
            if err <= 0.0:
                continue
            noise.add_quantum_error(depolarizing_error(err, 1), gate, [int(q_str)])

    for gate, per_pair in (desc.get("two_qubit_fidelity") or {}).items():
        for pair, fid in per_pair.items():
            a_s, b_s = pair.split(",")
            a, b = int(a_s), int(b_s)
            err = max(0.0, min(1.0, 1.0 - float(fid)))
            if err <= 0.0:
                continue
            noise.add_quantum_error(depolarizing_error(err, 2), gate, [a, b])

    for q_str, ro in (desc.get("readout_error") or {}).items():
        e = float(ro)
        # Symmetric classical readout flip approximation.
        noise.add_readout_error(ReadoutError([[1 - e, e], [e, 1 - e]]), [int(q_str)])

    return noise


def run_counts(qasm: str, shots: int, seed: int, noise_model=None) -> dict[str, int]:
    import quon_aer
    from qiskit import qasm3
    from qiskit_aer import AerSimulator

    circuit = qasm3.loads(quon_aer._qiskit_qasm3_compat(qasm))
    if circuit.num_clbits == 0:
        circuit.measure_all()
    sim = AerSimulator(noise_model=noise_model) if noise_model is not None else AerSimulator()
    job = sim.run(circuit, shots=shots, seed_simulator=seed)
    return dict(job.result().get_counts())


def compile_program(source: Path, target: Path | None) -> str:
    import quon_aer

    return quon_aer.compile_to_qasm(str(source), str(target) if target else None)


def compile_with(source: Path, target: Path, gamma: float | None) -> str:
    import quon_aer

    if gamma is None:
        return compile_program(source, target)
    cmd = [
        quon_aer.quonc_binary(),
        "--emit-qasm",
        "--target",
        str(target),
        "--sabre-gamma",
        str(gamma),
        str(source),
    ]
    result = subprocess.run(cmd, capture_output=True, text=True, check=True)
    return result.stdout


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--target",
        type=Path,
        default=REPO_ROOT / "targets" / "ibm" / "fake_manila_v2.json",
        help="IBM BackendTarget JSON",
    )
    parser.add_argument("--shots", type=int, default=4096)
    parser.add_argument("--seed", type=int, default=1234)
    parser.add_argument(
        "--sabre-gamma",
        type=float,
        default=None,
        help="Forwarded to quonc when compiling against the IBM target",
    )
    args = parser.parse_args(argv)

    try:
        import qiskit  # noqa: F401
        import qiskit_aer  # noqa: F401
    except ImportError:
        print(
            "qiskit/qiskit-aer not installed; cannot run live fidelity sims.\n"
            "See targets/ibm/sample_fidelity_results.md for a checked-in sample.\n"
            "Install: pip install -r python/requirements.txt",
            file=sys.stderr,
        )
        return 2

    ibm = load_target(args.target)
    verify_dir = REPO_ROOT / "test" / "verify"

    rows: list[dict[str, object]] = []
    with tempfile.TemporaryDirectory() as tmp:
        all_to_all_path = Path(tmp) / "all_to_all_noise.json"
        all_to_all_with_noise(ibm, all_to_all_path)
        noise_ibm = build_noise_model(ibm)

        print(f"{'program':<22} {'F_all2all':>10} {'F_ibm':>10} {'delta':>10}")
        print("-" * 56)

        for name in PROGRAMS:
            source = verify_dir / f"{name}.qn"
            if not source.exists():
                print(f"{name:<22} SKIP (missing {source.name})")
                continue
            try:
                qasm_a2a = compile_with(source, all_to_all_path, None)
                qasm_ibm = compile_with(source, args.target, args.sabre_gamma)
            except Exception as exc:  # noqa: BLE001 — report-only tool
                print(f"{name:<22} ERROR compile: {exc}")
                continue

            try:
                ideal = normalize(run_counts(qasm_a2a, args.shots, args.seed, None))
                noisy_a2a = normalize(
                    run_counts(qasm_a2a, args.shots, args.seed, noise_ibm)
                )
                noisy_ibm = normalize(
                    run_counts(qasm_ibm, args.shots, args.seed, noise_ibm)
                )
            except Exception as exc:  # noqa: BLE001
                print(f"{name:<22} ERROR simulate: {exc}")
                continue

            f_a2a = hellinger_fidelity(ideal, noisy_a2a)
            f_ibm = hellinger_fidelity(ideal, noisy_ibm)
            delta = f_ibm - f_a2a
            rows.append(
                {
                    "program": name,
                    "fidelity_all_to_all_noise": f_a2a,
                    "fidelity_ibm_target": f_ibm,
                    "delta": delta,
                }
            )
            print(f"{name:<22} {f_a2a:10.4f} {f_ibm:10.4f} {delta:10.4f}")

    print()
    print(
        "Note: F_all2all = Hellinger fidelity of noisy all-to-all vs ideal; "
        "F_ibm = noisy full IBM target vs the same ideal. Lower F_ibm usually "
        "reflects SWAP overhead under limited connectivity."
    )
    return 0 if rows else 1


if __name__ == "__main__":
    raise SystemExit(main())
