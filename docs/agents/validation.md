# Validation

Static analysis and refinement-type checks for the Quon workspace.

## CI matrix

| Workflow | Trigger | What runs |
| -------- | ------- | --------- |
| [ci.yml](../../.github/workflows/ci.yml) | every push and PR | `cargo fmt --check`, `clippy`, `build --release`, `test --workspace` on stable (excludes `flux_verify`; needs LLVM 22 + MLIR + z3) |
| [taskless.yml](../../.github/workflows/taskless.yml) | every PR; push to `main` | diff-scoped `@taskless/cli check` (Node 22+) |
| [flux.yml](../../.github/workflows/flux.yml) | PR when `flux_verify/` or lockfile changes; push to `main` | `cargo flux -p flux_verify` (nightly + z3) |

Not in CI yet: `lit test/lit/` (FileCheck IR tests) and Python Aer end-to-end checks — run locally per [README.md](../../README.md#testing).

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

1. **Every PR** — fmt, clippy, `cargo test --workspace --exclude flux_verify`, Taskless on changed files (see [code-quality.md](./code-quality.md#pre-pr-checklist)).
2. **Language / parser / typechecker work** — add or extend unit tests; keep reference-algorithm fixtures passing (`frontend/tests/reference_algorithms.rs`).
3. **Algorithms and serializers** — add proptest coverage where an oracle or invariant exists (`backend/tests/props.rs`, `mlir_bridge/tests/depth_props.rs`); consider `cargo fuzz` for byte/text parsers.
4. **MLIR dialect changes** — unit/integration tests in `mlir_bridge/tests/`; builders must call `verify()` (enforced by Taskless).
5. **Refinement-heavy Rust** — Flux specs in `flux_verify` when `{v: …}` contracts clarify intent beyond tests.

Slow checks (`lit`, Aer, full fuzz campaigns) are valuable before phase milestones but should not replace the fast layers above for day-to-day development.
