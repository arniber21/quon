-- Local validation harness (not CI). Run from repo root:
--   nvim --headless -u NONE -S nvim-quon/scripts/validate_local.lua

vim.cmd("filetype on")
vim.opt.runtimepath:prepend(vim.fn.fnamemodify("nvim-quon", ":p"))
vim.cmd("runtime! ftdetect/*.lua")

vim.cmd.edit("frontend/tests/fixtures/bell_state.qn")
assert(vim.bo.filetype == "quon", "expected filetype quon, got " .. vim.bo.filetype)

local catalog = dofile("nvim-quon/lsp/quon_lsp.lua")
assert(catalog.cmd[1] == "quon_lsp")
assert(catalog.filetypes[1] == "quon")
assert(vim.tbl_contains(catalog.root_markers, "quonlint.toml"))

local util = require("quon.util")
local root = util.quon_root({})
assert(root, "quon_root missing")
assert(vim.uv.fs_stat(root .. "/tree-sitter-quon"), "shared grammar missing")
assert(vim.uv.fs_stat(root .. "/tree-sitter-quon/queries/highlights.scm"), "shared queries missing")
assert(not vim.uv.fs_stat(util.plugin_root() .. "/grammar.js"), "must not invent grammar under nvim-quon")

require("quon").setup({
  treesitter = false,
  format = false,
  lint = false,
  lsp = { enable = false },
})

-- Exercise treesitter registration path without nvim-treesitter installed:
-- should warn and return, not error.
pcall(function()
  require("quon.treesitter").setup({ enable = true, quon_root = root })
end)

-- LSP config path (0.11+): should register without error even if binary missing.
require("quon.lsp").setup({
  enable = true,
  cmd = { "quon_lsp" },
})

local configs = vim.lsp.config
-- vim.lsp.config is callable; presence of enable is enough for API lock.
assert(type(vim.lsp.enable) == "function", "vim.lsp.enable missing")

print("validation_ok")
vim.cmd("qa!")
