local util = require("quon.util")

local M = {}

local function report_bin(name, opts)
  local path = util.resolve_bin(name, opts)
  if vim.fn.executable(path) == 1 then
    vim.health.ok(name .. ": " .. path)
  else
    vim.health.warn(name .. " not found (looked for `" .. path .. "` on PATH / target/{release,debug})")
  end
end

function M.check()
  vim.health.start("quon")

  local ver = vim.version()
  if vim.fn.has("nvim-0.11") == 1 then
    vim.health.ok(
      string.format("Neovim %d.%d.%d (vim.lsp.config / enable supported)", ver.major, ver.minor, ver.patch)
    )
  else
    vim.health.warn(
      string.format(
        "Neovim %d.%d.%d — 0.11+ recommended for vim.lsp.config / vim.lsp.enable",
        ver.major,
        ver.minor,
        ver.patch
      )
    )
  end

  local plugin = util.plugin_root()
  local catalog = plugin .. "/lsp/quon_lsp.lua"
  if vim.uv.fs_stat(catalog) then
    vim.health.ok("lsp/quon_lsp.lua present: " .. catalog)
  else
    vim.health.error("lsp/quon_lsp.lua missing from plugin rtp")
  end

  local quon_root = util.quon_root({})
  if quon_root then
    vim.health.ok("quon monorepo root: " .. quon_root)
    local grammar = quon_root .. "/tree-sitter-quon"
    if vim.uv.fs_stat(grammar) then
      vim.health.ok("shared grammar: " .. grammar)
    else
      vim.health.warn("tree-sitter-quon missing under monorepo root")
    end
  else
    vim.health.info("not running from a Quon monorepo checkout (set opts.quon_root if needed)")
  end

  report_bin("quon_lsp", {})
  report_bin("quonfmt", {})
  report_bin("quonlint", {})

  local ts_ok = pcall(require, "nvim-treesitter")
  if ts_ok then
    vim.health.ok("nvim-treesitter available")
  else
    vim.health.warn("nvim-treesitter not installed (highlights via Tree-sitter disabled)")
  end

  local conform_ok = pcall(require, "conform")
  if conform_ok then
    vim.health.ok("conform.nvim available")
  else
    vim.health.warn("conform.nvim not installed (:Format / format-on-save unavailable)")
  end

  local lint_ok = pcall(require, "lint")
  if lint_ok then
    vim.health.info("nvim-lint available (optional; default off — prefer LSP diagnostics)")
  else
    vim.health.info("nvim-lint not installed (optional)")
  end
end

return M
