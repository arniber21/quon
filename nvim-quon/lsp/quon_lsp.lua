---@brief
---
--- Default server catalog entry for Quon (`quon_lsp`).
--- Loaded from the plugin rtp as `lsp/quon_lsp.lua` for Neovim 0.11+
--- `vim.lsp.config` / nvim-lspconfig catalog consumers.
---
--- Do not call `require("lspconfig").quon_lsp.setup(...)` — use
--- `vim.lsp.config("quon_lsp", opts)` then `vim.lsp.enable("quon_lsp")`.

---@type vim.lsp.Config
return {
  cmd = { "quon_lsp" },
  filetypes = { "quon" },
  root_markers = { "quonlint.toml", ".quonlintrc.toml", ".git" },
  settings = {},
}
