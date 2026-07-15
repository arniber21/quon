# Quon developer bootstrap — source of truth for local gates and CI (ADR-0012).
# Prefer: `devbox run -- just <recipe>` or `just <recipe>` inside `devbox shell`.

set shell := ["bash", "-euo", "pipefail", "-c"]

export WORKSPACE_EXCLUDE := "--exclude flux_verify"
CI_CORPUS := "test/tooling/ci-corpus.txt"
QUONLINT_CONFIG := ".quonlint.toml"

# Default: list public recipes.
default:
    @just --list

# ---------------------------------------------------------------------------
# Doctor — readiness matrix (required rows fail; optional WARN; --strict fails all)
# ---------------------------------------------------------------------------

# Print toolchain readiness. Pass `--strict` to fail on optional WARN rows too.
doctor *args:
    #!/usr/bin/env bash
    set -euo pipefail
    strict=0
    for a in {{args}}; do
      case "$a" in
        --) ;;
        --strict) strict=1 ;;
        -h|--help)
          echo "Usage: just doctor [--strict]"
          exit 0
          ;;
        *)
          echo "error: unknown doctor argument: $a" >&2
          exit 2
          ;;
      esac
    done

    required_fail=0
    optional_fail=0

    row() {
      local tier="$1" name="$2" status="$3" detail="${4:-}"
      printf '%-10s %-28s %-8s %s\n' "[$tier]" "$name" "$status" "$detail"
      if [[ "$status" != "OK" ]]; then
        if [[ "$tier" == "required" ]]; then
          required_fail=1
        else
          optional_fail=1
        fi
      fi
    }

    echo "Quon doctor"
    echo "==========="

    if [[ -n "${MLIR_SYS_220_PREFIX:-}" && -d "${MLIR_SYS_220_PREFIX}" ]]; then
      ver=""
      if command -v llvm-config >/dev/null 2>&1; then
        ver="$(llvm-config --version 2>/dev/null || true)"
      fi
      row "required" "MLIR_SYS_220_PREFIX" "OK" "${MLIR_SYS_220_PREFIX}${ver:+ (llvm $ver)}"
    else
      row "required" "MLIR_SYS_220_PREFIX" "MISSING" "set via devbox shell or export"
    fi

    if command -v FileCheck >/dev/null 2>&1; then
      row "optional" "FileCheck" "OK" "$(command -v FileCheck)"
    else
      row "optional" "FileCheck" "MISSING" "needs LLVM bin on PATH"
    fi

    if command -v z3 >/dev/null 2>&1; then
      row "required" "z3" "OK" "$(z3 --version 2>/dev/null | head -1 || command -v z3)"
    else
      row "required" "z3" "MISSING" "devbox package or brew/apt libz3"
    fi

    if [[ -x .venv/bin/python ]]; then
      if .venv/bin/python -c "import qiskit_aer" >/dev/null 2>&1; then
        row "optional" "Python+Qiskit (.venv)" "OK" ".venv"
      else
        row "optional" "Python+Qiskit (.venv)" "MISSING" "venv exists but qiskit_aer missing — just setup-python"
      fi
    else
      row "optional" "Python+Qiskit (.venv)" "MISSING" "just setup-python"
    fi

    if command -v lit >/dev/null 2>&1; then
      row "optional" "lit" "OK" "$(command -v lit)"
    elif [[ -x .venv/bin/lit ]]; then
      row "optional" "lit" "OK" ".venv/bin/lit"
    else
      row "optional" "lit" "MISSING" "just setup-python (installs lit)"
    fi

    if [[ -x target/release/quonc ]]; then
      row "optional" "QUONC / quonc" "OK" "target/release/quonc"
    else
      row "optional" "QUONC / quonc" "MISSING" "cargo build --release -p quonc"
    fi

    echo
    if [[ "$required_fail" -ne 0 ]]; then
      echo "doctor: REQUIRED checks failed" >&2
      exit 1
    fi
    if [[ "$optional_fail" -ne 0 ]]; then
      if [[ "$strict" -eq 1 ]]; then
        echo "doctor: optional checks failed (--strict)" >&2
        exit 1
      fi
      echo "doctor: optional WARN rows present (ok; use --strict to fail)"
    else
      echo "doctor: all checks OK"
    fi

# Create .venv and install python/requirements.txt + lit (CI + local Aer/lit).
setup-python:
    python3 -m venv .venv
    .venv/bin/python -m pip install --upgrade pip
    .venv/bin/python -m pip install -r python/requirements.txt lit
    @echo "Activate with: source .venv/bin/activate"

# ---------------------------------------------------------------------------
# Test surfaces
# ---------------------------------------------------------------------------

# Unit + integration tests (soft-skip lit if tools missing). No Aer.
test-fast:
    cargo test --workspace {{WORKSPACE_EXCLUDE}}

