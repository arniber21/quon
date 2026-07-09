-- Filetype helpers (ftdetect/ftplugin cover the common path; this module is for setup()).
local M = {}

function M.setup()
  vim.filetype.add({
    extension = {
      qn = "quon",
    },
  })
end

return M
