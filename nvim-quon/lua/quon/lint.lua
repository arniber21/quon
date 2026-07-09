local util = require("quon.util")

local M = {}

--- Convert a UTF-8 byte offset into (1-indexed row, 0-indexed col) for the buffer.
---@param bufnr integer
---@param byte_offset integer
---@return integer row
---@return integer col
local function byte_to_pos(bufnr, byte_offset)
  local line_count = vim.api.nvim_buf_line_count(bufnr)
  -- nvim_buf_get_offset(bufnr, line) → byte offset of start of 0-indexed line
  local lo, hi = 0, line_count
  while lo < hi do
    local mid = math.floor((lo + hi + 1) / 2)
    local off = vim.api.nvim_buf_get_offset(bufnr, mid)
    if off <= byte_offset then
      lo = mid
    else
      hi = mid - 1
    end
  end
  local row = lo
  local line_start = vim.api.nvim_buf_get_offset(bufnr, row)
  local col = math.max(0, byte_offset - line_start)
  -- Clamp to last line if offset is past EOF
  if row >= line_count then
    row = math.max(0, line_count - 1)
    local line = vim.api.nvim_buf_get_lines(bufnr, row, row + 1, false)[1] or ""
    col = #line
  end
  return row + 1, col
end

---@param severity string
---@return integer|nil
local function map_severity(severity)
  local s = string.lower(severity or "")
  if s == "error" then
    return vim.diagnostic.severity.ERROR
  elseif s == "warn" or s == "warning" then
    return vim.diagnostic.severity.WARN
  elseif s == "info" then
    return vim.diagnostic.severity.INFO
  elseif s == "allow" then
    return nil
  end
  return vim.diagnostic.severity.HINT
end

--- Best-effort nvim-lint parser for `quonlint --format json` (byte spans).
---@param output string
---@param bufnr integer
---@return table[]
function M.parse_json(output, bufnr)
  if output == nil or output == "" then
    return {}
  end
  local ok, decoded = pcall(vim.json.decode, output)
  if not ok or type(decoded) ~= "table" then
    return {}
  end
  local diags = {}
  for _, item in ipairs(decoded.diagnostics or {}) do
    local sev = map_severity(item.severity)
    if sev ~= nil and item.span then
      local srow, scol = byte_to_pos(bufnr, item.span.start or 0)
      local erow, ecol = byte_to_pos(bufnr, item.span["end"] or item.span.start or 0)
      table.insert(diags, {
        lnum = srow - 1,
        col = scol,
        end_lnum = erow - 1,
        end_col = ecol,
        severity = sev,
        message = item.message or item.rule or "quonlint",
        source = "quonlint",
        code = item.rule,
      })
    end
  end
  return diags
end

---@param opts table
function M.setup(opts)
  opts = opts or {}
  if not opts.enable then
    return
  end

  local ok, lint = pcall(require, "lint")
  if not ok then
    vim.notify(
      "quon: nvim-lint not found; skip optional lint bridge",
      vim.log.levels.WARN
    )
    return
  end

  local command = util.resolve_bin("quonlint", opts)
  lint.linters.quonlint = {
    cmd = command,
    stdin = false,
    args = { "--format", "json" },
    stream = "stdout",
    ignore_exitcode = true,
    parser = function(output, bufnr)
      return M.parse_json(output, bufnr)
    end,
  }
  lint.linters_by_ft = lint.linters_by_ft or {}
  lint.linters_by_ft.quon = { "quonlint" }

  vim.api.nvim_create_autocmd({ "BufWritePost", "InsertLeave" }, {
    group = vim.api.nvim_create_augroup("quon_lint", { clear = true }),
    pattern = "*.qn",
    callback = function()
      require("lint").try_lint()
    end,
  })
end

return M
