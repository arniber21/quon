# Benchmark: quonc vs quilc

Reproducible comparison of Quon's `quonc` against Rigetti's [quilc](https://github.com/quil-lang/quilc) on a shared circuit corpus (GitHub issue #116).

## What this measures

Per circuit, for both compilers targeting a **matched 5-qubit linear topology** with **CNOT/CX** as the native two-qubit gate:

| Metric | quonc | quilc |
|--------|-------|-------|
| 2Q-gate count | Parsed from emitted OpenQASM (`cx`/`cz`/…) | Parsed from compiled Quil (`CNOT`/`CZ`/…) |
| Depth | `CircuitMetrics.depth` (schedule layers) | `--print-statistics` "Compiled gate depth" |
| T-count | IR `t_count` (pre-final native decomp) | Input Quil T count; **post-compile T→RZ** under quilc's Xhalves 1Q ISA |

**Caveat (1Q natives):** quonc uses `{rz, sx, x}`; quilc's default 1Q layer is Xhalves (`RZ` + discrete `RX`). Depth numbers are therefore not perfectly apples-to-apples, but 2Q count and routing pressure on the same linear topology are.

## Setup: quilc via Docker (no hardware, no SBCL brew)

Docker is the supported path (SBCL/Quicklisp local builds are heavy and not required).

```bash
docker pull rigetti/quilc:latest
echo 'H 0' | docker run --rm -i rigetti/quilc -P --quiet
```

Image: [`rigetti/quilc`](https://hub.docker.com/r/rigetti/quilc) (public). Optional: set `DOCKER=/usr/local/bin/docker` if `docker` is not on `PATH`.

### Matched ISA / target

| Compiler | File |
|----------|------|
| quonc | [`target/linear5_cx.json`](target/linear5_cx.json) |
| quilc | [`target/linear5_cnot.qpu`](target/linear5_cnot.qpu) |

Both describe qubits `0–4` with edges `0-1, 1-2, 2-3, 3-4` and native CNOT/CX on each edge.

## Setup: quonc

```bash
export PATH="/opt/homebrew/opt/llvm@22/bin:$PATH"   # macOS Homebrew LLVM 22
export MLIR_SYS_220_PREFIX="/opt/homebrew/opt/llvm@22"
cargo build -p quonc --release
```

## Run the benchmark

```bash
./bench/quilc/scripts/run_bench.sh
# quonc-only (if Docker unavailable):
./bench/quilc/scripts/run_bench.sh --skip-quilc
```

Outputs:

- [`RESULTS.md`](RESULTS.md) / [`RESULTS.json`](RESULTS.json) — checked-in comparison table + findings (copied from `out/` after each run)
- `out/qasm/`, `out/quonc/`, `out/quilc/` — regenerable per-circuit artifacts (gitignored via repo `out/`)

## Corpus (≥12 circuits)

See [`corpus/manifest.json`](corpus/manifest.json). Sources live under `corpus/qn/` (Quon) and `corpus/quil/` (Quil). Small/medium mix: Bell, BV, GHZ, ladders, QFT-3, Ising/QAOA steps, Clifford+T, rotation network, routing stress, Deutsch–Jozsa.

## Literature / methodology

quilc is Rigetti's optimizing Quil compiler (Smith et al., Forest/quilc; DOI [10.5281/zenodo.3677536](https://doi.org/10.5281/zenodo.3677536)). There is no Quon paper required for this bench. Comparison methodology:

1. Same logical circuits expressed as Quon (`.qn`) and Quil (`.quil`).
2. Same connectivity (linear 5) and same native 2Q (CNOT).
3. Report 2Q count, depth, and T-count with the definition differences above documented.
4. Flag circuits where quonc is within 20% of quilc on 2Q and depth; list laggards to prioritize optimizer work (#96/#97).

## Residual risks

- **1Q ISA mismatch** (sx/x vs RX halves) inflates depth deltas.
- **T-count after quilc** is usually 0 (folded into RZ); use input Quil / quonc IR for Clifford+T.
- **CZ-heavy programs** (e.g. stock `test/verify/grover.qn`) can segfault quonc on linear targets today; corpus uses a CNOT+Z Grover variant.
- **Docker image age:** `rigetti/quilc:latest` last pushed ~2023; pin digest in CI if bit-stability matters.
