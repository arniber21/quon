from __future__ import annotations

"""
Quon -> Qiskit Aer verification seam (issue #29; deepened by issue #204).

The module's interface is: Quon source (or already-compiled QASM) + an
expected outcome distribution -> pass/fail. Four adapters sit behind that
seam, in the order data flows through them:

    1. Compile          `compile_to_qasm`      resolve QUONC, invoke quonc
    2. Dialect normalize `load_circuit`         quonc QASM -> Qiskit circuit
    3. Simulate          `run_on_aer`           Aer (shots, seed, noise)
    4. Oracle            `verify_distribution`  compare counts to a reference

`test/verify/*.py` and `python/noisy_fidelity.py` are the callers. Piping
`quonc --emit-qasm` straight into `qiskit.qasm3.loads` (or any other raw
Qiskit entry point), skipping `normalize_bit_int_conditions`, is
unsupported: quonc emits spec-valid `bit[i] == 1` integer conditions that
`qiskit_qasm3_import`'s indexed-bit grammar rejects.

CLI usage (unchanged since issue #29):
    quonc --emit-qasm program.qn | python python/quon_aer.py --shots 4096
    python python/quon_aer.py program.qn --shots 4096
"""

import argparse
import os
import re
import subprocess
import sys

# ---------------------------------------------------------------------------
# Errors — actionable failure modes (issue #204)
# ---------------------------------------------------------------------------


class VerificationError(Exception):
    """Base class for verification-seam failures; every subclass carries an
    actionable message (what's missing, and the exact command to fix it)."""


class QuoncNotFoundError(VerificationError):
    def __init__(self, binary: str) -> None:
        super().__init__(
            f"quonc binary {binary!r} was not found on PATH or via $QUONC.\n"
            "Build it and point at the binary:\n"
            "  cargo build --release -p quonc\n"
            "  export QUONC=$PWD/target/release/quonc"
        )


class QuoncCompileError(VerificationError):
    def __init__(self, source: str, args: list[str], stderr: str) -> None:
        super().__init__(
            f"quonc failed to compile {source!r} (args: {args}):\n{stderr}"
        )


class SimulationDependencyMissingError(VerificationError):
    """A required optional Python dependency is not installed.

    Names the exact pip package, since the issue this class fixes (#204)
    calls out a specific real-world confusion: the pip package name uses
    hyphens (`qiskit-qasm3-import`), while the Python import name it
    provides uses underscores (`qiskit_qasm3_import`).
    """

    def __init__(self, package: str, cause: Exception | None = None) -> None:
        super().__init__(
            f"{package} is not installed (required for the Qiskit Aer "
            "verification seam, issue #204/#29).\n"
            "Install every verification dependency with:\n"
            "  pip install -r python/requirements.txt\n"
            "(note: the pip package name uses hyphens, e.g. "
            "`qiskit-qasm3-import`; the `import qiskit_qasm3_import` "
            "statement it provides uses underscores — both refer to the "
            "same package.)"
        )
        if cause is not None:
            self.__cause__ = cause


# ---------------------------------------------------------------------------
# Adapter 1: compile — resolve QUONC, invoke with consistent flags
# ---------------------------------------------------------------------------


def quonc_binary() -> str:
    """The quonc executable: the QUONC env var if set, else `quonc` on PATH."""
    return os.environ.get("QUONC", "quonc")


def compile_to_qasm(
    source_file: str,
    target: str | None = None,
    extra_args: list[str] | None = None,
) -> str:
    """Compile a .qn source file to OpenQASM 3 via `quonc --emit-qasm`.

    `extra_args` is appended after `--target` and before the source path,
    e.g. `["--sabre-gamma", "0.5"]` — this is the one place that invokes
    quonc, so callers (like `noisy_fidelity.py`) that need extra flags no
    longer hand-roll a second `subprocess.run`.
    """
    binary = quonc_binary()
    args = [binary, "--emit-qasm"]
    if target is not None:
        args += ["--target", target]
    if extra_args:
        args += extra_args
    args.append(source_file)
    try:
        result = subprocess.run(args, capture_output=True, text=True, check=True)
    except FileNotFoundError as exc:
        raise QuoncNotFoundError(binary) from exc
    except subprocess.CalledProcessError as exc:
        raise QuoncCompileError(source_file, args, exc.stderr) from exc
    return result.stdout


# ---------------------------------------------------------------------------
# Adapter 2: dialect normalize — explicit Qiskit importer adapter
# ---------------------------------------------------------------------------

_BIT_INT_CONDITION = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*\[\d+\])\s*==\s*([01])\b")


def normalize_bit_int_conditions(qasm_src: str) -> str:
    """Normalize spec-valid bit integer conditions for Qiskit's importer.

    quonc emits `if (c[i] == 1)` to match the OpenQASM integer-comparison
    convention used by the backend. qiskit_qasm3_import currently accepts
    indexed-bit conditions only as `bit == const bool`, while accepting
    integer comparisons for whole bit arrays. Keep quonc's output unchanged
    and adapt only this verification bridge.

    This is the named, public, unit-tested Qiskit-importer dialect adapter
    (issue #204) — it was previously an unnamed private regex duplicated by
    reaching across module boundaries (`quon_aer._qiskit_qasm3_compat`).
    """

    def replace(match: re.Match[str]) -> str:
        bit_ref, value = match.groups()
        return f"{bit_ref} == {'true' if value == '1' else 'false'}"

    return _BIT_INT_CONDITION.sub(replace, qasm_src)


