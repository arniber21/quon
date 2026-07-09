# Issue #49 — Tooling quality gates in CI (LSP smoke + formatter + lint)

**Branch**: `issue-49-ci-tooling-gates`  
**Blocked by**: #43 (`quon_lsp`), #46 (`quonfmt`), #47 (`quonlint`) — assume landed before executing this plan.  
**Parent**: #1 (MVP PRD)

## Goal

Extend CI so every push and pull request enforces Quon developer-tooling correctness:

1. **`quonfmt --check`** — no formatting drift in the curated fixture corpus (and, optionally, other tracked `.qn` paths).
2. **`quonlint`** — lint diagnostics at or above a configured severity threshold fail the job.
3. **LSP smoke tests** — fast stdio JSON-RPC checks that the language server starts, handles representative requests, and shuts down cleanly.

Keep wall-clock time bounded with a **small, fixed fixture corpus** and **protocol-level** LSP tests (not editor integration). Document **local reproduction commands** in `docs/agents/validation.md` (and cross-link from `code-quality.md`).

Existing workflows (`ci.yml` rust job, `taskless.yml`, `flux.yml`, `coverage.yml`) must remain unchanged in behavior except for additive documentation.

---

## Current CI landscape (baseline)

| Workflow | Trigger | Runtime profile | Blocking? |
| -------- | ------- | ----------------- | --------- |
| [`ci.yml`](../../.github/workflows/ci.yml) `rust` | push, PR | Heavy (LLVM 22 + MLIR + z3, full workspace build/test, Aer scripts) | Yes |
| [`taskless.yml`](../../.github/workflows/taskless.yml) | PR; push to `main` | Light (Node 22, diff-scoped ast-grep) | Yes |
| [`flux.yml`](../../.github/workflows/flux.yml) | path-filtered PR; push to `main` | Medium (nightly Flux install) | Yes (when triggered) |
| [`coverage.yml`](../../.github/workflows/coverage.yml) | PR | Heavy (LLVM + llvm-cov) | No (`continue-on-error`) |

[`docs/agents/validation.md`](../agents/validation.md) documents the matrix above. Tooling gates are **not** listed yet.

**Relevant fixture assets already in repo:**

- `frontend/tests/fixtures/*.qn` — 8 SPEC §12 reference algorithms + `stdlib_forms.qn`, `error_correction.qn`
- `frontend/tests/fixtures/corpus/*.qn` — 50-program regression corpus (discovered at test time in `frontend/tests/corpus.rs`)
- `frontend/tests/pretty_roundtrip.rs` — AST roundtrip via `frontend::pretty::pretty` on reference fixtures (precursor to `quonfmt` semantics)

---

## Upstream interface contract (hard deps on #43, #46, #47)

#49 is blocked until these interfaces are verified. Run pre-flight before merging:

```bash
cargo tree -p quonfmt -p quonlint -p quon_lsp   # all three in workspace
quonfmt --help | grep -q check
quonlint --help | grep -q fail-on
cargo build --release -p quon_lsp && env | grep CARGO_BIN_EXE_quon_lsp
```

### Required deliverables from upstream issues

| Crate | Binary | Required CLI / API | Exit codes |
|-------|--------|-------------------|------------|
| #46 `quonfmt` | `quonfmt` | `--check PATH…` (check-only, no write) | 0=clean, 1=drift, 2=error |
| #47 `quonlint` | `quonlint` | `--config PATH`, `--fail-on SEVERITY`, `--files-from PATH`, `--format github` | 0=pass, 1=violations, 2=error |
| #43 `quon_lsp` | `quon_lsp` | stdio JSON-RPC server; smoke tests in `tests/smoke.rs` | subprocess exit 0 |

### Config schema (#47 must ship)

- Root `.quonlint.toml` with `[rules."rule/id"]` severity overrides (not category names).
- Rule IDs use slash namespace: `depth/sequential-for-blowup`, `monad/circuit-bind-without-apply`, etc.
- Inline suppressions: `# quonlint-disable rule-id` and `# quonlint-disable-next-line rule-id`.

### Formatter invariant (#46)

`quonfmt` must share the same pretty core as `frontend::pretty::pretty` — corpus manifest files must pass both `quonfmt --check` and `frontend/tests/pretty_roundtrip.rs`.

---

## Design decisions

### 1. New parallel CI job, not steps in `rust`

