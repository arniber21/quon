# Contributing to Quon

Thanks for your interest in Quon. This guide is the fast path; the linked
docs have the details.

## Setup

Quon is a Rust workspace with native LLVM/MLIR 22 and Z3 dependencies.
Use Devbox for a reproducible environment:

```bash
git clone https://github.com/arniber21/quon.git
cd quon
devbox shell          # pins LLVM/MLIR 22, Z3, Python, Node, just, sccache, cargo-nextest
just doctor           # check required tools
cargo build -p quonc
```

All build, test, and lint commands go through `devbox run -- just <recipe>`
(or run them inside `devbox shell`). See [getting started][install] for the
full setup and troubleshooting.

## Daily workflow

```bash
just test-fast        # unit + integration tests (cargo-nextest; soft-skip lit)
just test-ci          # local CI-parity path (fmt + clippy + build + tests + tooling + docs)
just ci-rust          # the rust CI gate alone
just ci-tooling       # quonfmt + quonlint + LSP smoke
just ci-docs-assert   # assert agent validation docs match Justfile / CI
```

`just test-ci` is the local mirror of what CI runs. Run it before pushing.

## Branches and pull requests

Quon uses the [Graphite CLI][graphite] (`gt`) for stacked branches and PRs.
Trunk is `main` — never commit or submit PRs directly from local `main`.

```bash
gt sync                          # update from trunk
gt create my-branch -m "..."     # stack on trunk (or an existing branch)
# ... make changes ...
gt submit --no-interactive --no-edit   # open / update the PR
```

Use `gh` for issue-tracker operations (`gh issue view`, `gh issue create`).

## Code quality

- **Library crates** use `thiserror` + `Result`; no `unwrap`/`expect`/`anyhow`
  in `src/` (only `quonc` may use `anyhow::Result` for the CLI driver). See
  [`docs/agents/code-quality.md`](docs/agents/code-quality.md).
- **MLIR builders**: `OperationBuilder` → `build()?` → `verify(&op)?` before
  returning.
- **JSON wire DTOs**: `#[serde(deny_unknown_fields)]` on `Deserialize` structs.
- **Workspace lints**: `[workspace.lints.rust] unsafe_code = "warn"`
  workspace-wide; `deny` in crates with no unsafe. `lints.workspace = true`
  in every crate.
- Run `just test-ci` (or `devbox run -- just test-ci`) before submitting.

## Validation

- Taskless ast-grep rules and Flux refinement checks — see
  [`docs/agents/validation.md`](docs/agents/validation.md).
- Prefer `just test-ci` for CI-parity gates. Run
  `npx @taskless/cli@latest check` on changed files.

## Domain language

Single-context repo: [`CONTEXT.md`](CONTEXT.md) is the domain glossary;
[`docs/adr/`](docs/adr/) holds architectural decisions. Read both before
making design changes — match the established vocabulary.

## Samples

New samples go under [`samples/`](samples/) with a row in
[`samples/catalog.yaml`](samples/catalog.yaml). See
[`samples/CONTRIBUTING.md`](samples/CONTRIBUTING.md) for the taxonomy,
catalog schema, and CI smoke requirements.

## Licensing

Quon is licensed under the [Business Source License 1.1](LICENSE) (BSL). After
the Change Date, it converts to Apache License 2.0. By contributing, you agree
your contributions will be licensed under the same license.

[install]: https://quon.arnabg.me/getting-started/install/
[graphite]: https://graphite.dev/docs
