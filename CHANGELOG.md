# Changelog

All notable changes to Quon are documented in this file. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and this project
adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.3.0] - 2026-08-04

### Added

- **Controlled composition of named parametric circuits** (#374): `controlled(f())`
  now elaborates the callee's body into a concrete gate tree before distributing
  control, enabling controlled recursive circuit functions.
- **Exact state-preparation solver** (#397): `--na-state-prep exact` invokes a
  z3-backed SMT-optimal CZ-pair scheduler; the resource report labels the
  schedule `Exact` (proven) or `Heuristic` (solver timeout fallback). Non-solver
  builds reject `Exact` with a typed error — never a silent fallback.
- **Canonical frontend AST visitor** (#399): one traversal interface shared by
  analysis, formatter, linter, and LSP, replacing duplicated exhaustive walkers.
- **Load-bearing Flux refinement contracts** (#411–#414): arithmetic overflow,
  capacity, QEC sizing/distance, and lattice-surgery bounds specifications on
  the Rust implementation kernels.
- **Taskless as a failing gate** (#390): Rust convention rules (unwrap/expect,
  anyhow in libs, serde DTOs) now fail CI instead of warning silently.
- **Workspace Rustdoc CI gate** (#406): `RUSTDOCFLAGS="-D warnings" cargo doc`
  enforced in `ci-rust`; unresolved intra-doc links and private-item references
  are denied.
- **Documentation quality-gate contract** (#377): normative language reference,
  diagnostic catalog, feature-support matrix, backend-target docs, and
  contributor style system.

### Changed

- **Unsafe code denied workspace-wide** (#415): `[workspace.lints.rust]
  unsafe_code = "deny"` with `undocumented_unsafe_blocks = "warn"`; all unsafe
  MLIR FFI centralized in `mlir_bridge/src/ffi.rs` with `// SAFETY:` comments.
- **MLIR-free crate seams** (#407): `quon_na` and `frontend` no longer pull
  MLIR/Z3 by default; tooling consumers build without heavyweight compiler deps.
- **Backend invariants sealed** (#394, #402): `ConnectivityGraph`,
  `FixedTarget`, interaction graphs, and reports expose private fields with
  public accessors, preventing construction of invalid states.
- **All-to-all connectivity is O(N²)** (#408): analytic distance matrix
  construction, no Floyd-Warshall for fully-connected targets.
- **DepthExpr canonical ordering** (#398): sort by byte representation instead
  of allocating S-expression strings as sort keys.
- **Elaboration context shared by reference** (#400): `Arc<HashMap>` for
  parametric definitions instead of cloning the whole context per call.

### Fixed

- **Classical arithmetic is total and fallible** (#409): `eval_classical` uses
  checked arithmetic with `ElabError::{Overflow, DivByZero, NegativeExponent}`;
  no more panics on division by zero or integer overflow.
- **MLIR passes fail closed** (#391): transformation passes return typed errors
  through `Diagnostics` accumulators instead of `eprintln!`-and-continue.
- **Phase-polynomial optimization safe beyond 128 qubits** (#392): dynamic
  `Parity` bitset (`Vec<u64>`) replaces fixed `u128`; no shift overflow.
- **QEC workload deserialization validates state transitions** (#396):
  `QecWorkload` deserializes via `try_from` that replays `WorkloadBuilder`
  ordering validation; post-measure memory rounds are rejected.
- **OpenQASM ingestion rejects unsupported semantics** (#405): `measure`,
  `reset`, gate definitions, classical control flow, and multi-parameter gates
  are rejected with line-tagged errors instead of silently dropped.
- **Production `test.qubit` allocations replaced** (#401): verified
  `quantum.dynamic.alloc` op with dialect verifier, replacing the unregistered
  `test.qubit` placeholder.
- **Raw analysis sink pointers removed from TypeChecker** (#393): no more `*mut`
  sinks; analysis results are returned or borrowed safely.
- **Stale scaffolds and dead-code allowances removed** (#395): orphaned
  duplicate types, no-op ZX rewrite stubs, and crate-wide `#![allow(dead_code)]`.
- **Flux CI restored** (#404): `quon_qec` and `backend` Flux jobs pass;
  tautological specs replaced with load-bearing contracts.
- **MLIR bridge fuzz targets restored** (#410): fuzz targets import `DepthExpr`
  from `quon_core` directly; detached workspace builds with the stable toolchain.

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

[Unreleased]: https://github.com/arniber21/quon/compare/v0.3.0...HEAD
[0.3.0]: https://github.com/arniber21/quon/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/arniber21/quon/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/arniber21/quon/releases/tag/v0.1.0