Add a **`tooling`** job to [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) that runs **in parallel** with the existing `rust` and `docs` jobs.

Rationale:

- **Isolation** — formatter/lint/LSP failures surface with a clear job name; the monolithic `rust` job stays focused on compiler correctness.
- **Bounded runtime** — tooling job skips Aer scripts, lit oracles, and full `cargo test --workspace`.
- **Same triggers** — push + PR, matching acceptance criteria (“each PR”).
- **Pattern match** — mirrors how `taskless.yml` keeps specialized gates separate while remaining required.

Do **not** fold Quon-language lint into `taskless.yml`. Taskless validates **Rust repo conventions** (unwrap, anyhow, serde DTOs). `quonlint` validates **Quon source quality**. They complement each other; document both in the validation stack.

### 2. Scoped build, not full workspace

The tooling job builds only what it needs:

```bash
cargo build --release -p quonfmt -p quonlint -p quon_lsp
cargo test --release -p quon_lsp --test smoke -- --include-ignored
```

**Dependency note:** Today `frontend` depends on `mlir_bridge` / Melior, so the tooling job likely still needs **LLVM 22 + MLIR + libz3** (same bootstrap as `rust`). If #43–#47 introduce an MLIR-free `frontend` surface (e.g. `frontend_syntax`), slim the install to **libz3-dev only** — verify with `cargo tree -p quonfmt` before merging.

### 3. Curated CI corpus, not all 59+ fixtures

Running `quonfmt` / `quonlint` over the full 50-file corpus on every PR is acceptable locally but wasteful in CI. Use a **manifest file** listing a fixed, representative subset (16 files, < 200 KB total).

Full corpus remains available for local `--full` mode and future nightly/scheduled expansion.

### 4. LSP tests live in `quon_lsp`, invoked by CI

Smoke tests are **`cargo test -p quon_lsp --test smoke`**, not a shell script driving `nc`. Keeps protocol helpers in Rust, reuses `insta`/assertion patterns, and benefits from `#[test]` timeouts.

### 5. Workspace test interaction policy

Once `quon_lsp` is a workspace member, `cargo test --workspace` in the `rust` job would run `quon_lsp/tests/smoke.rs` by default — duplicating and slowing the monolithic job.

**Policy (decided):**

| Job | LSP smoke invocation |
|-----|---------------------|
| `rust` | Does **not** run LSP smoke (tests marked `#[ignore]`) |
| `tooling` | `cargo test --release -p quon_lsp --test smoke -- --include-ignored` |

Implementation requirements for #43:
- All LSP smoke tests in `quon_lsp/tests/smoke.rs` use `#[ignore]` with reason `"tooling job only"`.
- Tests spawn server via `env!("CARGO_BIN_EXE_quon_lsp")` (same pattern as `quonc/tests/smoke.rs`).
- Add `[[test]]` dependency on `quon_lsp` binary in `quon_lsp/Cargo.toml`.

### 6. Single local entrypoint script

Add [`scripts/tooling-check.sh`](../../scripts/tooling-check.sh) mirroring CI step order. Developers and agents run one command before opening a PR.

---

## CI job design

### Job: `tooling`

| Property | Value |
| -------- | ----- |
| **Name** | `quonfmt · quonlint · LSP smoke` |
| **Runner** | `ubuntu-latest` |
| **Timeout** | `10` minutes (hard cap; target wall time ≤ 5 min warm-cache) |
| **Triggers** | Same as parent workflow: `push`, `pull_request` |
| **Permissions** | `contents: read` (no secrets) |
| **Required check** | Yes — add to branch protection alongside `rust` and Taskless |

### Step sequence

```
checkout
→ install system deps (LLVM 22 + MLIR + libz3, or z3-only if deps allow)
→ dtolnay/rust-toolchain@stable
→ Swatinem/rust-cache@v2
→ cargo build --release -p quonfmt -p quonlint -p quon_lsp
→ quonfmt --check  (CI corpus manifest)
→ quonlint         (CI corpus manifest, config threshold)
→ cargo test --release -p quon_lsp --test smoke
```

Each step exits non-zero on failure; no `continue-on-error`.

### Failure semantics (acceptance mapping)

