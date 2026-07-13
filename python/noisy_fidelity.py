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
import sys
import tempfile
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(REPO_ROOT / "python"))

import quon_aer  # noqa: E402

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


def compile_with(source: Path, target: Path, gamma: float | None) -> str:
    extra = ["--sabre-gamma", str(gamma)] if gamma is not None else None
    return quon_aer.compile_to_qasm(str(source), target=str(target), extra_args=extra)


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
                ideal = quon_aer.normalize_counts(
                    quon_aer.run_on_aer(qasm_a2a, shots=args.shots, seed=args.seed)
                )
                noisy_a2a = quon_aer.normalize_counts(
                    quon_aer.run_on_aer(
                        qasm_a2a, shots=args.shots, seed=args.seed, noise_model=noise_ibm
                    )
                )
                noisy_ibm = quon_aer.normalize_counts(
                    quon_aer.run_on_aer(
                        qasm_ibm, shots=args.shots, seed=args.seed, noise_model=noise_ibm
                    )
                )
            except Exception as exc:  # noqa: BLE001
                print(f"{name:<22} ERROR simulate: {exc}")
                continue

            f_a2a = quon_aer.hellinger_fidelity(ideal, noisy_a2a)
            f_ibm = quon_aer.hellinger_fidelity(ideal, noisy_ibm)
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
