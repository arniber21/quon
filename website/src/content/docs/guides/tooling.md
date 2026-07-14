---
title: Developer tooling
description: Run Quon's language server, formatter, and linter from a source checkout.
---

Quon ships a language server, formatter, and linter in the repository, plus
first-party editor packages for VS Code (`extensions/vscode-quon/`), Neovim
(`nvim-quon/`), and Zed (`extensions/zed-quon/`). Build the tools from source
and use those packages (or wire the binaries into another editor) as described
below.

## Build the tools

From the repository root:

```sh
cargo build --release -p quon_lsp -p quonfmt -p quonlint-cli
```

This creates `target/release/quon_lsp`, `target/release/quonfmt`, and
`target/release/quonlint`. Building requires the same LLVM 22, MLIR, and Z3
environment as the rest of the Quon workspace.

## Language server

`quon_lsp` is an LSP server that communicates over standard input and output. Run it
directly:

```sh
target/release/quon_lsp
```

Configure an LSP client to:

- start that command for files ending in `.qn`;
- use `quon` as the language ID; and
- communicate with the process over stdio.

The server currently provides:

- compiler and `quonlint` diagnostics as you edit;
- hover information for resolved symbols, gates, and built-ins;
- completion for keywords, gates, built-ins, visible symbols, types, and a function
  snippet;
- go-to-definition for symbols and type aliases in the current document;
- full-document semantic tokens; and
- quick-fix and rewrite code actions supplied by compiler diagnostics.

Analysis is debounced by 100 ms by default. Set `QUON_LSP_DEBOUNCE_MS` on the server
process to change that delay. Lint diagnostics run after parsing and type analysis
succeed, using a nearby `quonlint.toml` or `.quonlintrc.toml` when one is found.

## Formatter

`quonfmt` is the canonical formatter for `.qn` source. It has one fixed style rather
than a user configuration file.

```sh
# Print one file's formatted source
target/release/quonfmt program.qn

# Rewrite one or more files in place
target/release/quonfmt -w src/main.qn src/library.qn

# Check formatting without writing; exits 1 when a file would change
target/release/quonfmt --check src/main.qn src/library.qn

# Read source from stdin and write formatted source to stdout
target/release/quonfmt < program.qn
```

`-w` is the short form of `--write`. Do not combine it with `--check`. A parse error
exits with status 2.

The style uses four-space indentation, a 100-column target, LF line endings, and one
blank line between declarations. See the
[complete formatter style notes](https://github.com/arniber21/quon/blob/main/docs/quonfmt-style.md).

:::caution
Version 1 of `quonfmt` strips line and block comments. Review commented source before
using `-w`.
:::

## Linter

`quonlint` checks experiment-quality concerns that are valid Quon but may be
surprising or expensive. Its rule families cover:

- `depth/`: circuit-depth growth and depth annotations;
- `gates/`: gate-set assumptions and optimization opportunities;
- `ancilla/`: borrowed ancilla lifetime and cleanup; and
- `monad/`: suspicious `run` blocks, circuit binds, and measurements.

List the exact rules and their default severities with:

```sh
target/release/quonlint --list-rules
```

Lint explicit files or let project mode discover files:

```sh
target/release/quonlint program.qn
target/release/quonlint --project
target/release/quonlint --config path/to/config.toml --fail-on warn src/
target/release/quonlint --format json program.qn
```

Project mode looks for `quonlint.toml` or `.quonlintrc.toml` in the project root.
For explicit paths, configuration is discovered by walking up from the first path.
You can also pass any filename with `--config`.

A configuration file has two sections:

```toml
[quonlint]
level = "info"       # minimum severity to report
fail_on = "error"    # configuration value; see the CLI note below
include = ["src/**/*.qn"]
exclude = ["vendor/**"]

[rules]
"gates/swap-in-source" = "allow"
"monad/circuit-bind-without-apply" = "error"
```

Severities are `allow`, `info`, `warn`, and `error`. Command-line options can adjust
the failure threshold, select rules with `--only`, exclude rules with `--except`,
and emit `human`, `json`, or `github` output. The current CLI defaults
`--fail-on` to `error`, overriding `fail_on` from the file; pass `--fail-on warn`
explicitly when warnings must fail the command.

### Match the repository's CI checks

The repository keeps a fixed 16-file corpus in
[`test/tooling/ci-corpus.txt`](https://github.com/arniber21/quon/blob/main/test/tooling/ci-corpus.txt).
Run the formatter check, linter threshold, and LSP smoke tests exactly as CI does:

```sh
just ci-tooling
```

CI passes `.quonlint.toml` explicitly and fails on `error` diagnostics. For a broader
local sweep over the repository's `.qn` fixtures, run:

```sh
just tooling-full
```

## Editor integration status

- **VS Code:** first-party extension at [`extensions/vscode-quon/`](https://github.com/arniber21/quon/tree/main/extensions/vscode-quon)
  ([#131](https://github.com/arniber21/quon/issues/131)). Install from a built `.vsix` or run
  the Extension Development Host. It starts `quon_lsp` over stdio and formats via `quonfmt`
  (format-on-save default **off** — `quonfmt` strips comments). If you get colors but no
  hover/squiggles, or `quon.showServerStatus` is "not found", rebuild/reinstall the `.vsix`
  (production `node_modules` must be packaged) and ensure `quon_lsp` is on `PATH` or set via
  `quon.lsp.path`.
- **Neovim:** first-party Lua module at [`nvim-quon/`](https://github.com/arniber21/quon/tree/main/nvim-quon)
  ([#133](https://github.com/arniber21/quon/issues/133)). Load via lazy.nvim (`ft = "quon"`) or
  packer from a monorepo checkout; see the [nvim-quon README](https://github.com/arniber21/quon/blob/main/nvim-quon/README.md).
  Uses `vim.lsp.config` / `vim.lsp.enable` with catalog entry `lsp/quon_lsp.lua`, shared
  Tree-sitter from `tree-sitter-quon/`, and conform.nvim → `quonfmt`.
- **Shared Tree-sitter grammar:** [`tree-sitter-quon/`](https://github.com/arniber21/quon/tree/main/tree-sitter-quon)
  (corpus at `tree-sitter-quon/test/corpus/`) for Zed/Neovim consumers.
- **Zed:** dev extension at [`extensions/zed-quon/`](https://github.com/arniber21/quon/tree/main/extensions/zed-quon)
  ([#132](https://github.com/arniber21/quon/issues/132)). Install via Zed → Extensions →
  Install Dev Extension; see the [zed-quon README](https://github.com/arniber21/quon/blob/main/extensions/zed-quon/README.md).
  Uses shared Tree-sitter from `tree-sitter-quon/`, `quon_lsp` over stdio, and
  format-on-save via external `quonfmt`.

You can also configure the stdio language server command above and wire
`quonfmt --check` or `quonfmt -w` and `quonlint` into editor tasks or save hooks without
an extension.