| Gate | Fail condition | Acceptance criterion |
| ---- | -------------- | -------------------- |
| `quonfmt --check` | Any listed file would change under formatter | CI fails on formatting drift |
| `quonlint` | Diagnostic count at/above configured severity exceeds threshold | CI fails on lint errors above threshold |
| LSP smoke | Any `#[test]` in `quon_lsp/tests/smoke.rs` fails or times out (via `--include-ignored`) | CI runs LSP smoke tests on each PR |
| Docs update | Manual — update `validation.md` in Phase 5 (not a CI gate) |
| Regression | `rust` / `taskless` / `flux` / `coverage` YAML unchanged except additive docs | Existing stable checks intact |

---

## Workflow YAML changes

### A. Extend `.github/workflows/ci.yml`

Add a new job after the existing `docs` job. Reuse the same `env` block (`CARGO_TERM_COLOR`, `RUST_BACKTRACE`, `MLIR_SYS_220_PREFIX`).

```yaml
  tooling:
    name: quonfmt · quonlint · LSP smoke
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - uses: actions/checkout@v4

      # Omit if quonfmt/quonlint/quon_lsp no longer transitively depend on MLIR.
      - name: Install LLVM 22, MLIR, and libz3
        run: |
          sudo apt-get update
          sudo apt-get install -y wget gnupg lsb-release software-properties-common libz3-dev
          wget -q https://apt.llvm.org/llvm.sh
          chmod +x llvm.sh
          sudo ./llvm.sh 22
          sudo apt-get install -y libmlir-22-dev mlir-22-tools llvm-22-dev llvm-22-tools libpolly-22-dev
          echo "/usr/lib/llvm-22/bin" >> "$GITHUB_PATH"

      - uses: dtolnay/rust-toolchain@stable

      - uses: Swatinem/rust-cache@v2
        with:
          # Optional: narrow cache key to tooling crates + frontend deps
          shared-key: tooling

      - name: Build tooling binaries
        run: cargo build --release -p quonfmt -p quonlint -p quon_lsp

      - name: quonfmt --check (CI corpus)
        run: ./scripts/tooling-check.sh --ci --fmt-only

      - name: quonlint (CI corpus)
        run: ./scripts/tooling-check.sh --ci --lint-only

      - name: LSP smoke tests
        run: cargo test --release -p quon_lsp --test smoke -- --include-ignored
```

**Alternative (DRY bootstrap):** Extract LLVM install into `.github/actions/llvm22-mlir/action.yml` and call it from both `rust` and `tooling`. Nice-to-have, not blocking #49.

**Do not modify** existing `rust` or `docs` job steps except, if desired, a one-line comment pointing at the new job.

### B. No changes to other workflows

- `taskless.yml` — unchanged
- `flux.yml` — unchanged
- `coverage.yml` — unchanged

---

## Fixture corpus

### Manifest: `test/tooling/ci-corpus.txt`

One path per line, `#` comments allowed, paths relative to repo root. Checked into git; CI and `scripts/tooling-check.sh` read the same file.

**Proposed initial 16 files** (covers reference algorithms, stdlib surface syntax, and diverse corpus patterns):

```
# SPEC §12 reference algorithms
frontend/tests/fixtures/bell_state.qn
frontend/tests/fixtures/bernstein_vazirani.qn
frontend/tests/fixtures/grover.qn
frontend/tests/fixtures/shor.qn
frontend/tests/fixtures/qaoa.qn
frontend/tests/fixtures/ising.qn
frontend/tests/fixtures/error_correction.qn
frontend/tests/fixtures/stdlib_forms.qn
frontend/tests/fixtures/teleport.qn

# Corpus representatives (syntax / domain diversity)
frontend/tests/fixtures/corpus/deutsch_jozsa.qn
frontend/tests/fixtures/corpus/recursive_qft.qn
frontend/tests/fixtures/corpus/qaoa_maxcut.qn
frontend/tests/fixtures/corpus/phase_flip_code.qn
frontend/tests/fixtures/corpus/fermi_hubbard.qn
frontend/tests/fixtures/corpus/amplitude_amplification.qn
frontend/tests/fixtures/corpus/graph_state.qn
```

**Invariants:**

