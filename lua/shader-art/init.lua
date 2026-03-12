local M = {}

local config = require "shader-art.config"
local detect = require "shader-art.detect"

--- Half-block encoding: each character cell encodes 2 vertical pixels (top=fg, bottom=bg)
--- but only 1 horizontal pixel per character column.
local ASCII_PX_PER_COL = 1
local ASCII_PX_PER_ROW = 2

--- Kitty render resolution: detected cell pixel size, or fallback defaults.
--- Populated by detect_cell_size() on first use; cleared on VimResized.
local cell_px_width = nil
local cell_px_height = nil

--- Kitty graphics cleanup: delete both double-buffer image IDs.
local KITTY_DELETE = "\x1b_Ga=d,d=I,i=1,q=2\x1b\\\x1b_Ga=d,d=I,i=2,q=2\x1b\\"

--- Plugin root directory (where lua/shader-art/ lives).
local plugin_root = vim.fn.fnamemodify(debug.getinfo(1, "S").source:sub(2), ":h:h:h")

--- Augroup for all shader-art autocmds (cleared on re-source to prevent accumulation).
local augroup = vim.api.nvim_create_augroup("ShaderArt", { clear = true })

--- Consolidated module state.
local state = {
  --- Active sidecar job ID, or nil.
  active_job = nil,
  --- Open tty file handle for the current sidecar, or nil.
  tty = nil,
  --- Cached resolved mode from make_element.
  mode = nil,
  --- Cached character dimensions.
  el_width = nil,
  el_height = nil,
  --- Row offset from window top to shader element (alpha layout dependent).
  row_offset = 3,
  --- Shader cycling index.
  shader_index = 1,
}

--- Detect the real tty device path for Neovim's stdout.
--- jobstart() children have no controlling terminal, so /dev/tty fails.
--- We find the actual device (e.g. /dev/ttys001) from Neovim's own FD 1.
local function detect_tty()
  local pid = vim.fn.getpid()
  local output = vim.fn.system("lsof -a -p " .. pid .. " -d 1 -Fn 2>/dev/null")
  for line in output:gmatch "[^\n]+" do
    if line:match "^n/dev/" then
      return line:sub(2)
    end
  end
  return "/dev/tty"
end

--- Detect cell pixel size via TIOCGWINSZ on the real tty.
--- Returns (cell_width, cell_height) in pixels.
local function detect_cell_size()
  if cell_px_width and cell_px_height then
    return cell_px_width, cell_px_height
  end
  local tty = detect_tty()
  local cmd = string.format(
    "python3 -c \""
      .. "import fcntl,struct,os,termios; "
      .. "fd=os.open('%s',os.O_RDONLY); "
      .. "r=fcntl.ioctl(fd,termios.TIOCGWINSZ,b'\\x00'*8); "
      .. "os.close(fd); "
      .. "rows,cols,xpx,ypx=struct.unpack('HHHH',r); "
      .. "print(xpx//cols if cols>0 else 8,ypx//rows if rows>0 else 16)"
      .. "\" 2>/dev/null",
    tty
  )
  local output = vim.fn.system(cmd):gsub("%s+$", "")
  local cw, ch = output:match "(%d+)%s+(%d+)"
  if cw and ch then
    cell_px_width = tonumber(cw)
    cell_px_height = tonumber(ch)
  else
    cell_px_width = 8
    cell_px_height = 16
  end
  return cell_px_width, cell_px_height
end

--- Path to the sidecar binary.
local function binary_path()
  return plugin_root .. "/shader-art-render/target/release/shader-art-render"
end

--- Return sorted list of bundled .art shader files.
--- @return string[]
local function get_sorted_shaders()
  local shaders = vim.fn.globpath(plugin_root .. "/shaders", "*.art", false, true)
  table.sort(shaders)
  return shaders
end

--- Resolve shader path: user config > bundled shader at current index.
local function shader_path()
  if config.config.shader then
    return vim.fn.expand(config.config.shader)
  end
  local shaders = get_sorted_shaders()
  if #shaders == 0 then
    return nil
  end
  local idx = state.shader_index
  if idx < 1 or idx > #shaders then
    idx = 1
    state.shader_index = idx
  end
  return shaders[idx]
end

