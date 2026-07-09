local util = require("quon.util")

local M = {}

---@param opts table
function M.setup(opts)
  opts = opts or {}
  if opts.enable == false then
    return
  end

  if vim.fn.has("nvim-0.11") ~= 1 then
    vim.notify(
      "quon: Neovim 0.11+ required for vim.lsp.config / vim.lsp.enable (got "
        .. tostring(vim.version())
        .. ")",
      vim.log.levels.WARN
    )
    return
  end

  local defaults = {
    cmd = util.resolve_lsp_cmd(opts),
    filetypes = { "quon" },
    root_markers = opts.root_markers
      or { "quonlint.toml", ".quonlintrc.toml", ".git" },
    settings = {},
  }

  if opts.cmd_env then
    defaults.cmd_env = opts.cmd_env
  end
  if opts.capabilities then
    defaults.capabilities = opts.capabilities
  end
  if opts.on_attach then
    defaults.on_attach = opts.on_attach
  end

  -- Merge user lsp opts over defaults (user wins for overlapping keys).
  local merged = vim.tbl_deep_extend("force", defaults, opts.config or {})
  if opts.cmd then
    merged.cmd = opts.cmd
  else
    merged.cmd = defaults.cmd
  end
  if opts.root_markers then
    merged.root_markers = opts.root_markers
  end

  -- Catalog entry on rtp: nvim-quon/lsp/quon_lsp.lua
  -- Registration path: vim.lsp.config + enable ONLY (no lspconfig .setup()).
  vim.lsp.config("quon_lsp", merged)
  vim.lsp.enable("quon_lsp")
end

return M
