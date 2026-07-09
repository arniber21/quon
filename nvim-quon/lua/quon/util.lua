local M = {}

---@param path string|nil
---@return boolean
local function is_executable(path)
  return type(path) == "string" and path ~= "" and vim.fn.executable(path) == 1
end

--- Absolute path of the nvim-quon plugin root (directory containing lua/).
---@return string
function M.plugin_root()
  local source = debug.getinfo(1, "S").source
  if source:sub(1, 1) == "@" then
    source = source:sub(2)
  end
  -- .../nvim-quon/lua/quon/util.lua → nvim-quon (no trailing slash)
  return vim.fs.normalize(vim.fs.dirname(vim.fs.dirname(vim.fs.dirname(source))))
end

--- Quon monorepo root (parent of nvim-quon), or nil if not in a checkout.
---@param opts { quon_root?: string }|nil
---@return string|nil
function M.quon_root(opts)
  opts = opts or {}
  if opts.quon_root and opts.quon_root ~= "" then
    return vim.fs.normalize(opts.quon_root)
  end
  local parent = vim.fs.dirname(M.plugin_root())
  if vim.uv.fs_stat(parent .. "/tree-sitter-quon") or vim.uv.fs_stat(parent .. "/Cargo.toml") then
    return parent
  end
  return nil
end

--- Resolve quon_lsp command argv.
---@param opts { cmd?: string[], quon_root?: string }|nil
---@return string[]
function M.resolve_lsp_cmd(opts)
  opts = opts or {}
  if opts.cmd and #opts.cmd > 0 then
    return opts.cmd
  end

  local on_path = vim.fn.exepath("quon_lsp")
  if on_path ~= "" then
    return { on_path }
  end

  local root = M.quon_root(opts)
  if root then
    for _, rel in ipairs({ "target/release/quon_lsp", "target/debug/quon_lsp" }) do
      local candidate = root .. "/" .. rel
      if is_executable(candidate) then
        return { candidate }
      end
    end
  end

  return { "quon_lsp" }
end

--- Resolve a CLI binary (quonfmt / quonlint), with optional monorepo fallback.
---@param name string
---@param opts { command?: string, quon_root?: string }|nil
---@return string
function M.resolve_bin(name, opts)
  opts = opts or {}
  if opts.command and opts.command ~= "" then
    return opts.command
  end

  local on_path = vim.fn.exepath(name)
  if on_path ~= "" then
    return on_path
  end

  local root = M.quon_root(opts)
  if root then
    local release_name = name
    if name == "quonlint" then
      -- Cargo package is quonlint-cli; binary is still quonlint.
      release_name = "quonlint"
    end
    for _, rel in ipairs({
      "target/release/" .. release_name,
      "target/debug/" .. release_name,
    }) do
      local candidate = root .. "/" .. rel
      if is_executable(candidate) then
        return candidate
      end
    end
  end

  return name
end

return M