def load_circuit(qasm_src: str):
    """The only supported bridge from quonc's OpenQASM 3 to a Qiskit circuit.

    Applies `normalize_bit_int_conditions` before handing the source to
    `qiskit.qasm3.loads`; raw `qasm3.loads(qasm_src)` without that step is
    unsupported (see module docstring). Falls back to `measure_all()` only
    for a purely unitary circuit with no classical bits, since quonc emits
    explicit `measure` statements into a `bit[m] c;` register whenever the
    source measures anything.
    """
    try:
        from qiskit import qasm3
    except ImportError as exc:
        raise SimulationDependencyMissingError("qiskit", exc) from exc

    try:
        circuit = qasm3.loads(normalize_bit_int_conditions(qasm_src))
    except ImportError as exc:
        if "qasm3_import" in str(exc):
            raise SimulationDependencyMissingError(
                "qiskit-qasm3-import", exc
            ) from exc
        raise

    if circuit.num_clbits == 0:
        circuit.measure_all()
    return circuit


# ---------------------------------------------------------------------------
# Adapter 3: simulate — Aer (shots, seed, optional noise model)
# ---------------------------------------------------------------------------


def run_on_aer(
    qasm_src: str,
    shots: int = 4096,
    seed: int | None = None,
    noise_model=None,
) -> dict[str, int]:
    """Run quonc-emitted OpenQASM 3 on `AerSimulator` and return shot counts.

    `noise_model` is an optional `qiskit_aer.noise.NoiseModel`; omitted (the
    default), the simulation is ideal. This absorbs `noisy_fidelity.py`'s
    previously-duplicated `AerSimulator` construction.
    """
    try:
        from qiskit_aer import AerSimulator
    except ImportError as exc:
        raise SimulationDependencyMissingError("qiskit-aer", exc) from exc

    circuit = load_circuit(qasm_src)
    sim = AerSimulator(noise_model=noise_model)
    # `seed` pins the sampler RNG so verification runs are reproducible; left
    # as None for the interactive CLI, where fresh randomness is fine.
    run_kwargs: dict = {"shots": shots}
    if seed is not None:
        run_kwargs["seed_simulator"] = seed
    job = sim.run(circuit, **run_kwargs)
    return dict(job.result().get_counts())


# ---------------------------------------------------------------------------
# Adapter 4: oracle — shared comparison helpers + the deep pass/fail seam
# ---------------------------------------------------------------------------


def normalize_key(bitstring: str) -> str:
    """Strip Qiskit's space separators between multi-register classical bits."""
    return bitstring.replace(" ", "")


def clbit(key: str, index: int, nbits: int) -> int:
    """Value of classical bit c[index]; Qiskit prints bits high-index-first."""
    return int(normalize_key(key)[nbits - 1 - index])


def normalize_counts(counts: dict[str, int]) -> dict[str, float]:
    """Shot counts -> probabilities, keyed on Qiskit's space-stripped bitstrings."""
    total = sum(counts.values())
    normalized: dict[str, float] = {}
    for key, value in counts.items():
        nk = normalize_key(key)
        normalized[nk] = normalized.get(nk, 0.0) + (value / total if total else 0.0)
    return normalized


def hellinger_fidelity(p: dict[str, float], q: dict[str, float]) -> float:
    """Hellinger fidelity between two probability distributions over bitstrings.

    For a point-mass `q = {key: 1.0}` this reduces exactly to `p[key]`
    (every other term vanishes because `q` is zero there), which is why
    `verify_distribution` can subsume the "count of one expected outcome /
    shots >= threshold" checks that several verify scripts used directly.
    """
    keys = set(p) | set(q)
    return sum((p.get(k, 0.0) * q.get(k, 0.0)) ** 0.5 for k in keys) ** 2


class VerifyResult:
    """Pass/fail outcome of `verify_distribution`."""

    def __init__(
        self, ok: bool, counts: dict[str, int], fidelity: float, message: str
    ) -> None:
        self.ok = ok
        self.counts = counts
        self.fidelity = fidelity
        self.message = message

    def __bool__(self) -> bool:
        return self.ok

    def __repr__(self) -> str:
        return f"VerifyResult(ok={self.ok}, fidelity={self.fidelity:.4f}, message={self.message!r})"


def verify_distribution(
    source: str,
    expected: dict[str, float],
    *,
    shots: int = 4096,
    seed: int | None = None,
    min_fidelity: float = 0.99,
    target: str | None = None,
    is_qasm: bool = False,
) -> VerifyResult:
    """The module's deep interface: Quon source (or QASM) + expected
    distribution -> pass/fail.

    Compiles `source` (unless `is_qasm=True`, in which case `source` is
    already-compiled OpenQASM), normalizes and simulates it on Aer, and
    passes when the Hellinger fidelity between the observed and `expected`
    distributions is at least `min_fidelity`. `expected` need not be
    normalized to sum to 1 caller-side key-by-key, but should represent a
    valid probability distribution (as every current caller's point-mass
    `{bitstring: 1.0}` does).
    """
    qasm_src = source if is_qasm else compile_to_qasm(source, target=target)
    counts = run_on_aer(qasm_src, shots=shots, seed=seed)
    observed = normalize_counts(counts)
    fidelity = hellinger_fidelity(observed, expected)
    ok = fidelity >= min_fidelity
    verdict = "PASS" if ok else "FAIL"
    message = (
        f"{verdict}: Hellinger fidelity {fidelity:.4f} "
        f"{'>=' if ok else '<'} {min_fidelity} vs expected {expected}"
    )
    return VerifyResult(ok=ok, counts=counts, fidelity=fidelity, message=message)


# ---------------------------------------------------------------------------
# CLI
# ---------------------------------------------------------------------------


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

    counts = run_on_aer(qasm_src, shots=args.shots, seed=args.seed)
    for bitstring, count in sorted(counts.items(), key=lambda x: -x[1]):
        print(f"{bitstring}: {count}")


if __name__ == "__main__":
    main()
