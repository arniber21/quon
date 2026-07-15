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
import math
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Mapping, Sequence, TextIO

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
# Structure-level `.stim` has no schedule µs; --tick-us is a documented proxy
# only — not wall-clock schedule time from the NA planner.
DEFAULT_TICK_US = 1.0

# Stim-legal probability maxima for noise channels used by annotate_noise.
# Never clamp depolarizing rates to 1.0 (illegal for DEM construction).
STIM_DEPOLARIZE1_MAX = 3.0 / 4.0
STIM_DEPOLARIZE2_MAX = 15.0 / 16.0
STIM_PAULI_ERROR_MAX = 1.0

# Per error_model key → Stim channel cap used by scale_error_model / validation.
# idle_per_us is a per-µs rate; the converted DEPOLARIZE1 is capped separately.
ERROR_MODEL_STIM_MAX: dict[str, float] = {
    "rydberg": STIM_DEPOLARIZE2_MAX,  # → DEPOLARIZE2
    "measurement": STIM_PAULI_ERROR_MAX,  # → X_ERROR / Z_ERROR
    "reset": STIM_PAULI_ERROR_MAX,  # → X_ERROR
    "movement": STIM_DEPOLARIZE1_MAX,  # → DEPOLARIZE1 (composed with idle)
    "transfer": STIM_DEPOLARIZE1_MAX,  # → DEPOLARIZE1
    "idle_per_us": STIM_PAULI_ERROR_MAX,  # rate; converted p capped at DEPOLARIZE1
}

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


def _require_positive_int(doc: Mapping[str, Any], key: str, path: Path) -> int:
    if key not in doc:
        raise ExperimentLoadError(f"{path}: missing required field {key!r}")
    value = doc[key]
    if isinstance(value, bool) or not isinstance(value, int):
        raise ExperimentLoadError(
            f"{path}: {key} must be a positive integer (got {value!r})"
        )
    if value < 1:
        raise ExperimentLoadError(f"{path}: {key} must be >= 1 (got {value})")
    return value


def _require_probability(value: Any, *, key: str, path: Path) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ExperimentLoadError(
            f"{path}: error_model[{key!r}] must be a number (got {value!r})"
        )
    p = float(value)
    if not math.isfinite(p):
        raise ExperimentLoadError(
            f"{path}: error_model[{key!r}] must be finite (got {p!r})"
        )
    if p < 0.0 or p > 1.0:
        raise ExperimentLoadError(
            f"{path}: error_model[{key!r}] must be in [0, 1] (got {p})"
        )
    return p


def _validate_stim_channel_probability(p: float, *, channel: str, maximum: float) -> None:
    """Reject probabilities Stim cannot accept for the named channel."""
    if not math.isfinite(p) or p < 0.0 or p > maximum:
        raise HarnessError(
            f"Stim-illegal {channel} probability {p}: must be in [0, {maximum}] "
            f"(DEPOLARIZE1 max {STIM_DEPOLARIZE1_MAX}, "
            f"DEPOLARIZE2 max {STIM_DEPOLARIZE2_MAX})"
        )


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

    distance = _require_positive_int(doc, "distance", path)
    rounds = _require_positive_int(doc, "rounds", path)
    doc["distance"] = distance
    doc["rounds"] = rounds

    error_model = doc.get("error_model")
    if not isinstance(error_model, dict):
        raise ExperimentLoadError(f"{path}: missing error_model object")
    validated: dict[str, float] = {}
    for key in ERROR_MODEL_KEYS:
        if key not in error_model:
            raise ExperimentLoadError(f"{path}: error_model missing {key!r}")
        validated[key] = _require_probability(error_model[key], key=key, path=path)
        # Direct Stim-channel keys: reject rates above Stim-legal maxima early.
        if key in ("rydberg", "movement", "transfer", "measurement", "reset"):
            _validate_stim_channel_probability(
                validated[key],
                channel=key,
                maximum=ERROR_MODEL_STIM_MAX[key],
            )
    doc["error_model"] = validated

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
    """Map per-µs idle rate to a single-TICK DEPOLARIZE1 probability.

    ``tick_us`` is a harness proxy (see ``--tick-us``); structure `.stim` has
    no schedule microseconds from the NA planner.
    """
    if idle_per_us <= 0.0 or tick_us <= 0.0:
        return 0.0
    # 1 - (1-p)^t, clamped to Stim-legal DEPOLARIZE1 maximum.
    p = 1.0 - (1.0 - idle_per_us) ** tick_us
    return min(STIM_DEPOLARIZE1_MAX, max(0.0, p))


def _compose_depolarize1(p_a: float, p_b: float) -> float:
    """Compose two independent DEPOLARIZE1 probabilities into one channel."""
    if p_a <= 0.0 and p_b <= 0.0:
        return 0.0
    p = 1.0 - (1.0 - max(0.0, p_a)) * (1.0 - max(0.0, p_b))
    return min(STIM_DEPOLARIZE1_MAX, max(0.0, p))


