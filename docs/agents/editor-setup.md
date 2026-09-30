# Editor setup (agents & contributors)

How Quon wires into editors. Binaries are editor-agnostic; packages live in-repo.

## Shared prerequisites

```sh
cargo build --release -p quon_lsp -p quonfmt -p quonlint-cli
export PATH="$PWD/target/release:$PATH"
```

- Language ID / filetype: **`quon`** for `*.qn` (never `"qn"`).
- LSP: stdio `quon_lsp`.
- Format: `quonfmt` stdin → stdout (`-w` / `--check` for CLI).
- Lint: merged into LSP; CLI `quonlint --format json` for offline tools.
- Tree-sitter: single package [`tree-sitter-quon/`](../../tree-sitter-quon/) owned by #131.
  Do **not** invent a second grammar for Zed/Neovim.
- Highlight vocabulary: [`tree-sitter-quon/highlight-lexicon.json`](../../tree-sitter-quon/highlight-lexicon.json)
  (#205). `grammar.js` reads it. TextMate keyword/operator patterns, Zed copies of
  `queries/{highlights,brackets,indents}.scm`, and `prelude_names::keywords()` are
  checked copies — `just ci-editor-grammar` fails if they drift. The compiler lexer
  is not generated from the lexicon.
- JS packages: `tree-sitter-quon/` and `extensions/vscode-quon/` use **npm**
  (`package-lock.json`, `npm ci`). `website/` stays **pnpm**. Do not add a second
  lockfile under the editor or grammar packages.

Public user docs: [Developer tooling](../../website/src/content/docs/guides/tooling.md).

---

## Neovim (`nvim-quon/`) — #133

In-repo Lua module. Primary install path: lazy.nvim `dir =` pointing at `nvim-quon/`.

### lazy.nvim

```lua
{
  dir = "/path/to/quon/nvim-quon",
  name = "nvim-quon",
  ft = "quon",  -- NOT "qn"
  dependencies = {
    "nvim-treesitter/nvim-treesitter",
    "stevearc/conform.nvim",
  },
  opts = {
    format = { format_on_save = false },
    lint = { enable = false },
  },
  config = function(_, opts)
    require("quon").setup(opts)
  end,
}
```

### packer.nvim

```lua
use {
  "/path/to/quon/nvim-quon",
  requires = { "nvim-treesitter/nvim-treesitter", "stevearc/conform.nvim" },
  config = function()
    require("quon").setup({})
  end,
}
```

### LSP registration (Neovim 0.11+)

Ship `nvim-quon/lsp/quon_lsp.lua`. Runtime wiring is **only**:

```lua
vim.lsp.config("quon_lsp", merged_opts)
vim.lsp.enable("quon_lsp")
```

Never call `require("lspconfig").quon_lsp.setup(...)`.

### Tree-sitter

Consume [`tree-sitter-quon/`](../../tree-sitter-quon/) via `User TSUpdate` +
`install_info.path` (absolute path to the grammar package). Shared queries under
`tree-sitter-quon/queries/`. No Neovim-private `grammar.js`.

```vim
:TSInstall quon
:checkhealth quon
```

### Format / lint

- conform.nvim → `quonfmt` stdin; `:Format` when `format.user_command` is true (default).
- format-on-save default **off** (`quonfmt` strips comments).
- nvim-lint optional, default **off** (LSP already publishes lint).

Full consumer docs: [`nvim-quon/README.md`](../../nvim-quon/README.md).

Optional headless smoke: `nvim-quon/scripts/smoke_headless.lua` (see script comments).

---

## VS Code (`extensions/vscode-quon/`) — #131

First-party extension: TextMate + `quon_lsp` + `quonfmt`. Owns the shared Tree-sitter
grammar package for other editors (VS Code itself uses TextMate for lexical highlight).

See [`extensions/vscode-quon/README.md`](../../extensions/vscode-quon/README.md).

**Syntax-only failure mode:** colors without hover/squiggles usually means
`activate()` never loaded (packaged `.vsix` missing `vscode-languageclient`) or
`quon_lsp` was not found. Do **not** exclude all of `node_modules/` in
`.vscodeignore` — production deps must ship in the `.vsix`. Use
**Quon: Show Server Status** and Output → Extension Host to confirm.

---

## Zed (`extensions/zed-quon/`) — #132

Dev extension: Tree-sitter + `quon_lsp` + `quonfmt`. Consumes the same
`tree-sitter-quon/` package (no forked `grammar.js`).

Sync highlight queries and pin the grammar in one command (after the grammar
commit exists on GitHub — Zed clones `rev` from the remote):

```sh
scripts/bump-zed-grammar-rev.sh [REV]
```

`REV` defaults to `HEAD` and must be a 40-character SHA. The command rewrites
`languages/quon/{highlights,brackets,indents}.scm` from
`tree-sitter-quon/queries/` and sets `[grammars.quon].rev`. `locals.scm` stays
in the grammar package (Neovim loads it from there). `outline.scm` is Zed-only.
`extensions/zed-quon/grammars/` and `*.wasm` are local build products; do not
commit them.

See [`extensions/zed-quon/README.md`](../../extensions/zed-quon/README.md).

**Brackets query:** `languages/quon/brackets.scm` matches anonymous `"{"` /
`"}"` tokens. Never reintroduce a named `delimiter` token in `grammar.js` —
Zed fails language load with `Invalid node type "{"`. After grammar changes,
bump `[grammars.quon].rev` in `extension.toml` and reinstall the Dev Extension.

---

## Workspace namespace (LSP, #177)

Quon has no `import` syntax. `quon_lsp` still answers cross-file go-to-definition,
find-references, and rename for top-level `fn` and `type` names:

- The server records workspace roots from `initialize` (`workspaceFolders`, or
  `rootUri` when folders are absent) and indexes on-disk `*.qn` files under
  those roots. Open buffers override disk text until the buffer closes.
- Files in the **same directory** share one namespace. A subdirectory is a
  different namespace. This is an interim convention, not a module system.
- `didSave` and a `**/*.qn` file watcher refresh the index. Open buffers are
  not replaced by a disk event.
- Several definitions of the same name in one directory are all returned.
  Rename refuses a new name that is already a top-level `fn` or `type` in that
  directory. Locals that shadow a name stay in-file and are not rewritten.
- Directories named `target`, `node_modules`, `.git`, `dist`, `.venv`,
  `grammars`, and any other dot-directory, are not indexed.

## Agent checklist

| Do | Don't |
|----|-------|
| Stack editor PRs under #131 when grammar is not on `main` | Author a second `grammar.js` in #132/#133 |
| Use filetype / languageId `quon` | Use filetype `qn` |
| Prefer `vim.lsp.config` / `enable` for Neovim | Dual-path `lspconfig.*.setup` + config |
| Document format-on-save comment hazard | Enable format-on-save by default |
