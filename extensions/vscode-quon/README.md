# Quon for VS Code

First-party Quon language support: TextMate syntax highlighting, `quon_lsp` client, and Format Document via `quonfmt`.

Extension id: `quon.quon-vscode` · License: **Apache-2.0**

## Prerequisites

Build the tooling binaries from the Quon repo root (requires LLVM 22 / MLIR / Z3 like the rest of the workspace):

```sh
cargo build --release -p quon_lsp -p quonfmt -p quonlint-cli
```

Or point the extension at existing binaries:

| Setting / env | Purpose |
| ------------- | ------- |
| `quon.lsp.path` / `QUON_LSP_PATH` | `quon_lsp` executable |
| `quon.fmt.path` / `QUON_FMT_PATH` | `quonfmt` executable |

Discovery order (symmetric for both tools): setting → env → `PATH` → `target/release` → `target/debug`.

## Install

### From `.vsix`

```sh
cd extensions/vscode-quon
npm ci
npm run package
code --install-extension dist/quon-vscode.vsix
```

### Extension Development Host (F5)

From the **monorepo root** (recommended):

```sh
cd extensions/vscode-quon && npm ci && npm run compile
```

Then press F5 / run **Run Quon Extension** — the root [`.vscode/launch.json`](../../.vscode/launch.json) points `--extensionDevelopmentPath` at this package and opens the repo as the workspace.

A matching config also lives under `extensions/vscode-quon/.vscode/launch.json` if you open this folder alone as the workspace.

## Quick check

1. Open `frontend/tests/fixtures/bell_state.qn`.
2. Confirm TextMate colors and (after the server starts) no error diagnostics.
3. Hover `bell_state` / `CNOT` / `measure`.
4. Format Document (explicit command) — do **not** rely on format-on-save.
5. Run **Quon: Show Server Status** — it should report `running` and a path to `quon_lsp`.

## Troubleshooting: colors work, but no hover / squiggles

TextMate highlighting is static and does **not** require the extension to activate.
Hover, diagnostics, and the Quon commands require `activate()` to load and
`quon_lsp` to start.

| Symptom | Likely cause | Fix |
| ------- | ------------ | --- |
| `command 'quon.showServerStatus' not found` | Extension JS failed to load (often a `.vsix` built without `vscode-languageclient`) | Rebuild with `npm ci && npm run package` from this directory and reinstall the `.vsix`. Confirm the archive contains `node_modules/vscode-languageclient/`. |
| Status says `not running` / `(not found)` | `quon_lsp` missing from PATH / settings | `cargo build --release -p quon_lsp -p quonfmt -p quonlint-cli`, then set `quon.lsp.path` or put `target/release` on `PATH`. |
| Activation error toast | `quon_lsp` crashed on start | Check **Output → Quon Language Server** and **Extension Host**. |

Also open **Developer: Show Running Extensions** and confirm `Quon` is activated (not "Activation Failed").

## Settings

| Setting | Default | Notes |
| ------- | ------- | ----- |
| `quon.lsp.path` | `""` | Auto-discover if empty |
| `quon.lsp.debounceMs` | `100` | → `QUON_LSP_DEBOUNCE_MS` |
| `quon.lsp.logLevel` | `info` | → `QUON_LOG` / `RUST_LOG` |
| `quon.lsp.trace` | `off` | Language client trace |
| `quon.fmt.path` | `""` | Also honors `QUON_FMT_PATH` |
| `quon.fmt.formatOnSave` | **`false`** | Docs/helper only |
| `quon.lint.configPath` | `""` | Reserved; server uses upward `quonlint.toml` discovery |

**Format-on-save is OFF by default** via `contributes.configurationDefaults`:

```json
"[quon]": {
  "editor.defaultFormatter": "quon.quon-vscode",
  "editor.formatOnSave": false
}
```

### Opt in to format-on-save

Only after reading the comment-stripping warning below:

```json
"[quon]": {
  "editor.formatOnSave": true
}
```

## Caution: `quonfmt` strips comments

Version 1 of `quonfmt` removes line (`--`) and block (`{- -}`) comments. Silent format-on-save would destroy comments, which is why the default is off. Format Document remains available from the command palette.

## Shared Tree-sitter grammar

Canonical grammar for Zed (#132) and Neovim (#133): [`tree-sitter-quon/`](../../tree-sitter-quon/). Corpus: `tree-sitter-quon/test/corpus/`.

VS Code uses **TextMate** for lexical highlighting plus LSP semantic tokens. Tree-sitter is not embedded in this extension for v1.

## Commands

- **Quon: Restart Language Server**
- **Quon: Show Server Status**

## Non-goals

No embedded circuit / topology / mapper webviews. Visualization follow-ups: #134–#136.

## License

Apache License 2.0 — see [`LICENSE`](./LICENSE).
