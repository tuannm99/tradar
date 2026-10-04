-- tradar.nvim: the editor is a real Neovim buffer, tradar-server does the
-- querying. See "Server headless" in docs/architecture.md.
local rpc = require('tradar.rpc')
local render = require('tradar.render')
local uv = vim.uv or vim.loop

local M = {}

local opts = {}
local ns = vim.api.nvim_create_namespace('tradar')

local PAGE = 200
local SPINNER = { '⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏' }
local HISTORY_MAX = 200

local state = {
  default = nil, -- last connection chosen explicitly; fallback for unbound buffers
  connected = {}, -- name -> true once the server confirmed `connect`
  connecting = {}, -- name -> callbacks waiting on an in-flight `connect`
  alive = {}, -- name -> bool, from the background `status` poll
  in_tx = {}, -- name -> bool
  history = nil, -- lazily loaded
  running = nil, -- { id, conn, started }
  last = nil, -- summary of the last finished query, for the statusline
  counter = 0,
  spinner = nil, -- uv timer while a query runs
  poller = nil, -- uv timer for status polling
  result = nil, -- the cursor currently shown in the results buffer
  results_buf = nil,
  schema_buf = nil,
  schema_targets = {},
  sql_buf = nil, -- buffer the navigator inserts into
}

local function notify(msg, level) vim.notify('tradar: ' .. msg, level or vim.log.levels.INFO) end

local function call(method, params, cb, on_error)
  rpc.ensure(opts, function(err)
    if err then
      if on_error then return on_error(err) end
      return notify(err, vim.log.levels.ERROR)
    end
    rpc.request(method, params, function(e, result, data)
      if e then
        if on_error then return on_error(e, data) end
        return notify(e, vim.log.levels.ERROR)
      end
      cb(result)
    end)
  end)
end

local function resolve_buf(buf)
  if buf == nil or buf == 0 then return vim.api.nvim_get_current_buf() end
  return buf
end

-- ── connection binding ───────────────────────────────────────────────────

local function modeline(buf)
  for _, line in ipairs(vim.api.nvim_buf_get_lines(buf, 0, 10, false)) do
    local name = line:match('^%s*%-%-%s*tradar:%s*(.-)%s*$') or line:match('^%s*//%s*tradar:%s*(.-)%s*$')
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

--- Whether the server has confirmed a connection (not merely a binding).
function M.is_connected(name) return state.connected[name] == true end

local function ensure_connected(name, cb, on_error)
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
    M._start_poller()
    settle(true)
  end, function(err) settle(false, err) end)
end

-- ── status (lualine / statusline) ────────────────────────────────────────

local function elapsed_text(ns_since)
  local seconds = (uv.hrtime() - ns_since) / 1e9
  return ('%.1fs'):format(seconds)
end

