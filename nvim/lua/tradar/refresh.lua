-- Re-running the result on screen: `<leader>rR` once, `:TradarWatch [secs]`
-- every few seconds (a log table, a queue, `show processlist`).
--
-- Only statements that cannot change data are ever repeated (see
-- `guard.is_read_only`): a refresh is a key you can hit by accident and a
-- watch repeats itself, so `INSERT ... RETURNING` is refused here and has to
-- be run again from the SQL buffer on purpose.
local ctx = require('tradar.ctx')
local M = ctx.M
local state = ctx.state
local notify = ctx.notify
local guard = ctx.guard
local uv = ctx.uv

local DEFAULT_SECONDS = 5

--- The result a refresh would repeat, or nil and why not.
local function target()
  local r = state.result
  if not r or (r.kind ~= 'table' and r.kind ~= 'documents') then
    return nil, 'no rows on screen to refresh'
  end
  if not (r.conn and r.query) then return nil, 'this result has no query to repeat' end
  return r
end

--- Re-runs the result on screen in place (same result tab, no history
--- entry, same cursor position and filter / sort). `on_error(err)` is told when it fails; the grid
--- stays on screen either way.
function M.refresh(on_error)
  local r, why = target()
  if not r then return notify(why, vim.log.levels.WARN) end
  if ctx.dirty() then
    return notify('unsaved grid edits — :w to commit or ' .. (ctx.opts.prefix or '<leader>r') .. 'u to revert first', vim.log.levels.WARN)
  end
  if state.running then return notify('a query is already running', vim.log.levels.WARN) end
  ctx.driver_of(r.conn, function(driver)
    if not guard.is_read_only(r.query, driver) then
      return notify('not repeated: this statement may change data — run it again from the SQL buffer', vim.log.levels.WARN)
    end
    local win = state.results_buf and vim.fn.bufwinid(state.results_buf) or -1
    local buf = (state.sql_buf and vim.api.nvim_buf_is_valid(state.sql_buf)) and state.sql_buf or state.results_buf
    ctx.run_text(buf, r.query, 0, 0, {
      confirmed = true,
      conn = r.conn,
      refresh = { replacing = r, cursor = win ~= -1 and vim.api.nvim_win_get_cursor(win) or nil },
      on_error = on_error,
    })
  end)
end

local function stop(reason)
  local w = state.watch
  if not w then return false end
  w.timer:stop()
  if not w.timer:is_closing() then w.timer:close() end
  state.watch = nil
  if ctx.repaint_results then ctx.repaint_results() end
  if reason then notify(reason) end
  return true
end

ctx.stop_watch = stop

local function tick()
  local w = state.watch
  if not w then return end
  local r = state.result
  if not (r and r.query == w.query and r.conn == w.conn) then
    return stop('watch stopped: the results on screen changed')
  end
  if not (state.results_buf and vim.api.nvim_buf_is_valid(state.results_buf)) then return stop() end
  -- Never pile up behind a slow query, and never repaint over unsaved edits.
  if state.running or ctx.dirty() then return end
  M.refresh(function(err) stop('watch stopped: ' .. err) end)
end

--- `:TradarWatch [seconds|off]`: re-run the result on screen every few
--- seconds (default 5) until `off`, an error, or another result replaces it.
function M.watch(arg)
  arg = vim.trim(arg or '')
  if arg == 'off' or arg == 'stop' or arg == '0' then
    return stop('watch stopped') or notify('not watching anything')
  end
  local secs = arg == '' and DEFAULT_SECONDS or tonumber(arg)
  if not secs or secs < 1 then return notify('usage: :TradarWatch [seconds >= 1 | off]', vim.log.levels.WARN) end
  local r, why = target()
  if not r then return notify(why, vim.log.levels.WARN) end
  ctx.driver_of(r.conn, function(driver)
    if not guard.is_read_only(r.query, driver) then
      return notify('not watched: this statement may change data, and a watch would repeat it', vim.log.levels.WARN)
    end
    stop()
    local timer = uv.new_timer()
    state.watch = { timer = timer, secs = secs, query = r.query, conn = r.conn }
    timer:start(secs * 1000, secs * 1000, vim.schedule_wrap(tick))
    if ctx.repaint_results then ctx.repaint_results() end
    notify(('watching every %gs — :TradarWatch off stops'):format(secs))
  end)
end

--- `<leader>rW`: watch on (default interval) / off.
function M.toggle_watch()
  if state.watch then return M.watch('off') end
  M.watch('')
end

return M
