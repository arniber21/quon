# Code quality

Agent guide for writing and reviewing Rust in the Quon workspace. The goal is **near-instant correctness feedback**: small, typed changes that compile, pass tests, and satisfy static checks without manual archaeology.

Full validation commands live in [validation.md](./validation.md). PR workflow is in [graphite.md](./graphite.md).

## Pre-PR checklist

Run these before opening or updating a PR (same order CI uses):

1. **Format** — `cargo fmt --all -- --check`
2. **Clippy** — `cargo clippy --workspace --exclude flux_verify --all-targets -- -D warnings`
3. **Tests** — `cargo test --workspace --exclude flux_verify`
4. **Taskless** — `npx @taskless/cli@latest check $(git diff --name-only main...HEAD)` (or full scan)
5. **Flux (if needed)** — `cargo flux -p flux_verify` when you touch refinement specs or the `flux` feature (see below)

Optional but valuable before large IR or emitter changes:

- `lit test/lit/` — FileCheck IR tests (not in CI yet)
- `cargo +nightly fuzz run …` in `mlir_bridge/fuzz/` — continuous fuzzing for parsers
- Python Aer checks — see [README.md](../../README.md#testing)

## Constant evaluation mindset

Treat correctness as something you **evaluate constantly**, not only at integration time. Prefer checks that return in seconds:

| Layer | What it gives you | Examples in this repo |
| ----- | ----------------- | --------------------- |
| **Unit tests** | Concrete examples, regression locks | `frontend/tests/reference_algorithms.rs` — every SPEC §12 fixture must lex and parse |
| **Property / fuzz tests** | Randomized invariants, differential oracles | `backend/tests/props.rs` — Floyd-Warshall vs petgraph Dijkstra; `mlir_bridge/tests/depth_props.rs` — `DepthExpr` S-expr round-trip |
| **cargo-fuzz** | Unbounded byte streams, panic-freedom | `mlir_bridge/fuzz/fuzz_targets/fuzz_depth_parse.rs` — parse never panics |
| **Type checker** | Language invariants on real programs | `frontend/src/typecheck.rs` — linear context `Δ`, circuit qubit counts, `Circuit<n,m,d,C>` |
| **IR verifiers** | Structural invariants on MLIR | `mlir_bridge/src/dialect/quantum_circ.rs` — `verify()` on every builder |
| **Static rules** | Repo-specific antipatterns | `.taskless/rules/` — unwrap/expect, anyhow in libs, etc. |
| **Flux** | Refinement proofs on small Rust kernels | `flux_verify/src/lib.rs` — specs like `{v: x < v}` |

**Patterns to follow:**

- Put **pure functions** behind property tests when an independent oracle exists (graph algorithms, parsers, serializers).
- Assert **invariants** (symmetry, triangle inequality, round-trip) not single hard-coded cases when the input space is large.
- Use **fixture tests** for end-user-visible acceptance criteria (reference algorithms, stdlib forms).
- Reserve slow paths (LLVM build, lit, Aer) for phase gates; do not rely on them for everyday feedback.

When adding fallible parsing or serialization, add at least: happy-path unit tests, one error-path test, and a proptest or fuzz target if input is text or bytes.

## Type system as invariant modeling

**Make illegal states unrepresentable.** If a value cannot exist in well-formed Quon or IR, do not model it as a bare `String`, `i64`, or untagged enum variant.

| Domain | Type-level model | Where |
| ------ | ---------------- | ----- |
| Quon types | `Ty`, linear `Circuit { n, m, d, c }` | `frontend/src/types.rs`, `frontend/src/ast.rs` |
| Depth bounds | `DepthExpr` AST + S-expr wire format | `frontend/src/refinement.rs`, `mlir_bridge/src/dialect/depth.rs` |
| Backend topology | `ConnectivityGraph`, `BackendTarget` after `TryFrom` | `backend/src/target.rs`, `backend/src/descriptor.rs` |
| MLIR ops | Typed builders + `VerifyError` | `mlir_bridge/src/dialect/quantum_circ.rs` |
| Refinement (Rust) | Flux `#[spec(...)]` on small functions | `flux_verify/` (nightly, separate CI job) |
| Refinement (Quon) | Z3 only when symbolic depths must unify | `frontend/src/refinement.rs` — pure constants skip Z3 |

**Two refinement tracks:**

1. **Quon `DepthExpr` + Z3** — symbolic depth arithmetic in the language and MLIR attributes. Z3 runs only when annotations must be proved or match branches unify (`RefinementCtx` in `frontend/src/refinement.rs`). Pure-constant expressions never call Z3.
2. **Flux on Rust** — optional refinement types for Rust implementation code. Lives in `flux_verify` behind nightly; does not block the stable workspace build. Use for new algorithms where a `{v: …}` spec is clearer than tests alone.

**"If it compiles, it's as correct as possible"** within the modeled invariants: lean on `Result` instead of `unwrap`, serde `deny_unknown_fields` on wire DTOs, builder functions that call `verify()` before returning, and the type checker for linearity and circuit typing. Compilation plus tests plus Taskless/Flux is the default bar for library changes.

## Error handling by crate

| Crate | Error style | Notes |
| ----- | ----------- | ----- |
| **quonc** | `anyhow::Result` | CLI driver; may aggregate errors from the pipeline (`quonc/src/main.rs`). |
| **frontend** | `thiserror` (`TypeError`, etc.) | Library API returns typed errors; span-aware reporting via `ariadne`. |
| **backend** | `thiserror` (`BackendError`) | Descriptor JSON → domain conversion; every fallible path returns `Result` (`backend/src/error.rs`). |
| **mlir_bridge** | `thiserror` per module + **`Diagnostics` monad** | Verifiers return `Result<(), E>`; passes accumulate with `Diagnostics::report` and flush once at the FFI boundary (`mlir_bridge/src/diagnostics.rs`). |
| **zx** | Typed errors (follow workspace convention) | Graph transforms; no `anyhow` in library code. |

**Diagnostics monad (mlir_bridge):** Dialect verifiers and passes stay pure Rust. They build a `Diagnostics` accumulator, fold `Result` values with `.report(location, result)`, and only `Diagnostics::emit` crosses into unsafe MLIR C API. Do not call `mlirEmitError` outside `diagnostics.rs`.

**anyhow:** Reserved for the **quonc** binary. Library crates should use `thiserror` enums (or plain `Result<T, E>` with a small `E`). Taskless rule `no-anyhow-in-lib-src` enforces this on new code.

## Defensive programming

- **Prefer typed errors over `assert!` / `unwrap` / `expect`** in library `src/`. Tests and `quonc` may use `expect` sparingly; Taskless flags unwrap/expect in lib sources.
- **Validate at boundaries** — JSON descriptors (`TargetDescriptor` with `deny_unknown_fields`), S-expr parsers (`DepthParseError`), MLIR op builders (`verify` after `build()`).
- **Fail with context** — `thiserror` messages name the field, index, or gate; MLIR diagnostics carry `Location`.
- **Do not use panics for user input** — lex/parse/type errors are recoverable and reported with spans.

## Antipatterns

| Antipattern | Prefer instead | Why |
| ----------- | -------------- | --- |
| `unwrap()` / `expect()` in lib `src/` | `?` and typed errors | Taskless `no-unwrap-expect-in-src`; panics hide bugs in production paths |
| `anyhow` in library crates | `thiserror` enums | Erases error structure; only `quonc` aggregates |
| `assert!` for invalid user/config input | `Result` + error enum | Assertions are for impossible internal states only |
| Skipping `verify()` after `OperationBuilder` | `build()?; verify(&op)?;` | Builders in `quantum_circ.rs` always verify — see Taskless rule |
| Wire DTO without `deny_unknown_fields` | `#[serde(deny_unknown_fields)]` on JSON structs | Typos in device JSON fail fast — `backend/src/descriptor.rs` |
| Calling Z3 for constant depths | Evaluate `DepthExpr` literally | Z3 only for symbolic proof obligations |
| Large untyped `HashMap<String, _>` in domain code | Parsed keys + validated indices | See `BackendTarget` conversion from descriptor |
| Fixing fmt/clippy only in CI | Run locally in pre-PR checklist | Same commands as `.github/workflows/ci.yml` |
| Relying on integration tests alone | Unit + proptest + typecheck fixtures | Near-instant feedback on every save |

## Per-crate cheat sheet

| Crate | Role | Quality focus |
| ----- | ---- | ------------- |
| **frontend** | Lex, parse, typecheck, Z3 refinement | Fixture tests for SPEC algorithms; `TypeError` / `Ty`; `DepthExpr` in AST |
| **mlir_bridge** | MLIR dialects, passes, emit | `Diagnostics`; `quantum_circ` builders + `verify`; `depth_props` / fuzz for parsers |
| **backend** | Target descriptor, connectivity, gates | `BackendError`; `deny_unknown_fields`; `props.rs` differential tests |
| **zx** | ZX-calculus rewriting | Graph invariants; typed errors |
| **quonc** | CLI driver | `anyhow`; clap; thin orchestration — no heavy logic |
| **flux_verify** | Flux refinement demos | `cargo flux`; nightly only |

## Related docs

- [validation.md](./validation.md) — Taskless, Flux, CI matrix, when to run each check
- [domain.md](./domain.md) — `CONTEXT.md`, ADRs, glossary vocabulary
- [README.md](../../README.md#testing) — full test commands and Flux install