- Every manifest file must **parse and type-check** (already true for reference + corpus entries).
- Every manifest file must be **quonfmt-clean** before enabling the gate (one-time formatting PR or included in #46 landing).
- Adding a new reference fixture in future issues requires updating the manifest.

### Optional full corpus mode

`scripts/tooling-check.sh --full` expands to:

- All `frontend/tests/fixtures/*.qn`
- All `frontend/tests/fixtures/corpus/*.qn`
- Any `test/verify/*.qn` if present

Used locally and in release/phase milestones; not in default CI.

### Formatting drift policy

When `quonfmt --check` fails in CI:

1. Run `quonfmt <files>` locally (or `quonfmt --write` if supported).
2. Commit formatted output.
3. Re-run `./scripts/tooling-check.sh`.

Do **not** auto-format in CI (check-only keeps PRs reviewable).

---

## Lint threshold configuration

### Config file: `.quonlint.toml` (repo root)

Ship a minimal config committed with #47; #49 wires CI to it.

```toml
# Severity baseline for CI (issue #49)
[ci]
fail-level = "error"      # warn/info never fail CI
max-errors = 0            # any error fails
# max-warnings set after baseline measurement (Phase 1) — do NOT use arbitrary 50

# Rule overrides (align with quonlint rule IDs from #47)
[rules]
"depth/sequential-for-blowup" = "info"
"monad/circuit-bind-without-apply" = "error"
```

### CLI invocation in CI

Exact flags depend on #47's CLI surface; target shape:

```bash
quonlint --config .quonlint.toml \
  --fail-level error \
  --max-errors 0 \
  --files-from test/tooling/ci-corpus.txt
```

If `quonlint` supports project mode (`quonlint check .`), still pass `--files-from` in CI to bound work.

**Threshold tuning workflow:**

1. **Measure baseline** — run `quonlint --config .quonlint.toml --fail-level warn` on all 16 manifest files; count warnings per rule and total.
2. Set `max-warnings` to measured baseline (or `0` with rules tuned to info on gating paths).
3. Commit baseline counts in `.quonlint.toml` comments or a tracked issue.
4. Ratchet down incrementally in follow-up issues with numeric targets.

---

## LSP smoke test design

### Location: `quon_lsp/tests/smoke.rs`

Fast integration tests spawning `quon_lsp` as a subprocess with stdin/stdout pipes. No VS Code / Cursor harness.

### Test cases (target ≤ 30 s total)

| Test | Protocol sequence | Assertion |
| ---- | ----------------- | --------- |
| `handshake` | `initialize` → response → `initialized` → `shutdown` → `exit` | Correct capability flags; clean exit code 0 |
| `did_open_clean_file` | handshake → `textDocument/didOpen` (`bell_state.qn`) | `textDocument/publishDiagnostics` with empty or expected diagnostics |
| `did_change_incremental` | open → `textDocument/didChange` (single-char insert) | Updated diagnostics within one analysis cycle; no hang |
| `did_close` | open → `textDocument/didClose` | No crash; server still responds to shutdown |
| `malformed_rpc` | send invalid JSON-RPC frame after init | Server returns parse error response or disconnects gracefully (no panic) |

### Implementation guidelines

- Mark tests `#[ignore]` with reason `"tooling job only"`; tooling CI runs with `--include-ignored`.
- Use **`Content-Length` framed JSON-RPC** (LSP spec); share helpers in `quon_lsp/tests/support/mod.rs`.
- Set per-test timeout (`std::thread::spawn` + `join` with deadline, or `tokio::time::timeout` if server tests are async).
- Embed small fixture snippets via `include_str!` — do **not** read the full corpus in smoke tests.
- Avoid filesystem watchers; use `file:///` URIs with in-memory buffers from `didOpen`/`didChange`.
- Spawn server via `env!("CARGO_BIN_EXE_quon_lsp")` (match `quonc/tests/smoke.rs` pattern).

### CI invocation

```bash
cargo test --release -p quon_lsp --test smoke -- --include-ignored
```

Optional: `RUST_TEST_THREADS=1` if tests share global state (prefer isolated subprocesses instead).

---

## Runtime budget

| Phase | Cold cache (est.) | Warm cache (est.) |
| ----- | ----------------- | ----------------- |
| System deps (LLVM) | 3–4 min | 3–4 min (apt cache helps slightly) |
| `cargo build --release` (3 binaries) | 2–4 min | 30–90 s |
| `quonfmt --check` (16 files) | < 5 s | < 5 s |
| `quonlint` (16 files) | 5–15 s | 5–15 s |
| LSP smoke (5 tests) | 10–30 s | 10–30 s |
| **Total** | **≤ 10 min** (timeout) | **≤ 5 min** (target) |

**Guardrails:**

- `timeout-minutes: 10` on the job.
- Fixed file list — never glob the workspace in CI.
- No `cargo test --workspace` in the tooling job.
- LSP tests: cap each at 10 s; entire smoke suite ≤ 60 s.

If warm-cache runtime exceeds 5 min after landing, profile with `cargo build -p quon_lsp --timings` and consider MLIR-free frontend splitting (follow-up issue).

---

## Local reproduction commands

### Primary script: `scripts/tooling-check.sh`

Add a bash script (modeled on [`scripts/coverage.sh`](../../scripts/coverage.sh)) with:

```bash
#!/usr/bin/env bash
# Tooling quality gates (quonfmt, quonlint, LSP smoke) — mirrors CI tooling job.
#
# Usage:
#   ./scripts/tooling-check.sh              # full local gate (fmt + lint + LSP)
#   ./scripts/tooling-check.sh --ci         # exact CI corpus + thresholds
#   ./scripts/tooling-check.sh --fmt-only
#   ./scripts/tooling-check.sh --lint-only
#   ./scripts/tooling-check.sh --lsp-only
#   ./scripts/tooling-check.sh --full       # all .qn fixtures (slow)
#
# Prerequisites: same as cargo test (LLVM 22 + MLIR + z3 on PATH).

set -euo pipefail
# ... build release binaries if missing ...
# ... read test/tooling/ci-corpus.txt ...
# ... invoke quonfmt --check, quonlint, cargo test -p quon_lsp --test smoke ...
```

### Document in `docs/agents/validation.md`

Add a row to the CI matrix table:

| Workflow | Trigger | What runs |
| -------- | ------- | --------- |
| `ci.yml` `tooling` | every push and PR | `quonfmt --check`, `quonlint`, `quon_lsp` smoke tests on CI corpus |

Add a new section **Tooling gates (quonfmt · quonlint · LSP)** with:

```bash
# Match CI exactly
./scripts/tooling-check.sh --ci

# Individual gates
cargo build --release -p quonfmt -p quonlint -p quon_lsp
quonfmt --check $(grep -v '^#' test/tooling/ci-corpus.txt | grep -v '^$')
quonlint --config .quonlint.toml --fail-level error --files-from test/tooling/ci-corpus.txt
cargo test --release -p quon_lsp --test smoke

# Broader local sweep (not CI)
./scripts/tooling-check.sh --full
```

### Update `docs/agents/code-quality.md` pre-PR checklist

Insert after Taskless (item 4), renumber Flux:

5. **Tooling** — `./scripts/tooling-check.sh --ci` (or full script without flags)
6. **Flux (if needed)** — …

Optional one-liner in [`README.md`](../../README.md#testing) pointing to `validation.md#tooling-gates`.

---

## Integration with existing checks

### Validation stack (after #49)

```
Every PR
├── ci.yml :: rust        → fmt, clippy, build, test, Aer verify
├── ci.yml :: tooling     → quonfmt, quonlint, LSP smoke     [NEW]
├── ci.yml :: docs        → website build
├── taskless.yml          → ast-grep on changed files
├── flux.yml              → refinement types (path-filtered)
└── coverage.yml          → llvm-cov summary (non-blocking)
```

### Overlap / non-overlap

| Check | Overlap with tooling | Action |
| ----- | -------------------- | ------ |
| `cargo fmt` | Independent — Rust vs Quon source | Keep both |
| `frontend/tests/pretty_roundtrip.rs` | Related to quonfmt semantics | Keep; quonfmt golden tests in #46 remain crate-local |
| `frontend/tests/corpus.rs` | Same files, different assertion (typecheck only) | Keep; CI corpus is a subset for speed |
| Taskless | Different language | No merge |
| `cargo test --workspace` | LSP smoke tests marked `#[ignore]` — only run in tooling job via `--include-ignored` | Document policy in §5; no duplication |

### Branch protection

Ensure GitHub required checks include the new job name `quonfmt · quonlint · LSP smoke` (or whatever appears in the Actions UI). Coordinate with repo admin outside this PR if protection rules are manual.

---

## Implementation checklist

Execute in order after #43, #46, #47 merge.

### Phase 1 — Corpus and config scaffolding

- [ ] Add `test/tooling/ci-corpus.txt` (16 files above).
- [ ] Add manifest validation test (every path exists, is `.qn`, readable).
- [ ] Confirm every manifest file passes `quonfmt --check`; format in a prep commit if needed.
- [ ] Add `.quonlint.toml` with CI threshold section (or verify #47 ships it).
- [ ] **Measure baseline lint warning count** on corpus; set `max-warnings` to measured value (not arbitrary 50).

### Phase 2 — Local script

- [ ] Implement `scripts/tooling-check.sh` with `--ci`, `--fmt-only`, `--lint-only`, `--lsp-only`, `--full`.
- [ ] Make executable (`chmod +x`).
- [ ] Verify script on macOS (dev) and Ubuntu (act or CI).

### Phase 3 — LSP smoke tests (if not complete in #43)

- [ ] Add `quon_lsp/tests/support/mod.rs` — JSON-RPC framing helpers.
- [ ] Add `quon_lsp/tests/smoke.rs` — five tests listed above.
- [ ] Confirm `cargo test --release -p quon_lsp --test smoke` ≤ 60 s.

### Phase 4 — CI wiring

- [ ] Add `tooling` job to `.github/workflows/ci.yml`.
- [ ] Open PR; confirm three `ci.yml` jobs (`rust`, `tooling`, `docs`) + Taskless workflow pass.
- [ ] Deliberately introduce formatting drift in a throwaway commit; confirm `tooling` fails and `rust` still passes.

### Phase 5 — Documentation

- [ ] Update `docs/agents/validation.md` — matrix + tooling section + local commands.
- [ ] Update `docs/agents/code-quality.md` — pre-PR checklist item.
- [ ] Optional: README testing blurb.

### Phase 6 — Acceptance verification

- [ ] **Formatting drift** — unformatted `.qn` in corpus → CI red on `tooling`.
- [ ] **Lint threshold** — inject lint error (or use known-bad snippet in test-only file excluded from corpus) → CI red.
- [ ] **LSP smoke** — break handshake in throwaway branch → CI red.
- [ ] **Local repro** — fresh clone, follow `validation.md`, reproduce CI result.
- [ ] **Regression** — no removed steps from `rust` job; taskless/flux/coverage unchanged.

---

## Risks and mitigations

| Risk | Mitigation |
| ---- | ---------- |
| Tooling crates pull MLIR → slow job | Scoped `-p` build; shared rust-cache; follow-up MLIR-free split |
| Corpus drift (new fixtures unformatted) | Document in code-quality.md; optional follow-up Taskless rule on `*.qn` in fixtures paths |
| Flaky LSP smoke (timing) | Subprocess isolation; generous but bounded timeouts; no parallel tests on one server |
| Lint warning ratchet too noisy | Start with high `max-warnings`; tighten incrementally |
| Duplicate LLVM install across jobs | Accept for #49; extract composite action later |

---

## Out of scope for #49

- Running quonfmt/quonlint on **`test/verify/*.qn`** in default CI (add to `--full` only unless those files become formatting-critical).
- Editor extensions or Cursor-specific LSP integration tests.
- Merging Quon lint into Taskless ast-grep rules.
- Nightly full-corpus or full-workspace lint/format sweeps.
- Changing `flux.yml` or `coverage.yml` behavior.

---

## Suggested PR title and commit message

**PR title:** `ci: add tooling quality gates (quonfmt, quonlint, LSP smoke)`

**Commit message:**

```
Add parallel CI tooling job for quonfmt, quonlint, and LSP smoke tests.

Introduces a curated fixture manifest and scripts/tooling-check.sh so
developers can reproduce CI locally. Documents commands in validation.md.
```

---

## References

- Issue #49 — acceptance criteria source
- Issues #43, #46, #47 — tooling implementations this plan integrates
- [`docs/agents/validation.md`](../agents/validation.md) — CI matrix (to extend)
- [`docs/agents/code-quality.md`](../agents/code-quality.md) — pre-PR checklist (to extend)
- [`.github/workflows/ci.yml`](../../.github/workflows/ci.yml) — primary workflow to modify
- [`frontend/tests/corpus.rs`](../../frontend/tests/corpus.rs) — full corpus harness
- [`frontend/tests/pretty_roundtrip.rs`](../../frontend/tests/pretty_roundtrip.rs) — pretty-printer semantics baseline
