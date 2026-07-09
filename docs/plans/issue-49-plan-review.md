# Issue #49 Plan Review — Adversarial Grade

**Plan:** `docs/plans/issue-49-plan.md`  
**Reviewed against:** `.github/workflows/ci.yml`, `docs/agents/validation.md`, `docs/agents/code-quality.md`, fixture layout, existing `quonc/tests/smoke.rs` patterns  
**Reviewer stance:** Adversarial — assume the plan will fail in CI unless gaps are closed.

---

## Overall Grade: **C+**

## Pass / Fail: **CONDITIONAL FAIL**

The plan is directionally sound and matches repo conventions (parallel CI job, `scripts/` entrypoint, `validation.md` updates, Taskless separation). It is **not safe to execute as written** without resolving critical blockers around workspace test interaction, upstream CLI contracts, and a few incorrect acceptance mappings.

---

## Critical Blockers

### 1. LSP smoke tests will also run in the `rust` job (plan does not address this)

Once `quon_lsp` is a workspace member, `cargo test --workspace --exclude flux_verify` in the existing `rust` job will run `quon_lsp/tests/smoke.rs` integration tests **by default**, unless they are `#[ignore]` or otherwise excluded.

The plan says:
- smoke tests must **not** require `--ignored` in default CI
- the tooling job exists for **isolation** and **bounded runtime**
- "Smoke tests are the CI contract; workspace tests stay as developer signal"

These statements conflict. The repo already runs heavyweight integration smoke in CI via `quonc/tests/smoke.rs` without a separate job — the same pattern will apply to `quon_lsp`.

**Required fix:** Pick one policy and document it explicitly:

| Option | Mechanism | Tradeoff |
|--------|-----------|----------|
| **A (recommended)** | `#[ignore]` on LSP smoke tests; tooling job runs `cargo test --release -p quon_lsp --test smoke -- --include-ignored` | LSP failures only surface in `tooling` job; rust job stays unchanged |
| **B** | Accept LSP smoke in `rust` job; drop LSP from tooling job | Tooling job becomes fmt/lint-only; still meets "LSP smoke on each PR" |
| **C** | Separate non-workspace test crate | Heavy; probably overkill |

Also require `env!("CARGO_BIN_EXE_quon_lsp")` and a `[[bin]]` dependency — the established pattern in `quonc/tests/smoke.rs` — which the plan omits.

### 2. Upstream CLI/config contracts are assumed, not pinned

The plan is blocked on #43, #46, #47 but specifies concrete interfaces that **do not exist in the repo today** (`quonfmt`, `quonlint`, `quon_lsp` crates are absent; workspace members are only `quonc`, `frontend`, `zx`, `mlir_bridge`, `backend`, `flux_verify`, `quon_core`).

Assumed without verification:
- `quonfmt --check`
- `quonlint --config .quonlint.toml --fail-level error --max-errors 0 --files-from …`
- `.quonlint.toml` with a `[ci]` section and rule names like `suspicious-depth-growth`

**Required fix:** Add a **"Upstream interface contract"** section listing exact flags, config schema, exit codes, and binary names that #43/#46/#47 must deliver before #49 merges. Include a pre-flight checklist: `cargo tree -p quonfmt`, `--help` snapshots, dry-run on corpus.

### 3. "Docs" row in failure semantics is false

| Gate | Fail condition |
|------|----------------|
| Docs | Missing/outdated repro commands in `validation.md` | CI docs describe local reproduction |

Nothing in the proposed CI validates documentation. This is manual acceptance only — do not map it to a CI gate in the failure semantics table.

---

## Major Concerns

### 4. Duplicate LLVM bootstrap across jobs (cost + flake surface)

`ci.yml` `rust`, the proposed `tooling` job, and `coverage.yml` each install LLVM 22 + MLIR (~3–4 min cold). On every PR that's potentially **two required installs** (`rust` + `tooling`) plus optional coverage.

The plan accepts duplication and defers composite action extraction. For a 10-minute tooling timeout with 3–4 min LLVM + 2–4 min cold `cargo build --release`, margin is thin.