--- Stop the active sidecar process and clean up resources.
--- @param cleanup_escapes? string optional escape sequences to write before closing tty
local function stop_sidecar(cleanup_escapes)
  if state.active_job then
    pcall(vim.fn.jobstop, state.active_job)
    state.active_job = nil
  end
  pcall(vim.api.nvim_del_augroup_by_name, "ShaderArtResize")
  if state.tty then
    if cleanup_escapes then
      pcall(state.tty.write, state.tty, cleanup_escapes)
    end
    pcall(state.tty.write, state.tty, "\x1b[?25h")
    pcall(state.tty.close, state.tty)
    state.tty = nil
  end
end

--- Build the sidecar command line.
--- @param mode string "ascii"|"kitty"|"sixel"
--- @param el_width number character columns
--- @param el_height number character rows
--- @return string[]|nil command args, or nil on error
local function build_command(mode, el_width, el_height)
  local bin = binary_path()
  if vim.fn.executable(bin) ~= 1 then
    return nil
  end

  local art = shader_path()
  if not art then
    return nil
  end

  local pixel_width, pixel_height
  if mode == "kitty" then
    local cw, ch = detect_cell_size()
    pixel_width = el_width * cw
    pixel_height = el_height * ch
  else
    pixel_width = el_width * ASCII_PX_PER_COL
    pixel_height = el_height * ASCII_PX_PER_ROW
  end

  local cmd = {
    bin,
    "--mode",
    mode,
    "--width",
    tostring(pixel_width),
    "--height",
    tostring(pixel_height),
    "--fps",
    tostring(config.config.fps),
    art,
  }

  if mode == "kitty" then
    table.insert(cmd, "--cols")
    table.insert(cmd, tostring(el_width))
    table.insert(cmd, "--rows")
    table.insert(cmd, tostring(el_height))
  end

  if mode == "sixel" then
    table.insert(cmd, "--tty")
    table.insert(cmd, detect_tty())
  end

  return cmd
end

--- Get the sidecar command as a string (for external use).
--- @return string|nil
function M.get_command()
  local mode = config.config.mode
  if mode == "auto" then
    mode = detect.detect()
  end

  local char_width = config.config.width or 69
  local char_height = config.config.height or 16

  local cmd = build_command(mode, char_width, char_height)
  if not cmd then
    return nil
  end

  return table.concat(cmd, " ")
end

--- Find the alpha window. Returns win ID or nil.
local function find_alpha_win()
  for _, win in ipairs(vim.api.nvim_list_wins()) do
    local ok, buf = pcall(vim.api.nvim_win_get_buf, win)
    if ok then
      local ft_ok, ft = pcall(vim.api.nvim_get_option_value, "filetype", { buf = buf })
      if ft_ok and ft == "alpha" then
        return win
      end
    end
  end
  return nil
end

--- Build a Kitty APC escape with chunked base64 data.
--- @param control string Kitty control params
--- @param b64 string base64-encoded image data
--- @return string complete APC escape sequence
local function build_kitty_apc(control, b64)
  local parts = {}
  local chunk_size = 4096
  local len = #b64

  if len == 0 then
    return "\x1b_G" .. control .. "\x1b\\"
  end

  local offset = 1
  local first = true
  while offset <= len do
    local chunk = b64:sub(offset, offset + chunk_size - 1)
    local more = (offset + chunk_size - 1 < len) and 1 or 0
    if first then
      table.insert(parts, "\x1b_G" .. control .. ",m=" .. more .. ";" .. chunk .. "\x1b\\")
      first = false
    else
      table.insert(parts, "\x1b_Gm=" .. more .. ";" .. chunk .. "\x1b\\")
    end
    offset = offset + chunk_size
  end

  return table.concat(parts)
end

