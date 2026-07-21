# Changelog

All notable changes to Quon are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-21

### Added

- **QEC experiment dual-emit** (`--emit-qec-experiment`): versioned semantic
  `*.qec.json` + sibling structure-only `.stim` circuit from one `quon_qec`
  workload IR pass (ADR-0018, #255/#264).
- **Python Stim/Sinter harness** (`python/quon_qec_sinter.py`): loads the
  dual-emit pair, annotates noise from the JSON `error_model` (ADR-0024),
  samples logical failures, emits CSV + sampled-evidence JSON with SHA-256
  provenance and Wilson confidence intervals (#253/#265).
- **Fused QEC validation report** (`--emit-qec-validation`): compiles,
  dual-emits, samples through Stim/Sinter, and fuses analytic + sampled
  evidence into `*.validation.json` + `.md` with provenance enforcement
  (ADR-0020 amendment, #280/#287).
- **QEC ablation benchmarks** (`python/quon_qec_benchmarks.py`): workload ×
  compiler-ablation grid with nested tiny Sinter samples; separate Sinter CSV
  + optional join CSV (ADR-0023, #254/#269).
- **Surface-code Clifford memory workload** and **logical CX via fixed-layout
  three-patch lattice surgery** (ADR-0019, #249/#267, #250/#268).
- **Magic-state-consuming logical T and CCZ operations** in the QEC workload IR
  (#283/#288).
- **qLDPC-style workload IR and resource model** prototype (#285/#293).
- **Generalized lattice-surgery planning** beyond the fixed CX template
  (#281/#291).
- **Mid-circuit measurement, reset, and qubit reuse** as first-class neutral-
  atom schedule resources (#282/#289).
- **Full single-qubit gate representation** in neutral-atom schedules (local
  rz, global ry, u3, #315).
- **End-to-end schedule fidelity estimate** (Enola Eq. (1)):
  `gate_fidelity_product` / `estimated_fidelity` in the resource report
  (#305/#327).
- **`--emit-na-stats`**: per-stage compiler-internals telemetry artifact
  (#307/#314).
- **Full RAP Table I sweep** + qmap-comparable CSV harness (#306/#330).
- **Business Source License 1.1** (BSL) added to the repository.
- **`CHANGELOG.md`** (this file).
- **Root `CONTRIBUTING.md`** contributor on-ramp.
- **CI**: `concurrency` cancel-in-progress, pip wheel cache, macOS runner on
  the rust job.
- **Workspace lints**: `[workspace.package]` inheritance (version, edition,
  rust-version, license, repository) and `[workspace.lints.rust]` with
  `unsafe_code` enforcement.
- **Devbox**: `sccache` and `cargo-nextest` added; `just`/`libxml2`/`zlib`
  pinned.
- **`clippy.toml`** with MSRV for MSRV-aware lint suggestions.

### Changed

- **Neutral-atom FT compiler demo** website page — the strongest end-to-end
  walkthrough (typed source → verified schedule → QEC experiment + Stim →
  resource report → fused validation report, #279/#290).
- Stim and Sinter now ship in the main `python/requirements.txt` (ADR-0022).
- Analytic resource reports and Sinter CSVs are kept as separate primary
  artifacts (ADR-0020, #246/#266).
- `quon_qec` is a shared workspace crate for QEC workload IR (ADR-0015).
- `cargo test` swapped to `cargo nextest run` in `just` recipes for faster,
  better-isolated test execution.

### Fixed

- Ancilla footprint reset after logical-CX `MeasureAncilla` (#313).
- False completeness claims in aware-search docs (#329).
- Stale non-Clifford claims in docs; promoted `--emit-qec-validation` demo
  (#312).

## [0.1.0] - 2026-07-14

Initial tagged release of the Quon quantum compiler toolkit: typed frontend,
MLIR lowering pipeline, OpenQASM 3 emission, neutral-atom schedule/resource
artifacts, Qiskit Aer verification seam, `quonfmt`/`quonlint`/`quon_lsp`
tooling, Tree-sitter grammar, editor integrations, and Devbox bootstrap.

[Unreleased]: https://github.com/arniber21/quon/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/arniber21/quon/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/arniber21/quon/releases/tag/v0.1.0