**Amendment:** Either extract `.github/actions/llvm22-mlir` as a **Phase 4 prerequisite** (not nice-to-have), or document expected PR latency increase and consider `concurrency: cancel-in-progress` if not already present.

### 5. `Swatinem/rust-cache` `shared-key: tooling` isolates caches

The `rust` job uses the default cache key; tooling uses `shared-key: tooling`. They will **not** share compiled artifacts. The tooling job pays a largely independent cold/warm build tax for `frontend` → `mlir_bridge` deps even when `rust` just built the same graph.

**Amendment:** Drop `shared-key: tooling` unless there's a measured collision problem, or document that duplicate compiles are intentional and budgeted.

### 6. CI YAML vs `scripts/tooling-check.sh` parity is underspecified

CI runs:
1. explicit `cargo build --release -p …`
2. `./scripts/tooling-check.sh --ci --fmt-only`
3. `./scripts/tooling-check.sh --ci --lint-only`
4. direct `cargo test … --test smoke`

But `validation.md` tells developers `./scripts/tooling-check.sh --ci` matches CI exactly. The script skeleton says "build release binaries if missing" — ambiguous whether it rebuilds always (wasteful) or uses `target/release/quonfmt` (correct).

**Amendment:** Specify script contract:
- never rebuild if `$ROOT/target/release/quon{fmt,lint}` exist and are newer than lockfile
- `--ci` runs fmt + lint + LSP in same order as CI
- CI may call sub-flags for step granularity, but `--ci` must be the single local repro entrypoint

### 7. Lint gate weakened by `max-warnings = 50`

Starting with 50 warnings means CI can pass with substantial lint debt on the corpus. The ratchet policy is hand-waved.

**Amendment:** Phase 1 must **measure baseline warning count** on the corpus and set `max-warnings` to baseline (or `0` with rules tuned to `warn` only on non-gating paths). Document ratchet as a tracked issue with numeric target.

### 8. Corpus manifest lacks validation harness

Proposed manifest has **16 files**, not "~15" as stated repeatedly. All paths exist today (verified against `frontend/tests/fixtures/`), but nothing in the plan prevents:
- typos in `test/tooling/ci-corpus.txt`
- stale entries after renames
- new reference fixtures added without manifest update

**Amendment:** Add a small Rust test or script assertion (e.g. in `quonfmt` or a `test/tooling/validate_manifest.rs`) that every manifest path exists, is `.qn`, and is UTF-8 readable. Optionally assert manifest ⊆ `{fixtures/*.qn, fixtures/corpus/*.qn}`.

### 9. `quonfmt` vs `pretty_roundtrip` semantic drift unaddressed

`frontend/tests/pretty_roundtrip.rs` validates AST roundtrip via `frontend::pretty::pretty` on reference fixtures. `quonfmt` may diverge if #46 uses a different code path. CI could pass fmt while roundtrip fails (or vice versa).

**Amendment:** Explicitly state invariant: `quonfmt` must share the same pretty core as `frontend::pretty`, or add a cross-check test in #46 that corpus manifest files pass both.

### 10. Phase 4 checklist error: Taskless is not a `ci.yml` job

Phase 4 says: "confirm all four jobs (`rust`, `tooling`, `docs`, Taskless) pass."

Taskless runs in **`.github/workflows/taskless.yml`**, a separate workflow. Required checks span workflows, but this wording will confuse implementers.

### 11. Branch protection is out-of-band with no verification step

Plan correctly notes manual admin coordination but provides no acceptance step to confirm the new check is **required** before merge. A green but non-blocking job satisfies "runs on each PR" but not "fails CI" in practice.

**Amendment:** Add Phase 6 item: verify branch protection / ruleset lists `quonfmt · quonlint · LSP smoke` as required (or document org policy if checks are auto-discovered).

### 12. Acceptance "lint threshold" test avoids the corpus

Phase 6 proposes injecting a lint error in a "test-only file **excluded from corpus**." That validates CLI exit codes, not the CI wiring path through `ci-corpus.txt`.

