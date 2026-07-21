# Validation

Static analysis and refinement-type checks for the Quon workspace.

**Native toolchain:** prefer `devbox shell` for a locked LLVM/MLIR 22 + Z3 environment (see root `devbox.json`). Orchestration lives in the root **Justfile** (ADR-0012): `devbox run -- just doctor`, `just test-fast`, `just test-ci`. Rust stays on rustup. Manual brew/apt LLVM remains an advanced fallback — see [README](../../README.md) and the [install guide](../../website/src/content/docs/getting-started/install.md).

## Developer bootstrap

| Command | Purpose |
| ------- | ------- |
| `just doctor` | Readiness matrix (LLVM/`MLIR_SYS_220_PREFIX`, z3, `.venv`+Qiskit, lit, FileCheck, `quonc`). Required rows fail; optional WARN. `just doctor --strict` fails on WARN too. |
| `just setup-python` | Create `.venv`, install `python/requirements.txt` + `lit` |
| `.venv/bin/python -m pytest python -q` | Local one-shot run of the Python harness tests (same unittest suites `just ci-rust` runs one-by-one via `python -m unittest`; `pytest` comes from `python/requirements.txt`). The two quonc-integration tests want a fresh `target/release/quonc` (or `QUONC=target/debug/quonc`) |
| `just test-fast` | `cargo test --workspace --exclude flux_verify` (lit soft-skips if tools missing; no Aer) |
| `just test-ci` | Local CI parity: `ci-rust` + `ci-tooling` + `ci-docs-assert` (not the website build) |
| `just ci-rust` | What the `ci.yml` `rust` job runs (sets `QUON_REQUIRE_LIT`); its `cargo test --workspace` step already includes the sample corpus catalog lint (`quonc/tests/samples_catalog.rs`, ADR-0025 / #185) as an ordinary workspace test crate |
| `just ci-samples` | Local-only convenience: re-run just the sample corpus catalog lint (schema, paths, category coverage, README sections, `ci: smoke` typecheck) in isolation. **Not** part of `test-ci` or `ci.yml` — it would double-pay for what `ci-rust` already covers |
| `just ci-tooling` | What the `ci.yml` `tooling` job runs |
| `just tooling-full` | Broader local fmt/lint corpus (not CI) |
| `just qec-benchmarks-smoke` | #254 local convenience: one-cell ablation + nested Sinter (`--mode smoke`). CI smoke is the unittest in `just ci-rust`. |
| `just qec-benchmarks-axis` | #254 local convenience: axis-coverage grid (`--mode axis`; gates full). Also covered by CI unittest when quonc is built. |
| `just qec-benchmarks-full` | #254 local-only full ablation grid (`--mode full`; proven by axis coverage) |
| `just rap-table-i` | #111 local-only convenience: `cargo test --release -p quonc --test rap_table_i -- --include-ignored --nocapture` (the metrics-dump test; the fast structural preflight already runs in `just ci-rust`). **Not invoked by any CI job** — see the `rust` row below. |
| `just na-rap-sweep` | #306 local-only: both `--na-placer` modes over every checked-in RAP Table I row (`ising_n42`, `ising_n98`), one qmap-comparable CSV (`python/na_rap_table_i_sweep.py`). Not CI — see `docs/neutral_atom/rap_table_i_methodology.md`'s "Full sweep harness (#306)". |
| `just ci-docs-assert` | `./scripts/assert-validation-docs.sh` |
| `just ci-website` | Starlight `pnpm build` under `website/` |

Inside Devbox: `devbox run -- just <recipe>` (or `just` after `devbox shell`).

## CI matrix

This table is an adapter of the **Justfile** recipes invoked by `.github/workflows/` — keep it in sync, and run `just ci-docs-assert` (also on the `ci.yml` docs job) so known-stale claims cannot return.

| Workflow | Trigger | What runs |
| -------- | ------- | --------- |
| [ci.yml](../../.github/workflows/ci.yml) `rust` | every push and PR | `just ci-rust`: fmt, clippy, release build (+ examples for lit oracles), `cargo test --workspace --exclude flux_verify` with `QUON_REQUIRE_LIT` so [`quonc/tests/lit.rs`](../../quonc/tests/lit.rs) hard-fails without lit/FileCheck/oracles, and [`quonc/tests/samples_catalog.rs`](../../quonc/tests/samples_catalog.rs) lints `samples/catalog.yaml` and typechecks every `ci: smoke` entry with the debug `quonc` this same `cargo test` builds (ADR-0025 / #185) — the RAP Table I preflight test (#111) runs here too, while the full `rap_table_i --include-ignored` metrics dump is **local-only** (`just rap-table-i`, not invoked by this or any other CI job): pre-#297 its routing-aware A* peaked ~17.5 GB RSS and OOM'd GitHub's 16 GB hosted runners; #297's heuristic search dropped that to ~64 MB, but the recipe has not been re-wired into CI since (documented follow-up, not done here or in #306) — see `docs/neutral_atom/rap_table_i_methodology.md`'s "Runtime / CI wiring" correction; then Qiskit Aer: `test/verify/{bell,teleport,bernstein_vazirani,routing,grover,qft,ising,qaoa,shor}.py` with `QUONC=target/release/quonc`, then QEC Python smokes (`test_qec_stim_smoke`, `test_quon_qec_sinter`, `test_quon_qec_benchmarks` / #254). |
| [ci.yml](../../.github/workflows/ci.yml) `docs` | every push and PR | `just ci-docs-assert` + `just ci-website` |
| [ci.yml](../../.github/workflows/ci.yml) `tooling` | every push and PR | `just ci-tooling`: `quonfmt --check`, `quonlint`, `quon_lsp` smoke on CI corpus |
| [release.yml](../../.github/workflows/release.yml) | tags `v*` (+ manual dry-run) | `devbox run release` — static MLIR/LLVM + release-built static libz3; link audit; upload `quon-{version}-{arch}-{os}.tar.gz` to GitHub Releases |
| [taskless.yml](../../.github/workflows/taskless.yml) | every PR (diff-scoped); push to `main` (full) | `@taskless/cli check` (Node 22+) |
| [flux.yml](../../.github/workflows/flux.yml) | PR when `flux_verify/`, `quon_core/`, `backend/`, or lockfile changes; push to `main` (unconditional) | `cargo flux -p flux_verify`, `cargo flux -p quon_core --features flux`, `cargo flux -p backend --features flux` (nightly + z3) |
| [coverage.yml](../../.github/workflows/coverage.yml) | every PR (non-blocking) | `cargo llvm-cov` summary via `./scripts/coverage.sh` (stable, excludes `flux_verify`; needs LLVM 22 + MLIR) |
| [vscode-extension.yml](../../.github/workflows/vscode-extension.yml) | path-filtered push/PR | tree-sitter corpus + VS Code extension package checks |

Local `lit test/lit/ -v` remains useful for verbose FileCheck output. CI and `just test-ci` require the suite via `QUON_REQUIRE_LIT`; bare `cargo test` / `just test-fast` still soft-skip when tools are missing.

**ADR/docs drift:** borrow cleanup semantics vs ADR-0003 were tracked in [#180](https://github.com/arniber21/quon/issues/180) (now closed) — check that issue's resolution before touching borrow docs from this validation matrix.

## Tooling gates (quonfmt · quonlint · LSP)

The `tooling` job in [ci.yml](../../.github/workflows/ci.yml) runs `just ci-tooling` on a fixed 16-file corpus (`test/tooling/ci-corpus.txt`):

- **`quonfmt --check`** — no formatting drift
- **`quonlint`** — diagnostics at or above `error` fail the job (config: `.quonlint.toml`)
- **LSP smoke** — protocol-level stdio JSON-RPC checks in `quon_lsp/tests/smoke.rs` (marked `#[ignore]`; tooling job runs with `--include-ignored`)

```bash
# Match CI exactly
just ci-tooling

# Broader local sweep (not CI)
just tooling-full
```

LSP smoke tests are intentionally skipped by `cargo test --workspace` (they use `#[ignore]`). Only the tooling job / `just ci-tooling` runs them via `--include-ignored`.

## Taskless (ast-grep rules)

[Taskless](https://github.com/taskless/skills) ships project-specific validation rules under `.taskless/rules/`. Rules are checked with ast-grep via the `@taskless/cli` package — no API auth required for `check`.

### Prerequisites

- Node.js 22+ (for `npx`; matches CI)

### Run locally

```bash
# Full workspace scan
npx @taskless/cli@latest check

# Changed files only (e.g. before opening a PR)
npx @taskless/cli@latest check $(git diff --name-only main...HEAD)

# JSON output (for agents / scripts)
npx @taskless/cli@latest check --json
```

### Add or change rules

1. Read the canonical skill at `.taskless/skills/taskless/SKILL.md` (or invoke `/tskl` in Cursor).
2. Author rules in `.taskless/rules/*.yml` with matching tests in `.taskless/rule-tests/`.
3. Verify a rule before committing:

   ```bash
   npx @taskless/cli@latest rule verify <rule-id> --json
   ```

4. Re-run `check` against the codebase.

### CI

`.github/workflows/taskless.yml` runs a diff-scoped check on pull requests and a full scan on pushes to `main`. No secrets required.

### Agent skill

Cursor and Claude Code hold thin stubs (`.cursor/skills/taskless/`, `.claude/skills/taskless/`) that delegate to `.taskless/skills/taskless/SKILL.md`. Re-install after upgrading the CLI:

```bash
npx @taskless/cli@latest init --no-interactive
```

### Rules in this repo

| Rule ID | What it catches |
| ------- | --------------- |
| `no-unwrap-expect-in-src` | `unwrap()` / `expect()` in library `src/` (tests and `quonc` excluded) |
| `no-anyhow-in-lib-src` | `anyhow` imports or types in library crates — use `thiserror` instead |
| `serde-deny-unknown-fields-on-dto` | `Deserialize` structs missing `#[serde(deny_unknown_fields)]` |
| `quantum-circ-builders-must-verify` | `OperationBuilder` helpers that return without calling `verify()` |

Authoring conventions and error-handling rationale: [code-quality.md](./code-quality.md).

When adding a rule, always add matching tests under `.taskless/rule-tests/` and run `npx @taskless/cli@latest rule verify <id> --json` before committing.

## Flux (refinement types)

See [README.md](../../README.md#flux-refinement-types) for installing `cargo-flux` and running refinement checks on the `flux_verify` crate.

### When to run Flux

Run `cargo flux -p flux_verify` locally (or rely on `.github/workflows/flux.yml`) when you:

- Change `flux_verify/src/**` or add new `#[spec(...)]` annotations
- Touch workspace Flux dependencies or the `flux` feature on a crate
- Introduce Rust code where a refinement spec is the primary correctness argument

Flux uses **nightly** and **z3**; it is intentionally excluded from the stable `cargo test` / clippy CI job. The stable workspace must still build without Flux installed.

### Testing and static analysis expectations

Agents should treat validation as a **stack of fast feedback loops**, not a single CI gate at the end:

1. **Every PR** — prefer `just test-ci` (or at least fmt/clippy/`just test-fast`, Taskless on changed files, `just ci-tooling`).
2. **Language / parser / typechecker work** — add or extend unit tests; keep reference-algorithm fixtures passing (`frontend/tests/reference_algorithms.rs`).
3. **Algorithms and serializers** — add proptest coverage where an oracle or invariant exists (`backend/tests/props.rs`, `quon_core/tests/depth_props.rs`); consider `cargo fuzz` for byte/text parsers.
4. **MLIR dialect changes** — unit/integration tests in `mlir_bridge/tests/`; builders must call `verify()` (enforced by Taskless).
5. **Refinement-heavy Rust** — Flux specs in `flux_verify` when `{v: …}` contracts clarify intent beyond tests.

Slow checks (full Aer list, full fuzz campaigns) are valuable before phase milestones but should not replace the fast layers above for day-to-day development. Use `just test-fast` day-to-day; `just test-ci` before opening a PR.

## LLVM source coverage (`cargo-llvm-cov`)

Optional line coverage for the stable workspace (excludes `flux_verify`). Requires LLVM/MLIR on PATH like `cargo test`.

### Prerequisites

```bash
rustup component add llvm-tools-preview
cargo install cargo-llvm-cov
```

### Run locally

```bash
./scripts/coverage.sh              # summary table
./scripts/coverage.sh --html       # HTML report → target/llvm-cov/html/
./scripts/coverage.sh --lcov       # lcov.info for upload tools
```

CI: `.github/workflows/coverage.yml` runs on pull requests (non-blocking summary). Same exclusions as the main `ci.yml` test job.
