local M = {}

---@class quon.Opts
---@field quon_root string|nil
---@field lsp quon.LspOpts|false|nil
---@field treesitter quon.TsOpts|false|nil
---@field format quon.FormatOpts|false|nil
---@field lint quon.LintOpts|false|nil

---@class quon.LspOpts
---@field enable boolean|nil
---@field cmd string[]|nil
---@field cmd_env table<string,string>|nil
---@field root_markers string[]|nil
---@field on_attach function|nil
---@field capabilities table|nil
---@field config table|nil
---@field quon_root string|nil

---@class quon.TsOpts
---@field enable boolean|nil
---@field parser_path string|nil
---@field quon_root string|nil

---@class quon.FormatOpts
---@field enable boolean|nil
---@field format_on_save boolean|nil
---@field user_command boolean|nil
---@field command string|nil
---@field quon_root string|nil

---@class quon.LintOpts
---@field enable boolean|nil
---@field command string|nil
---@field quon_root string|nil

local function section_enabled(section, default_enable)
  if section == false then
    return false, {}
  end
  section = section or {}
  local enable = section.enable
  if enable == nil then
    enable = default_enable
  end
  return enable, section
end

--- Configure Quon Neovim integration (LSP, Tree-sitter, format, optional lint).
---@param opts quon.Opts|nil
function M.setup(opts)
  opts = opts or {}

  require("quon.filetype").setup()

  local lsp_on, lsp_opts = section_enabled(opts.lsp, true)
  if lsp_on then
    lsp_opts.quon_root = lsp_opts.quon_root or opts.quon_root
    require("quon.lsp").setup(lsp_opts)
  end

  local ts_on, ts_opts = section_enabled(opts.treesitter, true)
  if ts_on then
    ts_opts.quon_root = ts_opts.quon_root or opts.quon_root
    require("quon.treesitter").setup(ts_opts)
  end

  local fmt_on, fmt_opts = section_enabled(opts.format, true)
  if fmt_on then
    fmt_opts.quon_root = fmt_opts.quon_root or opts.quon_root
    if fmt_opts.format_on_save == nil then
      fmt_opts.format_on_save = false
    end
    require("quon.format").setup(fmt_opts)
  end

  local lint_on, lint_opts = section_enabled(opts.lint, false)
  if lint_on then
    lint_opts.quon_root = lint_opts.quon_root or opts.quon_root
    lint_opts.enable = true
    require("quon.lint").setup(lint_opts)
  end
end

function M.format(opts)
  require("quon.format").format(opts)
end

return M