**Amendment:** Require a throwaway commit that adds a known-bad `.qn` snippet **to the manifest** (or temporarily swaps a corpus file), confirms `tooling` goes red, then reverts.

---

## Minor Nits

1. **File count:** Manifest lists 16 files; plan says "~15" in goal, runtime budget, and checklist — fix arithmetic.
2. **`insta` mention for LSP tests:** LSP smoke is protocol assertion, not snapshot testing; misleading reference.
3. **`validation.md` grep example:** `quonfmt --check $(grep …)` breaks on paths with spaces; script should read manifest line-by-line (as planned for the script, not for docs examples).
4. **`permissions: contents: read`:** Good on tooling job; consider matching `taskless.yml` / `flux.yml` explicitly (rust job lacks it today — not a regression, but inconsistent).
5. **Reference fixture count:** Plan cites "8 SPEC §12 reference algorithms"; `pretty_roundtrip.rs` covers 9 files including `stdlib_forms.qn` — align terminology.
6. **`test/tooling/` directory:** New convention alongside `frontend/tests/fixtures/` — fine, but note in plan why not `frontend/tests/tooling/` for co-location with fixtures.
7. **No mention of adding workspace members** in root `Cargo.toml` — implied by blockers, worth one checklist line.
8. **LSP `malformed_rpc` test:** "disconnects gracefully (no panic)" is hard to assert reliably; specify observable behavior (JSON-RPC parse error response with known code, or clean stderr + exit).

---

## Specific Required Plan Amendments

1. **Add § "Upstream interface contract (hard deps on #43, #46, #47)"** with exact CLI flags, config schema, binary names, exit codes; block #49 until signed off.
2. **Add § "Workspace test interaction policy"** resolving LSP smoke duplication (`#[ignore]` + `--include-ignored` recommended); reference `quonc/tests/smoke.rs` + `CARGO_BIN_EXE_*` pattern.
3. **Remove or relabel Docs row** in failure semantics — manual doc update only, not a CI gate.
4. **Fix manifest count** to 16; add manifest validation test/script in Phase 1.
5. **Tighten lint baseline:** measure warnings on corpus before setting `max-warnings`; default to baseline not 50.
6. **Define `scripts/tooling-check.sh` binary resolution** (`target/release/*`, no redundant rebuild); ensure `--ci` runs all three gates.
7. **Revise rust-cache strategy** — drop `shared-key: tooling` or justify with metrics.
8. **Elevate LLVM composite action** from nice-to-have to Phase 4 item if 10-min timeout stays.
9. **Fix Phase 4 wording:** "three `ci.yml` jobs + Taskless workflow."
10. **Add branch protection verification** to Phase 6.
11. **Corpus lint acceptance:** test failure through manifest, not excluded file.
12. **Add quonfmt/pretty invariant** cross-reference to #46 acceptance.
13. **Add pre-merge invariant:** all manifest files must pass `quonfmt --check` (prep commit) — already listed but make it a **hard gate before enabling CI step**.

---

## Grading Rubric Summary

| Criterion | Grade | Notes |
|-----------|-------|-------|
| **Correctness** | C | Core CI shape is right; workspace/LSP interaction and docs-as-gate are wrong |
| **Completeness** | B- | Good phases, docs, corpus, risks; missing upstream contract, manifest validation, branch protection |
| **Feasibility** | B | Executable after #43–#47; 10-min timeout tight with duplicate LLVM |
| **Test rigor** | C+ | Manual throwaway commits good; weak automated guards; lint acceptance avoids corpus |
| **Missed risks** | C | Cache isolation, double LLVM, fmt/pretty drift, corpus/manifest drift |
| **Repo conventions** | B+ | Matches `scripts/coverage.sh`, validation stack docs, Taskless separation, Graphite implied |

---

## Verdict

**Do not implement until blockers 1–3 are amended.** The parallel `tooling` job design is appropriate; the curated corpus strategy is sensible; documentation updates align with `validation.md` and `code-quality.md`. The plan reads like a strong first draft that assumes happy-path upstream tooling and underestimates interaction with the existing `rust` workspace test job.
