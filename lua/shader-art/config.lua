local M = {}

M.defaults = {
  --- Path to .art shader file. nil = use bundled default.
  shader = nil,
  --- Output mode: "auto" | "kitty" | "sixel" | "ascii"
  mode = "auto",
  --- Target FPS
  fps = 15,
  --- Render width in terminal columns (ASCII) or pixels (kitty/sixel)
  width = nil, -- auto-detect from alpha layout
  --- Render height in terminal rows (each row = 2 pixels for ASCII)
  height = nil, -- auto-detect from alpha layout
}

M.config = vim.deepcopy(M.defaults)

function M.setup(opts)
  M.config = vim.tbl_deep_extend("force", vim.deepcopy(M.defaults), opts or {})
end

return M
