#!/usr/bin/env python3
from __future__ import annotations

"""
Quon QEC Stim/Sinter evaluation harness (issue #253; ADR-0022 / ADR-0024).

Loads a compiler dual-emit pair (`*.qec.json` + sibling structure-only `.stim`),
annotates physical noise from the JSON `error_model` in Python, then samples
logical failures with Stim + Sinter/pymatching decoding.

    quonc examples/na_qec/repetition_d3_memory.qn \\
      --target targets/neutral_atom/generic_rna_v0.json \\
      --emit-qec-experiment /tmp/rep_d3.qec.json

    python python/quon_qec_sinter.py /tmp/rep_d3.qec.json --shots 64 --seed 7

Compiler resource reports and this CSV are separate artifacts (ADR-0020).
Sampled logical failure rates are not threshold claims.
"""

import argparse
import csv
import json
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Sequence, TextIO

# ---------------------------------------------------------------------------
# Optional deps — actionable install hint (mirrors quon_aer style)
# ---------------------------------------------------------------------------


class HarnessError(Exception):
    """Base class for harness failures with actionable messages."""


class DependencyMissingError(HarnessError):
    def __init__(self, package: str, cause: Exception | None = None) -> None:
        super().__init__(
            f"{package} is not installed (required for the QEC Stim/Sinter "
            "harness, issue #253 / ADR-0022).\n"
            "Install evaluation dependencies with:\n"
            "  pip install -r python/requirements.txt\n"
            "  # or: just setup-python"
        )
        if cause is not None:
            self.__cause__ = cause


class ExperimentLoadError(HarnessError):
    """Malformed or incomplete `*.qec.json` / sibling `.stim` pair."""


def _require_stim_stack():
    try:
        import numpy  # noqa: F401
        import pymatching  # noqa: F401
        import sinter
        import stim
    except ImportError as exc:
        missing = getattr(exc, "name", None) or "stim/sinter/pymatching"
        raise DependencyMissingError(missing, cause=exc) from exc
    return stim, sinter, numpy


# Coarse idle duration assumed per Stim TICK when mapping idle_per_us.
# Structure-level `.stim` has no schedule µs; this is a documented proxy only.
DEFAULT_TICK_US = 1.0

CSV_COLUMNS = [
    "distance",
    "rounds",
    "shots",
    "rydberg",
    "measurement",
    "reset",
    "movement",
    "transfer",
    "idle_per_us",
    "logical_failures",
    "logical_failure_rate",
]

ERROR_MODEL_KEYS = (
    "rydberg",
    "measurement",
    "reset",
    "movement",
    "transfer",
    "idle_per_us",
)


@dataclass(frozen=True)
class SampleResult:
    shots: int
    logical_failures: int

    @property
    def logical_failure_rate(self) -> float:
        return self.logical_failures / self.shots if self.shots else 0.0


@dataclass(frozen=True)
class ResultRow:
    distance: int
    rounds: int
    shots: int
    error_model: dict[str, float]
    logical_failures: int
    logical_failure_rate: float


# ---------------------------------------------------------------------------
# Load experiment JSON + sibling Stim
# ---------------------------------------------------------------------------


def sibling_stim_path(json_path: Path, stim_file: str | None = None) -> Path:
    """Resolve the sibling `.stim` path next to a `*.qec.json`."""
    if stim_file:
        return json_path.with_name(stim_file)
    name = json_path.name
    if name.endswith(".qec.json"):
        stem = name[: -len(".qec.json")]
    elif name.endswith(".json"):
        stem = name[: -len(".json")]
    else:
        stem = json_path.stem
    return json_path.with_name(f"{stem}.stim")


