-- Quon filetype defaults aligned with docs/quonfmt-style.md (4-space, 100-col, LF).
vim.bo.commentstring = "-- %s"
vim.bo.comments = "s:{-,e:-},:--"
vim.bo.expandtab = true
vim.bo.shiftwidth = 4
vim.bo.tabstop = 4
vim.bo.softtabstop = 4
vim.bo.textwidth = 100

-- Prefer conform / :Format (stdin quonfmt). Do not shell quonfmt -w on the buffer path.
vim.bo.formatexpr = ""
