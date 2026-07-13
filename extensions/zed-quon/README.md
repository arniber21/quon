# Quon for Zed

In-repo Zed language extension: Tree-sitter highlighting (shared `tree-sitter-quon`), `quon_lsp` over stdio, and format-on-save via external `quonfmt`.

Extension id is `quon` (registry-safe). Directory name `extensions/zed-quon/` is for humans only.

## Build tools

From the monorepo root:

```bash
cargo build -p quon_lsp --release
cargo build -p quonfmt --release
```

Add `target/release` to your `PATH` so Zed can find both binaries (or set absolute paths in settings — see below).

## Install Dev Extension

1. Install a Rust toolchain via [rustup](https://rustup.rs/) (Zed compiles the WASM extension on install).
2. Zed → **Extensions** → **Install Dev Extension**.
3. Select this directory: `extensions/zed-quon`.

Open a smoke fixture: `frontend/tests/fixtures/bell_state.qn`.

## Grammar pin

`extension.toml` loads the shared grammar with a portable monorepo pin:

```toml
[grammars.quon]
repository = "https://github.com/arniber21/quon"
rev = "<commit that contains tree-sitter-quon/>"
path = "tree-sitter-quon"
```

Do **not** use absolute `file://…` paths or `rev = "local"`. After grammar updates on `#131` / `main`, bump `rev` to a commit that contains the matching `tree-sitter-quon/` (and keep `languages/quon/*.scm` in sync with `tree-sitter-quon/queries/`).

## LSP discovery

`quon_lsp` is resolved in this order:

1. **Settings** — `lsp.quon-lsp.binary.path` (and optional `arguments` / `env`)
2. **PATH** — `worktree.which("quon_lsp")`
3. **Worktree targets** — only if the opened folder is a Quon checkout (`Cargo.toml` workspace members include `quon_lsp` and `frontend`, plus a marker such as `frontend/src/lib.rs`, `SPEC.md`, or `tree-sitter-quon/`):
   - `{root}/target/release/quon_lsp`
   - `{root}/target/debug/quon_lsp`
   Existence is probed via Zed’s **host** `process` API (`test -x` / Windows `if exist`), not guest `Path::is_file` — Zed’s WASM sandbox only preopens the extension work dir, so guest FS checks cannot see monorepo `target/`.
4. **Clear error** — build instructions + settings JSON snippet

Override example (also in `settings.example.json`):

```json
{
  "lsp": {
    "quon-lsp": {
      "binary": {
        "path": "/ABS/PATH/TO/quon/target/release/quon_lsp",
        "env": { "RUST_LOG": "quon_lsp=debug" }
      }
    }
  }
}
```

Empty `binary.path` in the example means omit the key (or leave blank) so discovery runs. Set an absolute path only when you need a fixed binary.

## Format on save

Zed does not register external formatters in `extension.toml`. Committed settings deliver format-on-save:

| File | Role |
| ---- | ---- |
| [`settings.example.json`](settings.example.json) | Copy into user/project settings |
| [`.zed/settings.json`](../../.zed/settings.json) | Project settings for this monorepo |

Both enable Quon `format_on_save` with external `quonfmt` and empty `arguments` (stdin → stdout). `quonfmt` must be on `PATH` (or use an absolute `command`).

**Note:** `quonfmt` strips comments in v1 — see [`quonfmt/README.md`](../../quonfmt/README.md) and [`docs/quonfmt-style.md`](../../docs/quonfmt-style.md).

### Manual verification

1. Put `quonfmt` on `PATH`.
2. Install Dev Extension; open `frontend/tests/fixtures/bell_state.qn`.
3. Confirm `.zed/settings.json` (or merged settings from `settings.example.json`) has Quon format-on-save.
4. Introduce whitespace drift; save (or Format Document).
5. Buffer should match `cargo run -p quonfmt -- frontend/tests/fixtures/bell_state.qn` / `--check` clean.

## Semantic tokens

Zed defaults semantic tokens to off. For richer Quon highlighting from `quon_lsp`, set `"semantic_tokens": "combined"` under `languages.Quon` (already in the committed settings files).

## Troubleshoot

| Symptom | What to try |
| ------- | ----------- |
| No LSP / binary error | Build `quon_lsp`, fix `PATH`, or set `lsp.quon-lsp.binary.path` |
| `failed to load language Quon` / `Invalid node type "{"` | Grammar pin is stale or still uses a lumped `delimiter` token. Pull `main`, bump `[grammars.quon].rev` to a commit that has anonymous `"{"`/`"}"` tokens + matching `brackets.scm`, then reinstall the Dev Extension |
| Stale / missing highlights | Bump `[grammars.quon].rev`; reinstall the Dev Extension |
| Published extension overrides Dev | Disable the registry copy while developing |
| Logs | Command palette: `zed: open log`, or run `zed --foreground` |
| LSP debug | `RUST_LOG=quon_lsp=debug` via settings `binary.env` or shell |

Confirm the language server is attached in Zed’s language server UI for the Quon buffer.

## Registry publish

Not in scope for v1. When publishing, keep `id = "quon"` (must not contain `zed` / `extension` substrings) and the same `repository` + `rev` + `path = "tree-sitter-quon"` grammar pin. License: Apache-2.0 (`LICENSE`).