def load_experiment(json_path: str | Path):
    """Load `*.qec.json` and its sibling structure-only `.stim` circuit.

    Returns `(experiment_dict, stim.Circuit)`. Physical noise is not present
    in the circuit (ADR-0024); call `annotate_noise` before sampling.
    """
    stim, _sinter, _np = _require_stim_stack()
    path = Path(json_path)
    try:
        doc = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        raise ExperimentLoadError(f"failed to read experiment JSON {path}: {exc}") from exc

    if not isinstance(doc, dict):
        raise ExperimentLoadError(f"{path}: expected a JSON object")
    if doc.get("kind") != "qec_experiment":
        raise ExperimentLoadError(
            f"{path}: kind must be 'qec_experiment' (got {doc.get('kind')!r})"
        )
    if doc.get("schema_version") != 1:
        raise ExperimentLoadError(
            f"{path}: unsupported schema_version {doc.get('schema_version')!r}"
        )
    error_model = doc.get("error_model")
    if not isinstance(error_model, dict):
        raise ExperimentLoadError(f"{path}: missing error_model object")
    for key in ERROR_MODEL_KEYS:
        if key not in error_model:
            raise ExperimentLoadError(f"{path}: error_model missing {key!r}")

    stim_path = sibling_stim_path(path, doc.get("stim_file"))
    if not stim_path.is_file():
        raise ExperimentLoadError(
            f"{path}: sibling Stim file not found at {stim_path}.\n"
            "Emit both artifacts together:\n"
            "  quonc <source.qn> --target <na.json> "
            "--emit-qec-experiment <path.qec.json>"
        )
    try:
        circuit = stim.Circuit(stim_path.read_text())
    except Exception as exc:  # stim raises various parse errors
        raise ExperimentLoadError(f"failed to parse Stim circuit {stim_path}: {exc}") from exc

    return doc, circuit


# ---------------------------------------------------------------------------
# Noise annotation from error_model (ADR-0024)
# ---------------------------------------------------------------------------


def _idle_probability(idle_per_us: float, tick_us: float = DEFAULT_TICK_US) -> float:
    """Map per-µs idle rate to a single-TICK DEPOLARIZE1 probability."""
    if idle_per_us <= 0.0 or tick_us <= 0.0:
        return 0.0
    # 1 - (1-p)^t, clamped so Stim accepts the probability.
    p = 1.0 - (1.0 - idle_per_us) ** tick_us
    return min(0.5, max(0.0, p))


def annotate_noise(circuit, error_model: dict[str, float], *, tick_us: float = DEFAULT_TICK_US):
    """Insert Stim noise channels from a JSON `error_model` snapshot.

    Mapping (structure-level `.stim` has no schedule µs — proxies are documented):

    - `rydberg`     → DEPOLARIZE2 after each CX (two-qubit Rydberg gate)
    - `measurement` → X_ERROR before M / MR / MX / MZ
    - `reset`       → X_ERROR after R and after MR
    - `transfer`    → DEPOLARIZE1 after R (load / zone transfer proxy)
    - `movement`    → DEPOLARIZE1 after each TICK (between-layer motion proxy)
    - `idle_per_us` → DEPOLARIZE1 after each TICK with p from ``tick_us``

    Zero rates omit the corresponding channel. Detectors/observables are preserved.
    """
    stim, _sinter, _np = _require_stim_stack()
    p_ryd = float(error_model["rydberg"])
    p_meas = float(error_model["measurement"])
    p_reset = float(error_model["reset"])
    p_move = float(error_model["movement"])
    p_xfer = float(error_model["transfer"])
    p_idle = _idle_probability(float(error_model["idle_per_us"]), tick_us=tick_us)

    out = stim.Circuit()
    all_qubits = list(range(circuit.num_qubits))

    for inst in circuit:
        name = inst.name
        targets = [t.value for t in inst.targets_copy() if t.is_qubit_target]

        if name in ("M", "MR", "MX", "MZ"):
            if p_meas > 0.0 and targets:
                out.append("X_ERROR", targets, [p_meas])
            out.append(inst)
            if name == "MR" and p_reset > 0.0 and targets:
                out.append("X_ERROR", targets, [p_reset])
            continue

        if name == "R":
            out.append(inst)
            if p_reset > 0.0 and targets:
                out.append("X_ERROR", targets, [p_reset])
            if p_xfer > 0.0 and targets:
                out.append("DEPOLARIZE1", targets, [p_xfer])
            continue

        if name == "CX":
            out.append(inst)
            if p_ryd > 0.0 and targets:
                out.append("DEPOLARIZE2", targets, [p_ryd])
            continue

        if name == "TICK":
            out.append(inst)
            if all_qubits:
                if p_move > 0.0:
                    out.append("DEPOLARIZE1", all_qubits, [p_move])
                if p_idle > 0.0:
                    out.append("DEPOLARIZE1", all_qubits, [p_idle])
            continue

        out.append(inst)

    return out


