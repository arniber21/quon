# nvim-quon

First-party Neovim integration for [Quon](https://github.com/arniber21/quon): filetype
`quon` for `*.qn`, `quon_lsp` over stdio, shared Tree-sitter grammar, and `quonfmt` via
conform.nvim.

| Piece | Source |
|-------|--------|
| LSP | `quon_lsp` (stdio); catalog at `lsp/quon_lsp.lua` |
| Highlights | Shared [`tree-sitter-quon/`](../tree-sitter-quon/) (#131) — **consume only** |
| Format | `quonfmt` stdin via [conform.nvim](https://github.com/stevearc/conform.nvim) |
| Lint | Prefer LSP diagnostics; optional nvim-lint bridge (off by default) |

**Filetype / languageId:** extension `.qn` → filetype **`quon`** → LSP languageId **`quon`**.
Never use filetype `"qn"`.

## Prerequisites

```sh
cargo build --release -p quon_lsp -p quonfmt -p quonlint-cli
export PATH="$PWD/target/release:$PATH"
```

`quon_lsp` needs the same LLVM 22 + Z3 environment as the Quon workspace. `quonfmt` alone
is MLIR-free. Neovim **0.11+** is required (`vim.lsp.config` / `vim.lsp.enable`).

## Install (lazy.nvim)

```lua
{
  dir = "/path/to/quon/nvim-quon",  -- monorepo path (supported workflow)
  name = "nvim-quon",
  ft = "quon",  -- filetype name is "quon" (extension .qn) — NOT ft = "qn"
  dependencies = {
    -- "neovim/nvim-lspconfig",    -- optional; catalog loads lsp/*.lua when present
    "nvim-treesitter/nvim-treesitter",
    "stevearc/conform.nvim",
    -- "mfussenegger/nvim-lint",   -- optional
  },
  opts = {
    lsp = {
      -- cmd = { vim.fn.expand("~/projects/quon/target/release/quon_lsp") },
      cmd_env = { QUON_LSP_DEBOUNCE_MS = "100" },
    },
    treesitter = { enable = true },
    format = { enable = true, format_on_save = false },
    lint = { enable = false },
  },
  config = function(_, opts)
    require("quon").setup(opts)
  end,
}
```

After setup, install the parser once:

```vim
:TSInstall quon
```

## Install (packer.nvim)

```lua
use {
  "/path/to/quon/nvim-quon",
  requires = { "nvim-treesitter/nvim-treesitter", "stevearc/conform.nvim" },
  config = function()
    require("quon").setup({})
  end,
}
```

## LSP

Registration uses **only**:

```lua
vim.lsp.config("quon_lsp", opts)
vim.lsp.enable("quon_lsp")
```

Do **not** call `require("lspconfig").quon_lsp.setup(...)`.

Defaults (also shipped as `lsp/quon_lsp.lua` on the plugin rtp):

- `cmd = { "quon_lsp" }` (PATH, then `target/release` / `target/debug` under the monorepo)
- `filetypes = { "quon" }`
- `root_markers = { "quonlint.toml", ".quonlintrc.toml", ".git" }`

Suggested (optional) keymaps in `opts.lsp.on_attach`: `gd`, `K`, `gra`. This plugin does not
force global maps.

Verify attachment with `:checkhealth vim.lsp`, `:checkhealth quon`, or
`vim.lsp.get_clients({ name = "quon_lsp" })`. On older builds, `:LspInfo` may still exist.

## Tree-sitter

Canonical grammar + queries live in [`tree-sitter-quon/`](../tree-sitter-quon/) (owned by
#131). This plugin registers the parser via `User TSUpdate` and
`install_info.path` pointing at that package (shared `queries/`).

There is **no** Neovim-private `grammar.js`. A Neovim-only query overlay under
`nvim-quon/queries/quon/` is allowed only for editor-specific captures; none ships in v1 —
prefer fixing shared queries upstream.

| Situation | `install_info` |
|-----------|----------------|
| `path` = abs dir of `tree-sitter-quon/` | `path = "…/tree-sitter-quon"` (default) |
| Override | `opts.treesitter.parser_path` |

## Format

```lua
require("quon").setup({
  format = {
    enable = true,
    format_on_save = false, -- default; quonfmt v1 strips comments
    user_command = true,    -- creates :Format
  },
})
```

Or call `require("quon").format()`. Check `:ConformInfo`.

**Caution:** `quonfmt` strips line and block comments. Keep format-on-save off unless you
accept that. See [`docs/quonfmt-style.md`](../docs/quonfmt-style.md).

Secondary (formatter.nvim) data: `require("quon.format").formatter_nvim_config()`.

## Optional nvim-lint

LSP already publishes `quonlint` diagnostics. Standalone nvim-lint duplicates noise:

```lua
require("quon").setup({ lint = { enable = true } })
```

The bridge is best-effort (JSON byte spans → buffer positions). Prefer LSP diagnostics.

## Health

```vim
:checkhealth quon
```

## Troubleshooting

| Symptom | Check |
|---------|-------|
| LSP not attaching | `:checkhealth quon`; filetype is `quon` not `qn`; lazy `ft = "quon"` |
| `cmd` not found | PATH / `opts.lsp.cmd` / build `target/release/quon_lsp` |
| No highlights | `:TSInstall quon`; `opts.treesitter.parser_path`; `:InspectTree` |
| Format no-op | conform installed; `quonfmt` on PATH; parse error exit 2 |
| Duplicate lint | keep `opts.lint.enable = false` |

## Layout

```
nvim-quon/
├── README.md
├── lua/quon/          # setup, lsp, treesitter, format, lint, health
├── lsp/quon_lsp.lua   # vim.lsp catalog entry
├── ftdetect/quon.lua
├── ftplugin/quon.lua
└── scripts/           # optional headless smoke
```