# Local CI parity: rust + tooling + validation-doc assert (not website build).
test-ci: ci-rust ci-tooling ci-docs-assert

# ---------------------------------------------------------------------------
# CI job recipes (Actions calls these via `devbox run -- just …`)
# ---------------------------------------------------------------------------

# fmt · clippy · build · examples · tests (QUON_REQUIRE_LIT) · Aer verify list
ci-rust: setup-python
    #!/usr/bin/env bash
    cargo fmt --all -- --check
    cargo clippy --workspace {{WORKSPACE_EXCLUDE}} --all-targets -- -D warnings
    cargo build --release --workspace {{WORKSPACE_EXCLUDE}}
    cargo build --examples --workspace {{WORKSPACE_EXCLUDE}}
    export PATH="$PWD/.venv/bin:$PATH"
    export QUON_REQUIRE_LIT=1
    cargo test --workspace {{WORKSPACE_EXCLUDE}}
    export QUONC=target/release/quonc
    for script in \
      test/verify/bell.py \
      test/verify/teleport.py \
      test/verify/bernstein_vazirani.py \
      test/verify/routing.py \
      test/verify/grover.py \
      test/verify/qft.py \
      test/verify/ising.py \
      test/verify/qaoa.py \
      test/verify/shor.py
    do
      echo "==> $script"
      .venv/bin/python "$script"
    done
    echo "==> python/test_qec_stim_smoke.py (ADR-0022 / #253)"
    .venv/bin/python -m unittest python/test_qec_stim_smoke.py
    echo "==> python/test_quon_qec_sinter.py (#253)"
    .venv/bin/python -m unittest python/test_quon_qec_sinter.py

# quonfmt · quonlint · LSP smoke on CI corpus
ci-tooling: _tooling-build
    #!/usr/bin/env bash
    files=()
    while IFS= read -r f; do
      files+=("$f")
    done < <(just _corpus-ci)
    if [[ ${#files[@]} -eq 0 ]]; then
      echo "error: no corpus files selected" >&2
      exit 2
    fi
    echo "==> quonfmt --check (${#files[@]} files)"
    ./target/release/quonfmt --check "${files[@]}"
    echo "==> quonlint --fail-on error"
    ./target/release/quonlint --config {{QUONLINT_CONFIG}} --fail-on error "${files[@]}"
    echo "==> quon_lsp smoke (--include-ignored)"
    cargo test --release -p quon_lsp --test smoke -- --include-ignored

# Broader local tooling sweep over fixture .qn files (not part of test-ci).
tooling-full: _tooling-build
    #!/usr/bin/env bash
    files=()
    while IFS= read -r f; do
      files+=("$f")
    done < <(just _corpus-full)
    if [[ ${#files[@]} -eq 0 ]]; then
      echo "error: no corpus files selected" >&2
      exit 2
    fi
    echo "==> quonfmt --check (${#files[@]} files, full)"
    ./target/release/quonfmt --check "${files[@]}"
    echo "==> quonlint --fail-on error (full)"
    ./target/release/quonlint --config {{QUONLINT_CONFIG}} --fail-on error "${files[@]}"
    echo "==> quon_lsp smoke (--include-ignored)"
    cargo test --release -p quon_lsp --test smoke -- --include-ignored

# Assert agent validation docs match Justfile / CI reality (#203).
ci-docs-assert:
    ./scripts/assert-validation-docs.sh

# Starlight site build under website/.
ci-website:
    #!/usr/bin/env bash
    cd website
    pnpm install --frozen-lockfile
    pnpm build

# ---------------------------------------------------------------------------
# Private helpers
# ---------------------------------------------------------------------------

[private]
_tooling-build:
    #!/usr/bin/env bash
    if [[ -x target/release/quonfmt && -x target/release/quonlint ]] \
      && ./target/release/quonfmt --version >/dev/null 2>&1 \
      && ./target/release/quonlint --version >/dev/null 2>&1; then
      echo "tooling: reusing existing release binaries"
    else
      cargo build --release -p quonfmt -p quonlint-cli -p quon_lsp
    fi

[private]
_corpus-ci:
    #!/usr/bin/env bash
    while IFS= read -r path || [[ -n "$path" ]]; do
      path="${path%%#*}"
      path="$(echo "$path" | sed 's/^[[:space:]]*//;s/[[:space:]]*$//')"
      [[ -z "$path" ]] && continue
      if [[ ! -f "$path" ]]; then
        echo "error: corpus path missing: $path" >&2
        exit 2
      fi
      echo "$path"
    done < {{CI_CORPUS}}

[private]
_corpus-full:
    #!/usr/bin/env bash
    find frontend/tests/fixtures -name '*.qn' -type f | sort
    if [[ -d test/verify ]]; then
      find test/verify -maxdepth 1 -name '*.qn' -type f | sort
    fi