def scale_error_model(error_model: dict[str, float], scale: float) -> dict[str, float]:
    """Multiply every physical error parameter by ``scale`` (clamped to [0, 1])."""
    out: dict[str, float] = {}
    for key in ERROR_MODEL_KEYS:
        out[key] = min(1.0, max(0.0, float(error_model[key]) * scale))
    return out


# ---------------------------------------------------------------------------
# Sampling (Stim detector sampler + Sinter/pymatching decode)
# ---------------------------------------------------------------------------


def sample_logical_failures(
    noisy_circuit,
    *,
    shots: int,
    seed: int,
    decoder: str = "pymatching",
) -> SampleResult:
    """Sample detection events and count logical failures after decoding.

    Uses Stim's detector sampler with a fixed ``seed`` for determinism, then
    Sinter's ``predict_observables`` with ``decoder`` (default pymatching).
    """
    _stim, sinter, np = _require_stim_stack()
    if shots < 1:
        raise HarnessError(f"shots must be >= 1 (got {shots})")

    dem = noisy_circuit.detector_error_model(decompose_errors=True)
    sampler = noisy_circuit.compile_detector_sampler(seed=seed)
    dets, obs = sampler.sample(shots=shots, separate_observables=True)
    predictions = sinter.predict_observables(dem=dem, dets=dets, decoder=decoder)
    failures = int(np.sum(np.any(predictions != obs, axis=1)))
    return SampleResult(shots=shots, logical_failures=failures)


# ---------------------------------------------------------------------------
# CSV emit
# ---------------------------------------------------------------------------


def write_csv(out: TextIO, rows: Sequence[ResultRow]) -> None:
    """Write result rows. Does not claim thresholds (ADR-0020)."""
    writer = csv.DictWriter(out, fieldnames=CSV_COLUMNS)
    writer.writeheader()
    for row in rows:
        record = {
            "distance": row.distance,
            "rounds": row.rounds,
            "shots": row.shots,
            "logical_failures": row.logical_failures,
            "logical_failure_rate": row.logical_failure_rate,
        }
        for key in ERROR_MODEL_KEYS:
            record[key] = row.error_model[key]
        writer.writerow(record)


# ---------------------------------------------------------------------------
# Orchestration
# ---------------------------------------------------------------------------


def run_experiments(
    json_paths: Sequence[str | Path],
    *,
    shots_list: Sequence[int],
    seed: int,
    error_scales: Sequence[float] = (1.0,),
    decoder: str = "pymatching",
    tick_us: float = DEFAULT_TICK_US,
) -> list[ResultRow]:
    """Load each experiment, annotate noise, sample, and collect CSV rows."""
    rows: list[ResultRow] = []
    for json_path in json_paths:
        experiment, structure = load_experiment(json_path)
        base_model = {k: float(experiment["error_model"][k]) for k in ERROR_MODEL_KEYS}
        distance = int(experiment["distance"])
        rounds = int(experiment["rounds"])
        for scale in error_scales:
            model = scale_error_model(base_model, scale)
            noisy = annotate_noise(structure, model, tick_us=tick_us)
            for shots in shots_list:
                sample = sample_logical_failures(
                    noisy, shots=shots, seed=seed, decoder=decoder
                )
                rows.append(
                    ResultRow(
                        distance=distance,
                        rounds=rounds,
                        shots=sample.shots,
                        error_model=model,
                        logical_failures=sample.logical_failures,
                        logical_failure_rate=sample.logical_failure_rate,
                    )
                )
    return rows


