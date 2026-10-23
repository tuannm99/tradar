local ctx = require('tradar.ctx')
local M = ctx.M
local rpc = ctx.rpc
local guard = ctx.guard
local blocks = ctx.blocks
local uv = ctx.uv
local opts = ctx.opts
local SPINNER = ctx.SPINNER
local state = ctx.state
local notify = ctx.notify
local call = ctx.call
local resolve_buf = ctx.resolve_buf

-- ── connection binding ───────────────────────────────────────────────────

local function modeline(buf)
  for _, line in ipairs(vim.api.nvim_buf_get_lines(buf, 0, 10, false)) do
    local name = line:match('^%s*%-%-%s*tradar:%s*(.-)%s*$')
      or line:match('^%s*//%s*tradar:%s*(.-)%s*$')
      or line:match('^%s*#%s*tradar:%s*(.-)%s*$')
    if name and name ~= '' then return name end
  end
end

local function project_file(buf)
  local file = vim.api.nvim_buf_get_name(buf)
  if file == '' then return nil end
  local found = vim.fs.find('.tradar', { upward = true, path = vim.fs.dirname(file), type = 'file' })[1]
  if not found then return nil end
  for line in io.lines(found) do
    local name = vim.trim(line)
    if name ~= '' and name:sub(1, 1) ~= '#' then return name end
  end
end

--- Which saved connection a buffer talks to: an explicit `:TradarConnect`
--- in this buffer, else a `-- tradar: name` line near the top, else a
--- `.tradar` file in a parent directory, else the last one chosen.
function M.connection_for(buf)
  buf = resolve_buf(buf)
  return vim.b[buf].tradar_connection or modeline(buf) or project_file(buf) or state.default
end

--- Whether `buf` is a `.tdb` file: by filetype, or -- when that did not get
--- set (the plugin loaded after the buffer, a filetype override) -- by name, so
--- the fences are never sent to the server as if they were a query.
function M.is_tdb(buf)
  buf = resolve_buf(buf)
  return vim.bo[buf].filetype == 'tdb' or vim.api.nvim_buf_get_name(buf):match('%.tdb$') ~= nil
end

--- The fenced block (see `tradar.blocks`) 1-based `row` falls inside, for
--- a `.tdb` buffer -- `nil` for every other filetype (block syntax is
--- `.tdb`-only) and for a `.tdb` row that isn't inside any block. `loose`:
--- a fence line belongs to its block too (what `M.run` wants).
function M.block_at(buf, row, loose)
  buf = resolve_buf(buf)
  if not M.is_tdb(buf) then return nil end
  return blocks.at(vim.api.nvim_buf_get_lines(buf, 0, -1, false), row, loose)
end

