local util = require("quon.util")

local M = {}

---@param opts table
function M.setup(opts)
  opts = opts or {}
  if opts.enable == false then
    return
  end

  local ok, _ = pcall(require, "nvim-treesitter")
  if not ok then
    vim.notify(
      "quon: nvim-treesitter not found; skip Tree-sitter registration",
      vim.log.levels.WARN
    )
    return
  end

  local quon_root = util.quon_root(opts)
  local grammar_path = opts.parser_path
  if not grammar_path or grammar_path == "" then
    if quon_root then
      grammar_path = quon_root .. "/tree-sitter-quon"
    else
      vim.notify(
        "quon: cannot locate tree-sitter-quon; set opts.treesitter.parser_path or opts.quon_root",
        vim.log.levels.WARN
      )
      return
    end
  end
  grammar_path = vim.fs.normalize(grammar_path)

  if not vim.uv.fs_stat(grammar_path) then
    vim.notify(
      "quon: tree-sitter-quon not found at " .. grammar_path,
      vim.log.levels.WARN
    )
    return
  end

  -- Modern nvim-treesitter: User TSUpdate + install_info.path / location.
  -- Do NOT use deprecated get_parser_configs().
  vim.api.nvim_create_autocmd("User", {
    pattern = "TSUpdate",
    group = vim.api.nvim_create_augroup("quon_treesitter", { clear = true }),
    callback = function()
      local parsers = require("nvim-treesitter.parsers")
      -- path points at the tree-sitter-quon package itself (no location needed).
      parsers.quon = {
        install_info = {
          path = grammar_path,
          queries = "queries",
          -- src/parser.c is committed by #131; no generate at install time.
          generate = false,
        },
        tier = 2,
      }
    end,
  })
end

return M
