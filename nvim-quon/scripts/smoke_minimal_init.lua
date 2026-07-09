-- Minimal init for headless Neovim smoke (optional CI / local).
-- Usage (from repo root, with quon_lsp on PATH):
--   nvim --headless -u nvim-quon/scripts/smoke_minimal_init.lua \
--     -c "luafile nvim-quon/scripts/smoke_headless.lua" -c qa

vim.cmd("filetype on")
vim.opt.runtimepath:prepend(vim.fn.fnamemodify("nvim-quon", ":p"))
vim.cmd("runtime! ftdetect/*.lua")

require("quon").setup({
  treesitter = { enable = false }, -- LSP-only smoke; skip TS compile in CI
  format = { enable = false },
  lint = { enable = false },
  lsp = {
    enable = true,
  },
})
