local util = require("quon.util")

local M = {}

---@param opts table
---@return table
function M.conform_formatter(opts)
  opts = opts or {}
  return {
    command = util.resolve_bin("quonfmt", opts),
    stdin = true,
  }
end

---@param opts table
function M.setup(opts)
  opts = opts or {}
  if opts.enable == false then
    return
  end

  local ok, conform = pcall(require, "conform")
  if not ok then
    vim.notify(
      "quon: conform.nvim not found; skip formatter wiring (install stevearc/conform.nvim)",
      vim.log.levels.WARN
    )
    return
  end

  local formatter = M.conform_formatter(opts)
  conform.formatters.quonfmt = formatter
  conform.formatters_by_ft = conform.formatters_by_ft or {}
  conform.formatters_by_ft.quon = { "quonfmt" }

  if opts.format_on_save then
    -- Comment-stripping hazard: quonfmt v1 drops comments. Opt-in only.
    vim.api.nvim_create_autocmd("BufWritePre", {
      group = vim.api.nvim_create_augroup("quon_format_on_save", { clear = true }),
      pattern = "*.qn",
      callback = function(args)
        require("conform").format({ bufnr = args.buf })
      end,
    })
  end

  local user_command = opts.user_command
  if user_command == nil then
    user_command = true
  end
  if user_command then
    vim.api.nvim_create_user_command("Format", function(cmd_opts)
      require("conform").format({
        bufnr = 0,
        -- conform range end is exclusive (end line is not included).
        range = cmd_opts.range ~= 0 and {
          start = { cmd_opts.line1, 0 },
          ["end"] = { cmd_opts.line2 + 1, 0 },
        } or nil,
      })
    end, { range = true, desc = "Format with conform (quonfmt for Quon)" })
  end
end

--- Thin wrapper used by require("quon").format().
function M.format(opts)
  local ok, conform = pcall(require, "conform")
  if not ok then
    vim.notify("quon: conform.nvim is required for formatting", vim.log.levels.ERROR)
    return
  end
  conform.format(opts or { filetype = "quon" })
end

--- Documented secondary adapter data for formatter.nvim users.
---@param opts table|nil
---@return table
function M.formatter_nvim_config(opts)
  opts = opts or {}
  local cmd = util.resolve_bin("quonfmt", opts)
  return {
    filetype = {
      quon = {
        function()
          return {
            exe = cmd,
            stdin = true,
          }
        end,
      },
    },
  }
end

return M