--- Short text for a statusline: `db demo · 120/5000 rows · 83ms`, a spinner
--- while a query runs, `✗` when the connection stopped answering. Empty off
--- SQL/tradar buffers so it costs a statusline nothing there.
function M.status()
  local bo = vim.bo
  local in_tradar = bo.filetype == 'sql' or vim.api.nvim_buf_get_name(0):find('^tradar://') ~= nil
  if not in_tradar and not state.running then return '' end
  local conn = (state.running and state.running.conn) or M.connection_for(0)
  if not conn then return '' end
  local parts = { 'db ' .. conn }
  if state.alive[conn] == false then parts[1] = parts[1] .. ' ✗' end
  if state.in_tx[conn] then parts[1] = parts[1] .. ' tx' end
  if state.running then
    local frame = SPINNER[(math.floor(uv.hrtime() / 1e8) % #SPINNER) + 1]
    parts[#parts + 1] = frame .. ' ' .. elapsed_text(state.running.started)
  elseif state.last then
    parts[#parts + 1] = state.last
  end
  return table.concat(parts, ' · ')
end

local function start_spinner()
  if state.spinner then return end
  state.spinner = uv.new_timer()
  state.spinner:start(100, 100, vim.schedule_wrap(function() vim.cmd.redrawstatus() end))
end

local function stop_spinner()
  if state.spinner then
    state.spinner:stop()
    state.spinner:close()
    state.spinner = nil
  end
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
          state.alive[name] = false
          if err:find('not connected', 1, true) then state.connected[name] = nil end
        else
          state.alive[name], state.in_tx[name] = result.alive, result.in_transaction
        end
        vim.cmd.redrawstatus()
      end)
    end
  end))
end

-- ── history ──────────────────────────────────────────────────────────────

local function history_path() return vim.fn.stdpath('state') .. '/tradar_history.json' end

function M.history()
  if state.history then return state.history end
  state.history = {}
  local f = io.open(history_path(), 'r')
  if f then
    local ok, decoded = pcall(vim.json.decode, f:read('*a'))
    f:close()
    if ok and type(decoded) == 'table' then state.history = decoded end
  end
  return state.history
end

local function remember(conn, text)
  local history = M.history()
  text = vim.trim(text)
  if text == '' or (history[1] and history[1].text == text and history[1].conn == conn) then return end
  table.insert(history, 1, { conn = conn, text = text })
  while #history > HISTORY_MAX do table.remove(history) end
  vim.fn.mkdir(vim.fn.fnamemodify(history_path(), ':h'), 'p')
  local f = io.open(history_path(), 'w')
  if f then
    f:write(vim.json.encode(history))
    f:close()
  end
end

-- ── results buffer ───────────────────────────────────────────────────────

local function scratch(name, existing)
  if existing and vim.api.nvim_buf_is_valid(existing) then return existing end
  local buf = vim.api.nvim_create_buf(false, true)
  vim.bo[buf].buftype = 'nofile'
  vim.bo[buf].bufhidden = 'hide'
  vim.bo[buf].swapfile = false
  vim.api.nvim_buf_set_name(buf, name)
  return buf
end

local function show(buf, height)
  local win = vim.fn.bufwinid(buf)
  if win ~= -1 then return win end
  local origin = vim.api.nvim_get_current_win()
  vim.cmd(('botright %dsplit'):format(height or 15))
  win = vim.api.nvim_get_current_win()
  vim.api.nvim_win_set_buf(win, buf)
  vim.wo[win].wrap = false
  vim.wo[win].cursorline = true
  -- Results are for looking at: keep the editor focused, as an IDE would.
  vim.api.nvim_set_current_win(origin)
  return win
end

local function set_lines(buf, lines)
  vim.bo[buf].modifiable = true
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
  vim.bo[buf].modifiable = false
end

local function results_title(r)
  local text = ('%d/%d rows%s'):format(r.shown, r.total, r.truncated and ' (server cap reached)' or '')
  if r.shown < r.total then text = text .. '  — scroll down or <leader>rm for more' end
  return text
end

local attach_results

local function paint_results()
  local r = state.result
  local buf = state.results_buf
  local lines
  if r.kind == 'table' then
    lines, r.spans = render.table(r.columns, r.rows)
  else
    lines = render.documents(r.rows)
  end
  lines[#lines + 1] = ''
  lines[#lines + 1] = results_title(r)
  local win = vim.fn.bufwinid(buf)
  local pos = win ~= -1 and vim.api.nvim_win_get_cursor(win) or nil
  set_lines(buf, lines)
  if pos then pcall(vim.api.nvim_win_set_cursor, win, { math.min(pos[1], #lines), pos[2] }) end
end

local function show_message(text, height)
  local buf = scratch('tradar://results', state.results_buf)
  state.results_buf = buf
  attach_results(buf)
  state.result = nil
  set_lines(buf, vim.split(text, '\n', { plain = true }))
  show(buf, height or 6)
end

local function show_result(r)
  local buf = scratch('tradar://results', state.results_buf)
  state.results_buf = buf
  attach_results(buf)
  if r.kind == 'affected' then
    state.result = nil
    set_lines(buf, { ('%d row(s) affected'):format(r.rows) })
    return show(buf, 4)
  end
  state.result = {
    id = r.cursor, total = r.total, shown = #r.rows, kind = r.kind,
    columns = r.columns, rows = r.rows, truncated = r.truncated, fetching = false,
  }
  paint_results()
  local win = show(buf)
  pcall(vim.api.nvim_win_set_cursor, win, { 1, 0 })
end

--- Appends the next page. Called by scrolling to the bottom and by
--- `<leader>rm`; `cb` runs when the rows have landed (or immediately if
--- there is nothing more).
function M.more(cb)
  local r = state.result
  if not r or r.shown >= r.total or r.fetching then
    if cb then cb() end
    return
  end
  r.fetching = true
  call('fetch', { cursor = r.id, offset = r.shown, limit = cb and 2000 or PAGE }, function(page)
    r.fetching = false
    if state.result ~= r then return end
    vim.list_extend(r.rows, page.rows)
    r.shown = #r.rows
    paint_results()
    if cb then cb() end
  end, function(err)
    r.fetching = false
    notify(err, vim.log.levels.ERROR)
  end)
end

local function all_rows(cb)
  local r = state.result
  if not r then return notify('no result to use', vim.log.levels.WARN) end
  if r.shown >= r.total then return cb(r) end
  local function step()
    if r.shown >= r.total or state.result ~= r then return cb(r) end
    M.more(step)
  end
  notify(('loading the remaining %d rows…'):format(r.total - r.shown))
  step()
end

local function yank(text, what)
  vim.fn.setreg('"', text)
  pcall(vim.fn.setreg, '+', text)
  notify(('yanked %s'):format(what))
end

--- Row index and column index under the results cursor (nil off the grid).
local function cell_at()
  local r = state.result
  if not r or r.kind ~= 'table' then return nil end
  local row = vim.api.nvim_win_get_cursor(0)[1] - 2
  if row < 1 or row > #r.rows then return nil end
  return row, render.column_at(r.spans, vim.fn.virtcol('.'))
end

local function export_text(fmt, r)
  if r.kind == 'documents' then
    if fmt == 'json' then return vim.json.encode(r.rows) .. '\n' end
    return nil, 'documents can only be exported as json'
  end
  if fmt == 'csv' then return render.csv(r.columns, r.rows) end
  if fmt == 'json' then return render.json(r.columns, r.rows) end
  if fmt == 'md' or fmt == 'markdown' then return render.markdown(r.columns, r.rows) end
  if fmt == 'tsv' then return render.tsv(r.columns, r.rows, true) .. '\n' end
  return nil, 'unknown format `' .. tostring(fmt) .. '` (csv, json, md, tsv)'
end

--- Yanks the whole result (every row, loading what's missing) in `fmt`.
function M.yank_all(fmt)
  all_rows(function(r)
    local text, err = export_text(fmt, r)
    if not text then return notify(err, vim.log.levels.WARN) end
    yank(text, ('%d rows as %s'):format(#r.rows, fmt))
  end)
end

--- Writes the whole result to `path` (asks if omitted).
function M.export(fmt, path)
  fmt = fmt or 'csv'
  all_rows(function(r)
    local text, err = export_text(fmt, r)
    if not text then return notify(err, vim.log.levels.WARN) end
    local function write(target)
      if not target or target == '' then return end
      local f, open_err = io.open(vim.fn.expand(target), 'w')
      if not f then return notify(open_err, vim.log.levels.ERROR) end
      f:write(text)
      f:close()
      notify(('wrote %d rows to %s'):format(#r.rows, target))
    end
    if path and path ~= '' then return write(path) end
    vim.ui.input({ prompt = 'Export to: ', default = 'result.' .. fmt, completion = 'file' }, write)
  end)
end

attach_results = function(buf)
  if vim.b[buf].tradar_results_attached then return end
  vim.b[buf].tradar_results_attached = true
  local function map(mode, lhs, rhs, desc) vim.keymap.set(mode, lhs, rhs, { buffer = buf, desc = 'tradar: ' .. desc }) end

  map('n', 'gyc', function()
    local row, col = cell_at()
    if not row then return end
    yank(tostring(state.result.rows[row][col] or ''), 'cell')
  end, 'yank cell')
  map('n', 'gyr', function()
    local row = cell_at()
    if not row then return end
    yank(render.tsv(state.result.columns, { state.result.rows[row] }, false), 'row')
  end, 'yank row (tab-separated)')
  map('x', 'gyr', function()
    local a, b = vim.fn.line('v') - 2, vim.fn.line('.') - 2
    if a > b then a, b = b, a end
    local rows = {}
    for i = math.max(a, 1), math.min(b, #(state.result and state.result.rows or {})) do rows[#rows + 1] = state.result.rows[i] end
    vim.api.nvim_feedkeys(vim.keycode('<Esc>'), 'nx', false)
    if #rows > 0 then yank(render.tsv(state.result.columns, rows, false), #rows .. ' rows') end
  end, 'yank selected rows (tab-separated)')
  map('n', 'gyC', function()
    local _, col = cell_at()
    if not col then return end
    local values = {}
    for _, row in ipairs(state.result.rows) do values[#values + 1] = tostring(row[col] or '') end
    yank(table.concat(values, '\n'), 'column ' .. state.result.columns[col])
  end, 'yank column')
  map('n', 'gyj', function() M.yank_all('json') end, 'yank all as JSON')
  map('n', 'gyv', function() M.yank_all('csv') end, 'yank all as CSV')
  map('n', 'gym', function() M.yank_all('md') end, 'yank all as Markdown table')
  map('n', 'q', function()
    local win = vim.fn.bufwinid(buf)
    if win ~= -1 then vim.api.nvim_win_close(win, true) end
  end, 'close results')

  -- Scrolling to the end loads the next page: no key to remember.
  vim.api.nvim_create_autocmd('CursorMoved', {
    buffer = buf,
    callback = function()
      local r = state.result
      if r and r.shown < r.total and vim.api.nvim_win_get_cursor(0)[1] >= vim.api.nvim_buf_line_count(buf) - 3 then
        M.more()
      end
    end,
  })
end

-- ── running queries ──────────────────────────────────────────────────────

local function offset_to_pos(lines, offset)
  -- `offset` is a byte offset into table.concat(lines, '\n').
  local row = 0
  for _, l in ipairs(lines) do
    if offset <= #l then return row, offset end
    offset = offset - #l - 1
    row = row + 1
  end
  return row, 0
end

local function set_diagnostic(buf, base_row, base_col, message, data)
  if not (data and data.line and data.column) then return end
  local lnum = base_row + data.line - 1
  local col = (data.line == 1 and base_col or 0) + data.column
  vim.diagnostic.set(ns, buf, { {
    lnum = lnum, col = col, severity = vim.diagnostic.severity.ERROR,
    message = vim.split(message, '\n', { plain = true })[1], source = 'tradar',
  } })
  if not vim.b[buf].tradar_diag_clear then
    vim.b[buf].tradar_diag_clear = true
    -- Stale squiggles are worse than none: drop them on the next edit.
    vim.api.nvim_create_autocmd({ 'TextChanged', 'InsertEnter' }, {
      buffer = buf,
      once = true,
      callback = function()
        vim.diagnostic.reset(ns, buf)
        vim.b[buf].tradar_diag_clear = false
      end,
    })
  end
end

local function run_text(buf, text, base_row, base_col, retried)
  if state.running then
    return notify('a query is already running — cancel it with <leader>rx', vim.log.levels.WARN)
  end
  local conn = M.connection_for(buf)
  if not conn then
    return M.connect(nil, function() run_text(buf, text, base_row, base_col) end)
  end
  state.sql_buf = buf
  ensure_connected(conn, function()
    state.counter = state.counter + 1
    local id = ('nv%d-%d'):format(vim.fn.getpid(), state.counter)
    state.running = { id = id, conn = conn, started = uv.hrtime() }
    start_spinner()
    local started = uv.hrtime()
    remember(conn, text)
    local function finish()
      state.running = nil
      stop_spinner()
    end
    call('execute', { connection = conn, query = text, page_size = PAGE, query_id = id }, function(r)
      finish()
      local ms = (uv.hrtime() - started) / 1e6
      local took = ms < 1000 and ('%dms'):format(ms) or ('%.1fs'):format(ms / 1000)
      state.last = r.kind == 'affected' and ('%d affected · %s'):format(r.rows, took)
        or ('%d/%d rows · %s'):format(#r.rows, r.total, took)
      if vim.api.nvim_buf_is_valid(buf) then vim.diagnostic.reset(ns, buf) end
      show_result(r)
    end, function(err, data)
      finish()
      if err == 'query cancelled' then
        state.last = 'cancelled'
        return notify('cancelled')
      end
      if err:find('not connected', 1, true) and not retried then
        -- The server restarted since we connected: reconnect once, quietly.
        state.connected[conn] = nil
        return run_text(buf, text, base_row, base_col, true)
      end
      state.last = 'error'
      show_message(err)
      if vim.api.nvim_buf_is_valid(buf) then set_diagnostic(buf, base_row, base_col, err, data) end
    end)
  end, function(err)
    notify(err, vim.log.levels.ERROR)
  end)
end

--- Visual selection / range: runs exactly those lines.
--- No range: the statement under the cursor (boundaries are the driver's,
--- not a regex here). `all`: every statement in the buffer, in order.
function M.run(range_start, range_end, all)
  local buf = vim.api.nvim_get_current_buf()
  if range_start then
    local lines = vim.api.nvim_buf_get_lines(buf, range_start - 1, range_end, false)
    return run_text(buf, table.concat(lines, '\n'), range_start - 1, 0)
  end
  local conn = M.connection_for(buf)
  if not conn then
    return M.connect(nil, function() M.run(nil, nil, all) end)
  end
  local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  local text = table.concat(lines, '\n')
  ensure_connected(conn, function()
    call('split', { connection = conn, text = text }, function(statements)
      if #statements == 0 then return notify('nothing to run', vim.log.levels.WARN) end
      local function position(s)
        local row, col = offset_to_pos(lines, s.start)
        return row, col
      end
      if all then
        -- Sequentially: each run starts when the previous one finished.
        local i = 0
        local function next_statement()
          i = i + 1
          local s = statements[i]
          if not s then return end
          local row, col = position(s)
          run_text(buf, s.text, row, col)
          local timer = uv.new_timer()
          timer:start(50, 50, vim.schedule_wrap(function()
            if not state.running then
              timer:stop()
              timer:close()
              if state.last ~= 'error' and state.last ~= 'cancelled' then next_statement() end
            end
          end))
        end
        return next_statement()
      end
      local row, col = unpack(vim.api.nvim_win_get_cursor(0))
      local offset = col
      for r = 1, row - 1 do offset = offset + #lines[r] + 1 end
      local chosen
      for _, s in ipairs(statements) do
        if offset >= s.start and offset <= s['end'] then chosen = s break end
        if s.start <= offset then chosen = s end -- between statements: the one above
      end
      chosen = chosen or statements[1]
      local srow, scol = position(chosen)
      run_text(buf, chosen.text, srow, scol)
    end)
  end, function(err) notify(err, vim.log.levels.ERROR) end)
end

function M.cancel()
  if not state.running then return notify('nothing is running') end
  call('cancel', { query_id = state.running.id }, function() end)
end

--- Binds the current buffer to a saved connection (picker if `name` empty).
function M.connect(name, after)
  local buf = vim.api.nvim_get_current_buf()
  local function go(chosen)
    vim.b[buf].tradar_connection = chosen
    state.default = chosen
    ensure_connected(chosen, function()
      call('schema', { connection = chosen }, function(entries)
        notify(('connected to %s (%d objects)'):format(chosen, #entries))
        if after then after() end
      end)
    end, function(err) notify(err, vim.log.levels.ERROR) end)
  end
  if name and name ~= '' then return go(name) end
  call('connections.list', nil, function(list)
    local usable = vim.tbl_filter(function(c) return c.supported end, list)
    if #usable == 0 then return notify('no saved connection this server can serve', vim.log.levels.WARN) end
    vim.ui.select(usable, {
      prompt = 'tradar connection',
      format_item = function(c) return ('%s  (%s)%s'):format(c.name, c.driver, c.connected and '  ●' or '') end,
    }, function(c) if c then go(c.name) end end)
  end)
end

-- ── navigation helpers (also used by the telescope pickers) ──────────────

--- `cb(entries, buf)` for the current buffer's connection.
function M.schema_entries(cb)
  local buf = vim.api.nvim_get_current_buf()
  local conn = M.connection_for(buf)
  if not conn then
    return M.connect(nil, function() M.schema_entries(cb) end)
  end
  ensure_connected(conn, function()
    call('schema', { connection = conn }, function(entries) cb(entries, buf, conn) end)
  end, function(err) notify(err, vim.log.levels.ERROR) end)
end

function M.insert_text(buf, text)
  local win = vim.fn.bufwinid(buf)
  if win == -1 then return notify('the SQL window is closed', vim.log.levels.WARN) end
  vim.api.nvim_set_current_win(win)
  vim.api.nvim_put({ text }, 'c', false, true)
end

--- Generates the driver's own "show me this table" statement and runs it.
function M.open_table(buf, entry)
  local conn = M.connection_for(buf)
  if not conn then return end
  call('snippet', { connection = conn, name = entry.name, schema = entry.schema, op = 'read' }, function(r)
    if not r.text then return notify('this connector has no snippet for that', vim.log.levels.WARN) end
    run_text(buf, r.text, 0, 0)
  end)
end

--- Navigator: tables and columns of the buffer's connection; <CR> inserts
--- the name under the cursor into the SQL buffer you came from.
function M.schema()
  M.schema_entries(function(entries, sql_buf)
    local lines, targets = render.schema(entries)
    local buf = scratch('tradar://schema', state.schema_buf)
    state.schema_buf, state.schema_targets, state.sql_buf = buf, targets, sql_buf
    set_lines(buf, lines)
    vim.keymap.set('n', '<CR>', function()
      local name = state.schema_targets[vim.api.nvim_win_get_cursor(0)[1]]
      if name and state.sql_buf and vim.api.nvim_buf_is_valid(state.sql_buf) then M.insert_text(state.sql_buf, name) end
    end, { buffer = buf, desc = 'tradar: insert name into the SQL buffer' })
    vim.keymap.set('n', 'q', '<cmd>close<CR>', { buffer = buf, desc = 'close' })
    if vim.fn.bufwinid(buf) == -1 then
      vim.cmd('topleft 40vsplit')
      vim.api.nvim_win_set_buf(0, buf)
      vim.wo.wrap = false
    end
  end)
end

-- ── completion ───────────────────────────────────────────────────────────

--- `cb(items)` with `{text, kind}` for the text before the cursor. Used by
--- the blink.cmp source; async, so typing never waits on it.
function M.complete(buf, text, cb)
  local conn = M.connection_for(buf)
  if not conn or not state.connected[conn] then return cb({}) end
  rpc.request('complete', { connection = conn, text = text }, function(err, result)
    cb((not err and result) and result.items or {})
  end)
end

--- Built-in `omnifunc` fallback for setups without blink.cmp.
function M.omnifunc(findstart, base)
  local row, col = unpack(vim.api.nvim_win_get_cursor(0))
  if findstart == 1 then
    local line = vim.api.nvim_get_current_line():sub(1, col)
    return col - #line:match('[%w_$]*$')
  end
  local conn = M.connection_for(0)
  if not conn or not state.connected[conn] then return {} end
  local lines = vim.api.nvim_buf_get_lines(0, 0, row, false)
  lines[#lines] = lines[#lines]:sub(1, col)
  local err, result = rpc.request_sync('complete', { connection = conn, text = table.concat(lines, '\n') })
  if err or not result then return {} end
  local out = {}
  for _, item in ipairs(result.items) do out[#out + 1] = { word = item.text, menu = '[' .. item.kind .. ']' } end
  return out
end

-- ── buffer setup ─────────────────────────────────────────────────────────

--- Called for every SQL buffer: keymaps, omnifunc, and a quiet background
--- connect so the first completion/run has nothing to wait for.
function M.attach(buf)
  buf = resolve_buf(buf)
  vim.bo[buf].omnifunc = "v:lua.require'tradar'.omnifunc"
  if opts.keymaps ~= false then
    local p = opts.prefix or '<leader>r'
    local function map(mode, suffix, rhs, desc)
      vim.keymap.set(mode, p .. suffix, rhs, { buffer = buf, desc = 'tradar: ' .. desc })
    end
    map('n', 'r', function() M.run() end, 'run statement under cursor')
    map('x', 'r', function()
      local a, b = vim.fn.line('v'), vim.fn.line('.')
      vim.api.nvim_feedkeys(vim.keycode('<Esc>'), 'nx', false)
      M.run(math.min(a, b), math.max(a, b))
    end, 'run selection')
    map('n', 'a', function() M.run(nil, nil, true) end, 'run all statements')
    map('n', 'x', M.cancel, 'cancel running query')
    map('n', 'c', function() M.connect() end, 'choose connection')
    map('n', 's', M.schema, 'schema panel')
    map('n', 't', function() require('tradar.telescope').tables() end, 'tables (telescope)')
    map('n', 'h', function() require('tradar.telescope').history() end, 'history (telescope)')
    map('n', 'm', function() M.more() end, 'load more rows')
  end
  local conn = M.connection_for(buf)
  if conn and not state.connected[conn] then
    ensure_connected(conn, function() end, function(err)
      notify(('could not connect `%s`: %s'):format(conn, err), vim.log.levels.WARN)
    end)
  end
end

function M.setup(user)
  opts = user or {}
  local ok, wk = pcall(require, 'which-key')
  if ok and wk.add then wk.add({ { opts.prefix or '<leader>r', group = 'database (tradar)' } }) end
  if vim.bo.filetype == 'sql' then M.attach(0) end
end

return M
