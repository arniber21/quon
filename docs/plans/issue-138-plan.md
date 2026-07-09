# Issue #138 — Install guide and Bell-to-Aer quickstart

**Branch:** `issue-138`  
**Scope:** Starlight documentation only  
**Dependency:** #137 defines the broader website information architecture. It is not present on this branch, so this change will add only the minimal Getting Started sidebar entries required by #138 and avoid landing-page or broader navigation work.

## Problem

The website has no usable onboarding path: its only page is an empty index. A new user must currently infer native dependencies, build environment variables, Python setup, compiler invocation, and the Aer bridge from several repository files. The issue requires one accurate path from a clean machine to a simulated Bell pair.

## Source-of-truth audit

The documentation will be derived from:

- `README.md` for supported prerequisites, platform setup, release build, and the end-to-end command.
- `Cargo.toml` and `rust-toolchain.toml` for Melior 0.27, LLVM/MLIR 22, Rust stable, and edition/toolchain expectations.
- `quonc/src/main.rs` for the current `SOURCE --emit-qasm` CLI contract and generic target default.
- `python/requirements.txt` and `python/quon_aer.py` for Python packages, stdin behavior, `--shots`, and `--seed`.
- `test/verify/bell.qn` for a known compiling `main` entry point and Bell measurement program.
- `SPEC.md` §9 for the OpenQASM/Aer boundary.

Where the draft SPEC differs from executable code, the CLI and scripts win.

## Design

1. Add `website/src/content/docs/getting-started/install.md`.
   - State supported prerequisites and why each is needed.
   - Give macOS Homebrew and Ubuntu/Debian apt.llvm.org commands.
   - Export `MLIR_SYS_220_PREFIX` and prepend its `bin` directory to `PATH`.
   - Create a local Python virtual environment and install `python/requirements.txt`.
   - Build with `cargo build --release` and verify both compiler and Python bridge with existing commands.
2. Add `website/src/content/docs/getting-started/quickstart.md`.
   - Start from the completed install guide and repository root.
   - Create `bell.qn` using the repository's verified Bell fixture syntax and a `main` entry point.
   - Explain H, CNOT, measurement, and expected correlated results briefly.
   - Show OpenQASM emission, then the exact pipe into `python/quon_aer.py`.
   - Use a seed for reproducibility while describing output statistically rather than promising exact counts.
3. Update `website/astro.config.mjs` with a narrow “Getting Started” sidebar linking Install and Quickstart.
4. Preserve the existing empty home page because landing-page work belongs to #137.

## Validation

- Run `pnpm install --frozen-lockfile` in `website/` if dependencies are absent.
- Run `pnpm build` in `website/`; this checks Starlight content, links, configuration, and static generation.
- Verify every documented flag against `quonc/src/main.rs` and, where a built binary is available, `quonc --help`.
- Verify the Bell program against `test/verify/bell.qn`.
- Verify Python setup and behavior against `python/requirements.txt` and `python/quon_aer.py`.
- Inspect the final diff for scope, prose, stale commands, accidental planned-only flags, and generated files.

## Flux and Taskless

- **Flux:** N/A. No Rust implementation or refinement specification changes.
- **Taskless:** N/A. Only Markdown and Starlight configuration change; no `.taskless/` rule applies to this documentation task.
- **Rust formatting:** N/A. No Rust files change.

## Edge cases

- LLVM is installed but not discoverable: explicitly persist `MLIR_SYS_220_PREFIX` and `PATH`.
- Multiple Python installations: use `python3 -m venv` and invoke the activated environment's `python`.
- Aer import fails despite `qiskit`: install the repository requirements, including `qiskit-aer` and `qiskit-qasm3-import`.
- Users run commands outside the repository root: state the working-directory assumption.
- Shot counts vary: promise only `00`/`11` outcomes near a 50/50 split, with seeded sample output labeled as illustrative.
- The compiler is not installed globally: consistently use `./target/release/quonc`.

## Risks and mitigations

- **#137 conflict:** Keep the sidebar edit additive and minimal so it can be reconciled with the broader IA.
- **Platform package drift:** Link apt.llvm.org and use the same LLVM 22 package names as repository CI/README.
- **SPEC drift:** Do not copy the SPEC's outdated bare `quonc` examples or its bridge pseudocode; use executable sources.
- **Over-scoping:** Do not add landing content, conceptual guides, API references, hardware targeting, or deployment changes.

## Expected diff shape

- Two product documentation pages under `website/src/content/docs/getting-started/`.
- One small Starlight sidebar configuration change.
- This plan and its adversarial review as execution records.
- No source, dependency, lockfile, CI, Flux, or Taskless changes.

## Implementation sequence

1. Record adversarial plan review and resolve every blocking finding.
2. Add Install and Quickstart pages.
3. Add minimal sidebar links.
4. Build the website.
5. Perform adversarial code/docs review and source verification.
6. Commit with `#138`, track the Graphite branch, and submit with a rich PR body.