--- `M.connection_for(buf)`, except a `.tdb` block that names its own
--- connection in its opening fence (` ```sql pg-local `) wins over the
--- buffer-wide one -- each block in a `.tdb` file can talk to a different
--- connection. Falls back to `M.connection_for(buf)` for a block with no
--- connection of its own, same as every other filetype.
function M.block_connection(buf, row)
  local block = M.block_at(buf, row)
  return (block and block.conn) or M.connection_for(buf)
end

--- `cb(driver_id)` for a connection ("sqlite", "postgres", ...); cached.
local function driver_of(conn, cb)
  if state.drivers then return cb(state.drivers[conn]) end
  call('connections.list', nil, function(list)
    state.drivers = {}
    for _, c in ipairs(list) do state.drivers[c.name] = c.driver end
    cb(state.drivers[conn])
  end)
end

--- The driver id of `conn` if `connections.list` has been fetched already
--- (nil otherwise) -- for callers that cannot wait for an async lookup.
function M.cached_driver(conn) return state.drivers and state.drivers[conn] or nil end

--- Whether the server has confirmed a connection (not merely a binding).
function M.is_connected(name) return state.connected[name] == true end

--- Connections whose name matches `setup{ protected = {...} }` (default
--- `{"prod"}`): every write asks first, and the statusline shows `⚠`.
function M.is_protected(name)
  return guard.is_protected(name, opts.protected or { 'prod' })
end

ctx.ensure_connected = function(name, cb, on_error)
  if state.connected[name] then return cb() end
  if state.connecting[name] then
    table.insert(state.connecting[name], { cb, on_error })
    return
  end
  state.connecting[name] = { { cb, on_error } }
  local function settle(ok, err)
    local waiting = state.connecting[name]
    state.connecting[name] = nil
    for _, w in ipairs(waiting) do
      if ok then w[1]() elseif w[2] then w[2](err) else notify(err, vim.log.levels.ERROR) end
    end
  end
  call('connect', { connection = name }, function()
    state.connected[name] = true
    state.idle[name] = nil
    M._start_poller()
    settle(true)
  end, function(err) settle(false, err) end)
end

-- ── status (lualine / statusline) ────────────────────────────────────────

local function elapsed_text(ns_since)
  local seconds = (uv.hrtime() - ns_since) / 1e9
  return ('%.1fs'):format(seconds)
end

--- Unsaved edits in the grid (`:w` commits them, `<leader>ru` reverts).
local function dirty()
  local buf = state.results_buf
  return buf ~= nil and vim.api.nvim_buf_is_valid(buf) and vim.bo[buf].modified and state.result ~= nil
end

--- Short text for a statusline: `db demo · 120/5000 rows · 83ms`, a spinner
--- while a query runs, `✗` when the connection stopped answering. Empty off
--- SQL/tradar buffers so it costs a statusline nothing there.
function M.status()
  local in_tradar = vim.b.tradar_attached or vim.api.nvim_buf_get_name(0):find('^tradar://') ~= nil
  if not in_tradar and not state.running then return '' end
  -- In the results buffer the connection is the *result's*, not whichever
  -- one the buffer-wide fallback (last chosen) happens to name.
  local in_results = vim.api.nvim_buf_get_name(0):find('^tradar://results') ~= nil
  local conn = (state.running and state.running.conn)
    or (in_results and state.result and state.result.conn)
    or M.block_connection(0, vim.api.nvim_win_get_cursor(0)[1])
  if not conn then return '' end
  local parts = { 'db ' .. conn .. (M.is_protected(conn) and ' ⚠' or '') }
  if state.alive[conn] == false then
    parts[1] = parts[1] .. ' ✗'
  elseif state.idle[conn] then
    parts[1] = parts[1] .. ' (idle)'
  end
  if state.in_tx[conn] then parts[1] = parts[1] .. ' tx' end
  if state.running then
    local frame = SPINNER[(math.floor(uv.hrtime() / 1e8) % #SPINNER) + 1]
    parts[#parts + 1] = frame .. ' ' .. elapsed_text(state.running.started)
  elseif state.last then
    parts[#parts + 1] = state.last
  end
  if in_results and dirty() then parts[#parts + 1] = '✎ unsaved edits' end
  return table.concat(parts, ' · ')
end


--- Shown on the results window while a query runs (and nowhere else): a
--- spinner, the elapsed time and how to cancel. It is a `winbar`, so each
--- tick repaints one window -- not the whole statusline, which is what the
--- old 10 Hz `redrawstatus` did to a heavy statusline (typing lagged for
--- as long as a query ran).
local function progress_text()
  local frame = SPINNER[(math.floor(uv.hrtime() / 1e8) % #SPINNER) + 1]
  return (' %s running %s on %s — %sx cancels'):format(
    frame, elapsed_text(state.running.started), state.running.conn, opts.prefix or '<leader>r')
end

local function set_progress_bar()
  local buf = state.results_buf
  local win = buf and vim.api.nvim_buf_is_valid(buf) and vim.fn.bufwinid(buf) or -1
  if win == -1 then return end
  vim.wo[win].winbar = state.running and progress_text():gsub('%%', '%%%%') or (ctx.slots_bar and ctx.slots_bar() or '')
end

local function start_spinner()
  if state.spinner then return end
  -- If the results window is closed, open it so a long query shows *something*.
  local buf = state.results_buf
  if not (buf and vim.api.nvim_buf_is_valid(buf) and vim.fn.bufwinid(buf) ~= -1) then
    ctx.show_message('running…', 4)
  end
  set_progress_bar()
  state.spinner = uv.new_timer()
  local ticks = 0
  state.spinner:start(200, 200, vim.schedule_wrap(function()
    set_progress_bar()
    ticks = ticks + 1
    -- The statusline (lualine & co.) only needs a nudge once a second.
    if ticks % 5 == 0 then vim.cmd.redrawstatus() end
  end))
end

local function stop_spinner()
  if state.spinner then
    state.spinner:stop()
    state.spinner:close()
    state.spinner = nil
  end
  set_progress_bar() -- state.running is nil by now: the slot tabs come back
  vim.cmd.redrawstatus()
end

--- Pings the connections in use every 15s (what the TUI's engine tick does)
--- so a dropped connection shows as `✗` before a query runs into it.
function M._start_poller()
  if state.poller then return end
  state.poller = uv.new_timer()
  state.poller:start(15000, 15000, vim.schedule_wrap(function()
    for name in pairs(state.connected) do
      rpc.request('status', { connection = name }, function(err, result)
        if err then
          if err:find('not connected', 1, true) then
            -- Closed on purpose (idle, or a restarted server), not broken:
            -- the next call reconnects, so this is not shown as `✗`.
            state.connected[name], state.idle[name], state.alive[name] = nil, true, nil
          else
            state.alive[name] = false
          end
        else
          state.alive[name], state.in_tx[name] = result.alive, result.in_transaction
        end
        vim.cmd.redrawstatus()
      end)
    end
  end))
end

ctx.driver_of = driver_of
ctx.dirty = dirty
ctx.start_spinner = start_spinner
ctx.stop_spinner = stop_spinner
ctx.modeline = modeline