def annotate_noise(circuit, error_model: dict[str, float], *, tick_us: float = DEFAULT_TICK_US):
    """Insert Stim noise channels from a JSON `error_model` snapshot.

    Mapping (structure-level `.stim` has no schedule µs — proxies are documented):

    - `rydberg`     → DEPOLARIZE2 after each CX (two-qubit Rydberg gate)
    - `measurement` → X_ERROR before M / MR / MZ; Z_ERROR before MX
    - `reset`       → X_ERROR after R and after MR
    - `transfer`    → DEPOLARIZE1 after R (load / zone transfer proxy)
    - `movement` + `idle_per_us` → one composed DEPOLARIZE1 after each TICK
      (``--tick-us`` converts idle_per_us; not NA schedule wall-clock)

    Zero rates omit the corresponding channel. Detectors/observables are preserved.
    Top-level ``REPEAT`` blocks are flattened before annotation.
    """
    stim, _sinter, _np = _require_stim_stack()
    p_ryd = float(error_model["rydberg"])
    p_meas = float(error_model["measurement"])
    p_reset = float(error_model["reset"])
    p_move = float(error_model["movement"])
    p_xfer = float(error_model["transfer"])
    p_idle = _idle_probability(float(error_model["idle_per_us"]), tick_us=tick_us)
    p_tick = _compose_depolarize1(p_move, p_idle)

    # Fail closed before DEM construction if any channel is Stim-illegal.
    _validate_stim_channel_probability(
        p_ryd, channel="DEPOLARIZE2(rydberg)", maximum=STIM_DEPOLARIZE2_MAX
    )
    _validate_stim_channel_probability(
        p_xfer, channel="DEPOLARIZE1(transfer)", maximum=STIM_DEPOLARIZE1_MAX
    )
    _validate_stim_channel_probability(
        p_tick, channel="DEPOLARIZE1(movement+idle)", maximum=STIM_DEPOLARIZE1_MAX
    )
    _validate_stim_channel_probability(
        p_meas, channel="measurement", maximum=STIM_PAULI_ERROR_MAX
    )
    _validate_stim_channel_probability(
        p_reset, channel="reset", maximum=STIM_PAULI_ERROR_MAX
    )

    out = stim.Circuit()
    flat = circuit.flattened() if hasattr(circuit, "flattened") else circuit
    all_qubits = list(range(circuit.num_qubits))

    for inst in flat:
        name = getattr(inst, "name", None)
        if name is None:
            # Should not appear after flattened(); recurse defensively.
            body = inst.body_copy() if hasattr(inst, "body_copy") else None
            if body is not None:
                annotated_body = annotate_noise(body, error_model, tick_us=tick_us)
                repeat_count = int(getattr(inst, "repeat_count", 1))
                for _ in range(repeat_count):
                    out += annotated_body
                continue
            raise HarnessError(f"unsupported Stim instruction type: {type(inst)!r}")

        targets = [t.value for t in inst.targets_copy() if t.is_qubit_target]

        if name in ("M", "MR", "MZ"):
            if p_meas > 0.0 and targets:
                out.append("X_ERROR", targets, [p_meas])
            out.append(inst)
            if name == "MR" and p_reset > 0.0 and targets:
                out.append("X_ERROR", targets, [p_reset])
            continue

        if name == "MX":
            # X-basis measurement: Z errors flip the outcome.
            if p_meas > 0.0 and targets:
                out.append("Z_ERROR", targets, [p_meas])
            out.append(inst)
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
            # One composed DEPOLARIZE1 for movement + idle (not two stacked channels).
            if all_qubits and p_tick > 0.0:
                out.append("DEPOLARIZE1", all_qubits, [p_tick])
            continue

        out.append(inst)

    return out


def scale_error_model(error_model: dict[str, float], scale: float) -> dict[str, float]:
    """Multiply every physical error parameter by ``scale``.

    Clamps each key to its Stim-legal channel maximum (DEPOLARIZE1 ≤ 3/4,
    DEPOLARIZE2 ≤ 15/16, Pauli ≤ 1). Never clamps depolarizing rates to 1.0.
    """
    if not math.isfinite(scale) or scale < 0.0:
        raise HarnessError(f"error scale must be a finite non-negative number (got {scale})")
    out: dict[str, float] = {}
    for key in ERROR_MODEL_KEYS:
        raw = float(error_model[key]) * scale
        maximum = ERROR_MODEL_STIM_MAX[key]
        out[key] = min(maximum, max(0.0, raw))
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

    try:
        dem = noisy_circuit.detector_error_model(decompose_errors=True)
        sampler = noisy_circuit.compile_detector_sampler(seed=seed)
        dets, obs = sampler.sample(shots=shots, separate_observables=True)
        predictions = sinter.predict_observables(dem=dem, dets=dets, decoder=decoder)
    except ValueError as exc:
        raise HarnessError(f"Stim/Sinter sampling failed: {exc}") from exc
    except Exception as exc:
        # Keep DEM / decoder failures actionable rather than raw tracebacks.
        raise HarnessError(f"Stim/Sinter sampling failed: {exc}") from exc

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
    if tick_us <= 0.0 or not math.isfinite(tick_us):
        raise HarnessError(f"--tick-us must be a finite positive number (got {tick_us})")
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
            "Output CSV is a sampled artifact only — separate from the compiler "
            "analytic ResourceReport (--emit-resource-report); not a threshold "
            "claim (ADR-0020)."
        ),
        epilog="""
Analytic vs sampled (ADR-0020)
  quonc --emit-resource-report writes analytic schedule / error-budget metrics.
  This harness writes a sampled Sinter CSV (logical_failures, …). Keep them as
  separate files — there is no merged summary generator. Analytic ≠ sampled;
  neither artifact is a threshold claim.

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
  Scaled rates are clamped to Stim-legal maxima (DEPOLARIZE1 ≤ 3/4,
  DEPOLARIZE2 ≤ 15/16), never to 1.0 for depolarizing channels.

--tick-us proxy
  Structure-level `.stim` has no NA schedule microseconds. --tick-us is only
  a coarse proxy converting idle_per_us → per-TICK DEPOLARIZE1; it is not
  wall-clock schedule time from the planner. Movement and idle are composed
  into a single DEPOLARIZE1 after each TICK.

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
            f"proxy µs per Stim TICK when mapping idle_per_us "
            f"(default: {DEFAULT_TICK_US}; not NA schedule wall-clock)"
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
    except ValueError as exc:
        # Stim DEM / probability errors that escaped HarnessError wrapping.
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
