# Issue #138 plan review — adversarial approval

**Plan:** `docs/plans/issue-138-plan.md`  
**Reviewed against:** issue #138, `README.md`, `Cargo.toml`, `rust-toolchain.toml`, `quonc/src/main.rs`, `python/requirements.txt`, `python/quon_aer.py`, `test/verify/bell.qn`, `SPEC.md` §9, and the current Starlight configuration  
**Stance:** Assume a first-time user follows every command literally on macOS or Ubuntu and reject any undocumented assumption.

## Findings

### 1. The issue is nominally blocked by #137, which is absent here

The branch still has the original minimal site, so a plan that assumes an established sidebar would fail acceptance or expand into #137.

**Resolution:** The plan now permits exactly one minimal “Getting Started” sidebar group and explicitly excludes landing-page and broader IA work. This satisfies #138 while keeping the likely conflict small and additive.

### 2. The SPEC contains stale examples

`SPEC.md` shows a globally installed `quonc`, includes pseudocode that always calls `measure_all()`, and contains setup details that are less current than the executable bridge. Copying it would create a misleading quickstart.

**Resolution:** The plan establishes precedence: executable CLI and scripts over draft SPEC. It uses `./target/release/quonc`, the bridge's stdin mode, and the verified test fixture.

### 3. A Bell circuit without `main` is not a safe quickstart

`frontend/tests/fixtures/bell_state.qn` calls its entry function `hello_bell`, while the end-to-end verification fixture uses `main`. A user-facing sample must use the known emission entry convention.

**Resolution:** Use `test/verify/bell.qn` verbatim in substance, including `fn main()`.

### 4. Python installation can appear complete while QASM import still fails

Installing only `qiskit` and `qiskit-aer` omits `qiskit-qasm3-import`, which `qiskit.qasm3.loads` needs on current Qiskit.

**Resolution:** Install the checked-in `python/requirements.txt` inside a virtual environment instead of listing an incomplete ad hoc package command.

### 5. Native dependency instructions can leave Cargo unable to find LLVM

Installing LLVM 22 is insufficient when it is keg-only or installed under `/usr/lib/llvm-22`.

**Resolution:** Both platform paths culminate in `MLIR_SYS_220_PREFIX` and `PATH` exports, with shell-profile persistence guidance.

### 6. Exact Aer counts would be dishonest and brittle

Bell outcomes are sampled. Even with a fixed seed, output details can vary across Qiskit versions.

**Resolution:** Label output illustrative and assert only the invariant: keys `00` and `11`, approximately half the requested shots each. Use `--seed` to make the walkthrough reproducible where versions match.

### 7. The validation plan cannot rely on building the Rust compiler locally

This machine currently lacks discoverable `llvm-config`, which is exactly the clean-machine dependency the Install page addresses. Making a compiler build a documentation acceptance gate would conflate host setup with docs correctness.

**Resolution:** Website build is the required executable validation. CLI, source, and bridge claims are verified directly against code; an existing binary may supplement but not replace that audit.

### 8. Sidebar links can silently point at wrong slugs

Starlight content paths derive from the files under `src/content/docs`.

**Resolution:** Use explicit slugs `getting-started/install` and `getting-started/quickstart`; `pnpm build` must resolve both.

## Scope audit

- Flux: correctly N/A.
- Taskless: correctly N/A.
- Rust formatting: correctly N/A.
- No dependency or lockfile change is needed.
- No landing page, hardware target tutorial, language reference, CI, or deployment work is included.
- Expected product diff remains two pages plus one small sidebar edit.

## Verdict

**APPROVED FOR IMPLEMENTATION.**

All blocking ambiguities have explicit resolutions, every command has an identified repository source of truth, validation matches the documentation-only risk profile, and the expected diff is the smallest change that satisfies #138.

## Post-implementation adversarial review

The completed pages and sidebar were reviewed again as if copied command-for-command on a new machine.

- **Command accuracy:** `--emit-qasm` is defined by the current Clap interface; `--shots`, `--seed`, optional source input, stdin mode, and `QUONC` resolution are defined by `python/quon_aer.py`. The bridge's live `--help` output matches the guide.
- **Dependency accuracy:** LLVM/MLIR 22, Melior 0.27, libz3, stable Rust, and Python 3.10+ match `README.md`, workspace manifests, and CI. Python installation uses the checked-in requirements, including the separate QASM 3 importer.
- **Program accuracy:** The sample matches `test/verify/bell.qn`'s compiling `main`, gate order, allocation, and measurements.
- **Output claims:** The guide treats counts as samples and asserts only the Bell correlation invariant. It does not promise version-specific counts.
- **Rendered quality:** The first build exposed an unsupported `qn` syntax-highlighting warning and unrendered LaTeX. The source fence was changed to supported plain text and the state was expressed without an undeclared math plugin. A second build completed without content warnings and generated both expected routes.
- **Navigation and scope:** The sidebar links resolve to generated pages. No #137 landing-page work, source code, dependency, lockfile, CI, Taskless, or Flux changes were introduced.
- **Diff hygiene:** `git diff --check` passes; generated `node_modules/` and `dist/` remain ignored.

**PRODUCTION-QUALITY APPROVAL: APPROVED.**
