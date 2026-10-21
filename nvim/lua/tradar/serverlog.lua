-- `:TradarLog`: the server's own log file, followed like `tail -f`.
-- The server (`tradar-server`) writes it -- requests with their timing,
-- failures, clients coming and going; never query text or passwords. See
-- `crates/tradar-server/src/logging.rs`.
local rpc = require('tradar.rpc')
local uv = vim.uv or vim.loop

local M = {}

--- Mirrors `logging::default_log_path` for when the server is not running
--- (or predates `log_path` in `version`).
local function default_path()
  local env = vim.env.TRADAR_LOG_FILE
  if env and env ~= '' then return env end
  local state = vim.env.XDG_STATE_HOME
  if not state or state == '' then state = (vim.env.HOME or '') .. '/.local/state' end
  return state .. '/tradar/server.log'
end

--- Where the running server logs (`version.log_path`), else the default.
function M.path(cb)
  rpc.request('version', nil, function(err, result)
    cb((not err and result and result.log_path) or default_path())
  end)
end

local timer

local function stop_following()
  if timer then
    timer:stop()
    if not timer:is_closing() then timer:close() end
    timer = nil
  end
end

--- Opens the log in a bottom split that follows new lines while the cursor
--- sits on the last one (move up to read; `G` to follow again). `q` closes.
function M.open()
  M.path(function(path)
    if vim.fn.filereadable(path) == 0 then
      return vim.notify(('tradar: no server log at %s (server not started yet, or TRADAR_LOG=off)'):format(path), vim.log.levels.WARN)
    end
    local existing = vim.fn.bufnr(vim.fn.fnameescape(path) --[[@as string]])
    local win = existing ~= -1 and vim.fn.bufwinid(existing) or -1
    if win ~= -1 then
      vim.api.nvim_set_current_win(win)
    else
      vim.cmd('botright 15split ' .. vim.fn.fnameescape(path))
    end
    local buf = vim.api.nvim_get_current_buf()
    vim.bo[buf].autoread = true
    vim.bo[buf].modifiable = false
    vim.bo[buf].swapfile = false
    vim.bo[buf].filetype = 'tradarlog'
    vim.wo.wrap = false
    vim.keymap.set('n', 'q', '<cmd>close<cr>', { buffer = buf, desc = 'tradar: close the log' })
    vim.cmd('checktime')
    vim.cmd('normal! G')

    stop_following()
    timer = uv.new_timer()
    timer:start(1000, 1000, vim.schedule_wrap(function()
      local w = vim.api.nvim_buf_is_valid(buf) and vim.fn.bufwinid(buf) or -1
      if w == -1 then return stop_following() end
      local at_end = vim.api.nvim_win_get_cursor(w)[1] >= vim.api.nvim_buf_line_count(buf)
      vim.cmd('silent! checktime ' .. buf)
      if at_end then
        pcall(vim.api.nvim_win_set_cursor, w, { vim.api.nvim_buf_line_count(buf), 0 })
      end
    end))
  end)
end

return M