--- Start a sidecar process with shared setup (tty, position, autocmds, lifecycle).
--- @param mode string "ascii"|"kitty"
--- @param el_width number character columns
--- @param el_height number character rows
--- @param on_stdout_fn function(tty, pos, data) mode-specific stdout handler
--- @param cleanup_escapes? string escape sequences for cleanup (e.g. Kitty image delete)
local function start_sidecar(mode, el_width, el_height, on_stdout_fn, cleanup_escapes)
  stop_sidecar(cleanup_escapes)

  -- Mutable position — updated on VimResized
  local pos = { row = 0, col = 0 }
  local function recalc_position()
    local win = find_alpha_win()
    if not win then
      return
    end
    local win_pos = vim.api.nvim_win_get_position(win)
    pos.row = win_pos[1] + state.row_offset
    local win_width = vim.api.nvim_win_get_width(win)
    pos.col = win_pos[2] + math.floor((win_width - el_width) / 2) + 1
    if pos.col < 1 then
      pos.col = 1
    end
  end
  recalc_position()

  local cmd = build_command(mode, el_width, el_height)
  if not cmd then
    return
  end

  local tty_path = detect_tty()
  local tty = io.open(tty_path, "w")
  if not tty then
    return
  end
  state.tty = tty

  tty:write "\x1b[?25l"
  tty:flush()

  local resize_group = vim.api.nvim_create_augroup("ShaderArtResize", { clear = true })
  vim.api.nvim_create_autocmd("VimResized", {
    group = resize_group,
    callback = function()
      -- Invalidate cell size cache (monitor/DPI may have changed)
      cell_px_width = nil
      cell_px_height = nil
      recalc_position()
    end,
  })

  local job_id = vim.fn.jobstart(cmd, {
    on_stdout = function(_, data)
      on_stdout_fn(tty, pos, data)
    end,
    on_exit = function(j)
      if state.active_job == j then
        state.active_job = nil
      end
      vim.schedule(function()
        if state.tty == tty then
          stop_sidecar(cleanup_escapes)
        else
          pcall(tty.close, tty)
        end
      end)
    end,
    on_stderr = function(_, data)
      if data and data[1] and data[1] ~= "" then
        vim.schedule(function()
          vim.notify("[shader-art] " .. table.concat(data, "\n"), vim.log.levels.WARN)
        end)
      end
    end,
  })
  state.active_job = job_id

  local sidecar_group = vim.api.nvim_create_augroup("ShaderArtSidecar", { clear = true })

  vim.api.nvim_create_autocmd("User", {
    group = sidecar_group,
    pattern = "AlphaClosed",
    once = true,
    callback = function()
      stop_sidecar(cleanup_escapes)
    end,
  })

  -- On BufLeave: restore cursor visibility (hidden by sidecar start) and
  -- delete Kitty images so they don't bleed through floating windows.
  -- Without this, the terminal cursor stays hidden/mispositioned when
  -- navigating away from alpha before AlphaClosed fires.
  vim.api.nvim_create_autocmd("BufLeave", {
    group = sidecar_group,
    callback = function()
      if state.tty then
        local esc = "\x1b[?25h"
        if cleanup_escapes then
          esc = cleanup_escapes .. esc
        end
        pcall(state.tty.write, state.tty, esc)
        pcall(state.tty.flush, state.tty)
      end
    end,
  })

  -- On BufEnter back to alpha: re-hide cursor so the sidecar can render cleanly.
  vim.api.nvim_create_autocmd("BufEnter", {
    group = sidecar_group,
    callback = function()
      if vim.bo.filetype == "alpha" and state.tty then
        pcall(state.tty.write, state.tty, "\x1b[?25l")
        pcall(state.tty.flush, state.tty)
      end
    end,
  })
end

