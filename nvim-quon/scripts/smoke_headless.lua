-- Headless LSP attach smoke for Quon.
-- Expects smoke_minimal_init.lua (or equivalent) already loaded and quon_lsp on PATH.
--
-- Assertions:
-- 1. Open bell_state.qn
-- 2. Wait until client name quon_lsp attaches (≤ 10s)
-- 3. Optional hover/definition request on a known position
-- 4. print("OK") or cquit(1)

local fixture = "frontend/tests/fixtures/bell_state.qn"
vim.cmd.edit(fixture)

if vim.bo.filetype ~= "quon" then
  io.stderr:write(
    string.format("FAIL: expected filetype quon, got %s\n", vim.bo.filetype)
  )
  vim.cmd.cquit(1)
end

local deadline = vim.uv.hrtime() + 10 * 1e9
local clients = {}
while vim.uv.hrtime() < deadline do
  clients = vim.lsp.get_clients({ name = "quon_lsp", bufnr = 0 })
  if #clients > 0 then
    break
  end
  vim.wait(100)
end

if #clients == 0 then
  io.stderr:write("FAIL: quon_lsp did not attach within 10s\n")
  vim.cmd.cquit(1)
end

-- Best-effort hover near start of file (server may still be warming analysis).
local ok_req, results = pcall(function()
  return vim.lsp.buf_request_sync(0, "textDocument/hover", {
    textDocument = { uri = vim.uri_from_bufnr(0) },
    position = { line = 0, character = 0 },
  }, 3000)
end)

if not ok_req then
  io.stderr:write("WARN: hover request errored (attach still OK): " .. tostring(results) .. "\n")
end

print("OK")
