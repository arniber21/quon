#!/usr/bin/env python3
"""Offline IBM fake-backend → Quon BackendTarget JSON converter (issue #117).

CI uses the checked-in snapshot under `targets/ibm/`. This script is for
maintainers regenerating that file from public FakeBackend fixtures or from
downloaded `conf_*.json` / `props_*.json` pairs. It never talks to live IBM
hardware and does not require an IBM token.

Examples:
    python python/ibm_snapshot_to_target.py \\
        --conf conf_manila.json --props props_manila.json \\
        --out targets/ibm/fake_manila_v2.json

    python python/ibm_snapshot_to_target.py --backend fake_manila \\
        --out targets/ibm/fake_manila_v2.json
"""

from __future__ import annotations

import argparse
import json
import sys
from pathlib import Path
from typing import Any


NATIVE_GATES_DEFAULT = ["cx", "rz", "sx", "x"]


def _param_value(parameters: list[dict[str, Any]], name: str) -> float | None:
    for param in parameters:
        if param.get("name") == name:
            return float(param["value"])
    return None


def _qubit_value(qubit_props: list[dict[str, Any]], name: str) -> float | None:
    for param in qubit_props:
        if param.get("name") == name:
            return float(param["value"])
    return None


def undirected_edges(coupling_map: list[list[int]]) -> list[list[int]]:
    seen: set[tuple[int, int]] = set()
    edges: list[list[int]] = []
    for pair in coupling_map:
        if len(pair) != 2:
            continue
        a, b = int(pair[0]), int(pair[1])
        key = (min(a, b), max(a, b))
        if key not in seen:
            seen.add(key)
            edges.append([key[0], key[1]])
    edges.sort()
    return edges


def convert(conf: dict[str, Any], props: dict[str, Any], target_id: str) -> dict[str, Any]:
    num_qubits = int(conf["n_qubits"])
    edges = undirected_edges(conf.get("coupling_map", []))

    single: dict[str, dict[str, float]] = {"sx": {}, "x": {}, "rz": {}}
    two: dict[str, dict[str, float]] = {"cx": {}}

    for gate in props.get("gates", []):
        name = gate.get("gate")
        qubits = gate.get("qubits") or []
        err = _param_value(gate.get("parameters") or [], "gate_error")
        if err is None or name is None:
            continue
        fidelity = max(0.0, min(1.0, 1.0 - float(err)))
        if name in single and len(qubits) == 1:
            single[name][str(int(qubits[0]))] = fidelity
        elif name == "cx" and len(qubits) == 2:
            two["cx"][f"{int(qubits[0])},{int(qubits[1])}"] = fidelity

    t1: dict[str, float] = {}
    t2: dict[str, float] = {}
    readout: dict[str, float] = {}
    readout_lengths_ns: list[float] = []
    for idx, qubit in enumerate(props.get("qubits", [])):
        t1_v = _qubit_value(qubit, "T1")
        t2_v = _qubit_value(qubit, "T2")
        ro = _qubit_value(qubit, "readout_error")
        length = _qubit_value(qubit, "readout_length")
        if t1_v is not None:
            t1[str(idx)] = t1_v
        if t2_v is not None:
            t2[str(idx)] = t2_v
        if ro is not None:
            readout[str(idx)] = ro
        if length is not None:
            readout_lengths_ns.append(length)

    meas_latency_us = (
        sum(readout_lengths_ns) / len(readout_lengths_ns) / 1000.0
        if readout_lengths_ns
        else 0.0
    )

    return {
        "id": target_id,
        "num_qubits": num_qubits,
        "topology": {"edges": edges},
        "native_gates": list(NATIVE_GATES_DEFAULT),
        "noise": {
            "single_qubit_fidelity": {k: v for k, v in single.items() if v},
            "two_qubit_fidelity": {k: v for k, v in two.items() if v},
            "t1_us": t1,
            "t2_us": t2,
            "readout_error": readout,
        },
        "meas_latency_us": round(meas_latency_us, 6),
        "supports_mid_circuit_meas": True,
        "supports_feed_forward": True,
    }


def load_from_files(conf_path: Path, props_path: Path) -> tuple[dict[str, Any], dict[str, Any]]:
    return (
        json.loads(conf_path.read_text()),
        json.loads(props_path.read_text()),
    )


def load_from_fake_backend(name: str) -> tuple[dict[str, Any], dict[str, Any], str]:
    """Load conf/props via qiskit-ibm-runtime FakeBackendV2 when installed."""
    try:
        from qiskit_ibm_runtime.fake_provider import FakeManilaV2
    except ImportError as exc:  # pragma: no cover - optional dependency
        raise SystemExit(
            "qiskit-ibm-runtime is not installed. Pass --conf/--props instead, "
            "or pip-install qiskit-ibm-runtime on a maintainer machine."
        ) from exc

    backends = {
        "fake_manila": FakeManilaV2,
        "fake_manila_v2": FakeManilaV2,
    }
    cls = backends.get(name.lower())
    if cls is None:
        raise SystemExit(f"unsupported --backend {name!r}; known: {sorted(backends)}")

    backend = cls()
    dirname = Path(backend.dirname)
    conf = json.loads((dirname / backend.conf_filename).read_text())
    props = json.loads((dirname / backend.props_filename).read_text())
    return conf, props, "fake_manila_v2"


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--conf", type=Path, help="IBM backend configuration JSON")
    parser.add_argument("--props", type=Path, help="IBM backend properties JSON")
    parser.add_argument(
        "--backend",
        help="Fake backend name (e.g. fake_manila); requires qiskit-ibm-runtime",
    )
    parser.add_argument(
        "--id",
        default=None,
        help="Override target id (default: fake_manila_v2 or derived)",
    )
    parser.add_argument(
        "--out",
        type=Path,
        required=True,
        help="Output BackendTarget JSON path",
    )
    parser.add_argument(
        "--raw-out",
        type=Path,
        default=None,
        help="Optional path to dump the raw conf+props bundle",
    )
    args = parser.parse_args(argv)

    if args.conf and args.props:
        conf, props = load_from_files(args.conf, args.props)
        target_id = args.id or "fake_manila_v2"
    elif args.backend:
        conf, props, default_id = load_from_fake_backend(args.backend)
        target_id = args.id or default_id
    else:
        parser.error("provide either --conf and --props, or --backend")

    target = convert(conf, props, target_id)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(target, indent=2) + "\n")
    print(f"wrote {args.out}", file=sys.stderr)

    if args.raw_out is not None:
        bundle = {
            "source": "ibm fake-backend conf+props",
            "conf": conf,
            "props": props,
        }
        args.raw_out.parent.mkdir(parents=True, exist_ok=True)
        args.raw_out.write_text(json.dumps(bundle, indent=2) + "\n")
        print(f"wrote raw dump {args.raw_out}", file=sys.stderr)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