def _parse_float_list(text: str) -> list[float]:
    parts = [p.strip() for p in text.split(",") if p.strip()]
    if not parts:
        raise argparse.ArgumentTypeError("expected at least one number")
    return [float(p) for p in parts]


def _parse_int_list(text: str) -> list[int]:
    parts = [p.strip() for p in text.split(",") if p.strip()]
    if not parts:
        raise argparse.ArgumentTypeError("expected at least one integer")
    values = [int(p) for p in parts]
    if any(v < 1 for v in values):
        raise argparse.ArgumentTypeError("values must be >= 1")
    return values


def build_arg_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        formatter_class=argparse.RawDescriptionHelpFormatter,
        description=(
            "Sample logical failures for Quon QEC experiment artifacts "
            "(`*.qec.json` + sibling structure `.stim`) via Stim/Sinter.\n"
            "Noise is annotated in Python from JSON error_model (ADR-0024).\n"
            "Output CSV is a sampled artifact only — not a threshold claim "
            "(ADR-0020)."
        ),
        epilog="""
Distance / round sweeps
  Distance and memory-round count are baked into the structure `.stim` by
  quonc. To sweep those axes, re-invoke quonc --emit-qec-experiment for each
  (distance, rounds) point (edit or generate the .qn source), then pass the
  resulting *.qec.json paths to this harness. Prefer re-emit over rewriting
  .stim by hand.

Shot / physical-error sweeps
  --shots 16,256,1024 samples the same annotated circuit at multiple shot
  counts without re-emitting.
  --scale-errors 0.5,1,2 multiplies every error_model rate (still no re-emit).

Local larger runs
  CI smoke uses tiny shots and a fixed seed, e.g.:
    python python/quon_qec_sinter.py /tmp/rep_d3.qec.json --shots 32 --seed 7

  Larger local evaluation (not for CI):
    python python/quon_qec_sinter.py /tmp/rep_d3.qec.json \\
      --shots 10000 --seed 7 --csv /tmp/rep_d3_sinter.csv
    python python/quon_qec_sinter.py a.qec.json b.qec.json \\
      --shots 1000,10000 --scale-errors 0.5,1,2 --csv /tmp/sweep.csv
""",
    )
    parser.add_argument(
        "experiments",
        nargs="+",
        help="one or more *.qec.json paths (sibling .stim required beside each)",
    )
    parser.add_argument(
        "--shots",
        type=_parse_int_list,
        default=[32],
        help="comma-separated shot counts (default: 32)",
    )
    parser.add_argument(
        "--seed",
        type=int,
        default=7,
        help="Stim detector-sampler seed for deterministic CI smoke (default: 7)",
    )
    parser.add_argument(
        "--scale-errors",
        type=_parse_float_list,
        default=[1.0],
        help="comma-separated multipliers applied to all error_model rates",
    )
    parser.add_argument(
        "--decoder",
        default="pymatching",
        help="Sinter decoder name (default: pymatching)",
    )
    parser.add_argument(
        "--tick-us",
        type=float,
        default=DEFAULT_TICK_US,
        help=(
            f"assumed µs per Stim TICK when mapping idle_per_us "
            f"(default: {DEFAULT_TICK_US})"
        ),
    )
    parser.add_argument(
        "--csv",
        type=str,
        default=None,
        help="write CSV to this path (default: stdout)",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    parser = build_arg_parser()
    args = parser.parse_args(argv)
    try:
        rows = run_experiments(
            args.experiments,
            shots_list=args.shots,
            seed=args.seed,
            error_scales=args.scale_errors,
            decoder=args.decoder,
            tick_us=args.tick_us,
        )
    except HarnessError as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 1

    if args.csv:
        with open(args.csv, "w", newline="") as f:
            write_csv(f, rows)
    else:
        write_csv(sys.stdout, rows)
    return 0


if __name__ == "__main__":
    sys.exit(main())