--- Start the ASCII sidecar process.
local function start_ascii_sidecar(el_width, el_height)
  local partial = ""
  local row_buf = {}
  local total_rows = el_height

  start_sidecar("ascii", el_width, el_height, function(tty, pos, data)
    if not data then
      return
    end
    for i, chunk in ipairs(data) do
      if i == 1 then
        chunk = partial .. chunk
        partial = ""
      end
      if i == #data then
        partial = chunk
      elseif chunk ~= "" then
        table.insert(row_buf, chunk)
        if #row_buf >= total_rows then
          if vim.bo.filetype == "alpha" and state.tty == tty then
            local parts = { "\x1b[s" }
            for r, row_data in ipairs(row_buf) do
              parts[#parts + 1] = string.format("\x1b[%d;%dH%s", pos.row + r - 1, pos.col, row_data)
            end
            parts[#parts + 1] = "\x1b[u"
            pcall(tty.write, tty, table.concat(parts))
            pcall(tty.flush, tty)
          end
          row_buf = {}
        end
      end
    end
  end)
end

--- Start the Kitty sidecar with double-buffered graphics protocol.
local function start_kitty_sidecar(el_width, el_height)
  local front_id = 0
  local back_id = 1
  local partial = ""

  start_sidecar("kitty", el_width, el_height, function(tty, pos, data)
    if not data then
      return
    end
    for i, chunk in ipairs(data) do
      if i == 1 then
        chunk = partial .. chunk
        partial = ""
      end
      if i == #data then
        partial = chunk
      elseif chunk ~= "" then
        local header, b64 = chunk:match "^([^;]+);(.+)$"
        if header and b64 and vim.bo.filetype == "alpha" and state.tty == tty then
          local w, h, cols, rows = header:match "^(%d+),(%d+),(%d+),(%d+)$"
          if w then
            local transmit_ctl = string.format("a=t,f=100,i=%d,s=%s,v=%s,q=2", back_id, w, h)
            local transmit_apc = build_kitty_apc(transmit_ctl, b64)

            local place_apc = string.format(
              "\x1b_Ga=p,i=%d,c=%s,r=%s,C=1,q=2\x1b\\",
              back_id, cols, rows
            )

            local delete_apc = ""
            if front_id > 0 then
              delete_apc = string.format("\x1b_Ga=d,d=I,i=%d,q=2\x1b\\", front_id)
            end

            local esc = string.format(
              "%s\x1b[s\x1b[%d;%dH%s%s\x1b[u",
              transmit_apc, pos.row, pos.col, place_apc, delete_apc
            )
            pcall(tty.write, tty, esc)
            pcall(tty.flush, tty)

            front_id = back_id
            back_id = (back_id == 1) and 2 or 1
          end
        end
      end
    end
  end, KITTY_DELETE)
end

--- Create an alpha dashboard element for the shader.
--- Both modes return a padding element. A sidecar process writes frames directly
--- to the tty with cursor positioning (piped-to-tty architecture).
--- Sixel mode: falls back to ASCII (no placeholder mechanism for Sixel).
--- @param opts? {width: number, height: number, fps: number, shader: string, row_offset: number}
--- @return table|nil alpha layout element, or nil if binary missing
function M.make_element(opts)
  opts = opts or {}

  if opts.fps or opts.shader then
    config.setup(opts)
  end

  local mode = config.config.mode
  if mode == "auto" then
    mode = detect.detect()
  end

  if mode == "sixel" then
    mode = "ascii"
  end

  local el_width = opts.width or config.config.width or 69
  local el_height = opts.height or config.config.height or 16

  state.mode = mode
  state.el_width = el_width
  state.el_height = el_height
  if opts.row_offset then
    state.row_offset = opts.row_offset
  end

  local bin = binary_path()
  if vim.fn.executable(bin) ~= 1 then
    return nil
  end
  if not shader_path() then
    return nil
  end

  local element = {
    type = "padding",
    val = el_height,
  }

  local start_fn = mode == "kitty" and start_kitty_sidecar or start_ascii_sidecar

  vim.api.nvim_create_autocmd("User", {
    group = augroup,
    pattern = "AlphaReady",
    callback = function()
      -- Pick a random shader on each alpha open (unless user specified one)
      if not config.config.shader then
        local shaders = get_sorted_shaders()
        if #shaders > 0 then
          state.shader_index = math.random(1, #shaders)
        end
      end
      start_fn(el_width, el_height)
    end,
  })

  return element
end

--- Cycle to the next bundled shader and restart rendering.
function M.next_shader()
  local shaders = get_sorted_shaders()
  if #shaders == 0 then
    vim.notify("[shader-art] No .art shaders found", vim.log.levels.WARN)
    return
  end

  if not state.el_width or not state.el_height then
    vim.notify("[shader-art] No active shader to cycle", vim.log.levels.WARN)
    return
  end

  -- Sync index to current shader (handles external changes to shader list)
  local current = shader_path()
  for i, path in ipairs(shaders) do
    if path == current then
      state.shader_index = i
      break
    end
  end

  state.shader_index = (state.shader_index % #shaders) + 1
  local name = vim.fn.fnamemodify(shaders[state.shader_index], ":t")
  vim.notify("[shader-art] Switched to " .. name, vim.log.levels.INFO)

  local alpha_visible = vim.bo.filetype == "alpha"

  if state.active_job or alpha_visible then
    if state.mode == "kitty" then
      start_kitty_sidecar(state.el_width, state.el_height)
    else
      start_ascii_sidecar(state.el_width, state.el_height)
    end
  end
end

--- Setup function called from lazy.nvim config.
--- @param opts? table
function M.setup(opts)
  config.setup(opts)
  math.randomseed(os.time() + (vim.uv or vim.loop).hrtime() % 1000000)

  vim.api.nvim_create_user_command("ShaderArtNext", function()
    M.next_shader()
  end, { desc = "Cycle to the next shader" })
end

return M
