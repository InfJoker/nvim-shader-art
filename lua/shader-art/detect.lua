local M = {}

--- Detect the best rendering tier for the current terminal.
--- @return "kitty"|"sixel"|"ascii"
function M.detect()
  local in_tmux = (vim.env.TMUX or "") ~= ""
  local is_ghostty = (vim.env.GHOSTTY_RESOURCES_DIR or "") ~= ""
    or (vim.env.TERM_PROGRAM or "") == "ghostty"
  local is_kitty = vim.env.KITTY_PID ~= nil or vim.env.TERM == "xterm-kitty"

  -- Tier 1: Kitty graphics protocol (Kitty and Ghostty, even in tmux)
  if is_kitty or is_ghostty then
    return "kitty"
  end

  -- Tier 2: Sixel
  local term_program = vim.env.TERM_PROGRAM or ""
  if term_program == "iTerm.app" then
    return "sixel"
  end
  if term_program == "WezTerm" then
    return "sixel"
  end

  -- foot terminal
  if (vim.env.TERM or ""):find "foot" then
    return "sixel"
  end

  -- tmux 3.4+ has native sixel support
  if in_tmux then
    local ok, result = pcall(vim.fn.system, "tmux -V")
    if ok and result then
      local major, minor = result:match "tmux (%d+)%.(%d+)"
      if major and minor then
        if tonumber(major) > 3 or (tonumber(major) == 3 and tonumber(minor) >= 4) then
          return "sixel"
        end
      end
    end
  end

  -- Tier 3: ASCII (universal fallback)
  return "ascii"
end

return M
