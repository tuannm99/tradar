-- tradar.nvim: the editor is a real Neovim buffer, tradar-server does the
-- querying. See "Server headless" in docs/architecture.md.
local rpc = require('tradar.rpc')
local render = require('tradar.render')
local guard = require('tradar.guard')
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
  schema_entries = nil, -- cached so a fold toggle can re-render without another `schema` call
  schema_nodes = {}, -- parallel to the schema buffer's lines, from the last render
  -- Keyed by a node's own key (see `render.schema_tree`), not by connection
  -- -- the schema buffer is shared across connections (same name, reused),
  -- so switching connections can coincidentally show a same-named table
  -- pre-expanded from a previous one. Cosmetic only, not worth a
  -- per-connection key for.
  schema_expanded = {},
  sql_buf = nil, -- buffer the navigator inserts into
  drivers = nil, -- connection name -> driver id (from connections.list)
  schema_cache = {}, -- name -> { at, entries }
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

--- `cb(driver_id)` for a connection ("sqlite", "postgres", ...); cached.
local function driver_of(conn, cb)
  if state.drivers then return cb(state.drivers[conn]) end
  call('connections.list', nil, function(list)
    state.drivers = {}
    for _, c in ipairs(list) do state.drivers[c.name] = c.driver end
    cb(state.drivers[conn])
  end)
end

--- Whether the server has confirmed a connection (not merely a binding).
function M.is_connected(name) return state.connected[name] == true end

--- Connections whose name matches `setup{ protected = {...} }` (default
--- `{"prod"}`): every write asks first, and the statusline shows `⚠`.
function M.is_protected(name)
  return guard.is_protected(name, opts.protected or { 'prod' })
end

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
  local in_tradar = vim.b.tradar_attached or vim.api.nvim_buf_get_name(0):find('^tradar://') ~= nil
  if not in_tradar and not state.running then return '' end
  local conn = (state.running and state.running.conn) or M.connection_for(0)
  if not conn then return '' end
  local parts = { 'db ' .. conn .. (M.is_protected(conn) and ' ⚠' or '') }
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

--- The grid behind a result as `columns, rows` of strings -- a SQL table, or
--- documents shown as a table -- or nil when it is raw JSON (or not rows).
local function tabular(r)
  if not r then return nil end
  if r.kind == 'table' then return r.columns, r.rows end
  if r.kind == 'documents' and r.view == 'table' then return r.tcols, r.trows end
end

local attach_results

local function paint_results()
  local r = state.result
  local buf = state.results_buf
  local lines
  if r.kind == 'table' then
    lines, r.spans = render.table(r.columns, r.rows)
  elseif r.view == 'table' then
    r.tcols, r.trows = render.flatten(r.rows, r.field_order)
    lines, r.spans = render.table(r.tcols, r.trows)
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

--- Documents are a table when they are *objects* (Mongo, Elasticsearch);
--- plain replies (a Redis string, a list of numbers) stay as JSON lines --
--- a one-column table of `value` adds nothing. `setup{ documents_view }`
--- forces either.
local function default_view(items)
  if opts.documents_view then return opts.documents_view end
  for _, item in ipairs(items) do
    if type(item) == 'table' and not vim.islist(item) then return 'table' end
  end
  return 'json'
end

local function show_result(r, meta)
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
    -- One entry per row in `rows`, parallel to it (see `M.more`, which
    -- extends both in lockstep as pages load) -- `render.flatten`'s own
    -- `order` parameter, `nil` for anything but a `documents` result from
    -- a server new enough to send it.
    field_order = r.field_order,
    view = r.kind == 'documents' and default_view(r.rows) or nil,
    conn = meta and meta.conn, query = meta and meta.query,
  }
  paint_results()
  local win = show(buf)
  pcall(vim.api.nvim_win_set_cursor, win, { 1, 0 })
end

--- `gT`: documents as a table (editable, flat columns) or raw JSON.
function M.toggle_view()
  local r = state.result
  if not r or r.kind ~= 'documents' then return notify('only document results have two views') end
  r.view = r.view == 'table' and 'json' or 'table'
  paint_results()
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
    if r.field_order and page.field_order then vim.list_extend(r.field_order, page.field_order) end
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
  local cols, rows = tabular(r)
  if not cols then return nil end
  local row = vim.api.nvim_win_get_cursor(0)[1] - 2
  if row < 1 or row > #rows then return nil end
  return row, render.column_at(r.spans, vim.fn.virtcol('.'))
end

local function export_text(fmt, r)
  local columns, rows = r.columns, r.rows
  if r.kind == 'documents' then
    -- Documents keep their real structure as JSON; the flat formats use the
    -- same flattened columns as the table view.
    if fmt == 'json' then return vim.json.encode(r.rows) .. '\n' end
    columns, rows = render.flatten(r.rows, r.field_order)
  end
  if fmt == 'csv' then return render.csv(columns, rows) end
  if fmt == 'json' then return render.json(columns, rows) end
  if fmt == 'md' or fmt == 'markdown' then return render.markdown(columns, rows) end
  if fmt == 'tsv' then return render.tsv(columns, rows, true) .. '\n' end
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
    local _, rows = tabular(state.result)
    yank(tostring(rows[row][col] or ''), 'cell')
  end, 'yank cell')
  map('n', 'gyr', function()
    local row = cell_at()
    if not row then return end
    local cols, rows = tabular(state.result)
    yank(render.tsv(cols, { rows[row] }, false), 'row')
  end, 'yank row (tab-separated)')
  map('x', 'gyr', function()
    local a, b = vim.fn.line('v') - 2, vim.fn.line('.') - 2
    if a > b then a, b = b, a end
    local cols, all = tabular(state.result)
    local rows = {}
    for i = math.max(a, 1), math.min(b, #(all or {})) do rows[#rows + 1] = all[i] end
    vim.api.nvim_feedkeys(vim.keycode('<Esc>'), 'nx', false)
    if #rows > 0 then yank(render.tsv(cols, rows, false), #rows .. ' rows') end
  end, 'yank selected rows (tab-separated)')
  map('n', 'gyC', function()
    local _, col = cell_at()
    if not col then return end
    local cols, rows = tabular(state.result)
    local values = {}
    for _, row in ipairs(rows) do values[#values + 1] = tostring(row[col] or '') end
    yank(table.concat(values, '\n'), 'column ' .. cols[col])
  end, 'yank column')
  map('n', 'gyj', function() M.yank_all('json') end, 'yank all as JSON')
  map('n', 'gyv', function() M.yank_all('csv') end, 'yank all as CSV')
  map('n', 'gym', function() M.yank_all('md') end, 'yank all as Markdown table')
  map('n', 'gT', function() M.toggle_view() end, 'documents: table <-> JSON')
  map('n', 'gd', function() M.follow_fk() end, 'follow foreign key under cursor')
  map('n', 'i', function() M.edit_cell() end, 'edit this cell (shows the UPDATE first)')
  map('n', 'dd', function() M.delete_row() end, 'delete this row (shows the DELETE first)')
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

local function run_text(buf, text, base_row, base_col, flags)
  flags = flags or {}
  if state.running then
    return notify('a query is already running — cancel it with <leader>rx', vim.log.levels.WARN)
  end
  local conn = M.connection_for(buf)
  if not conn then
    return M.connect(nil, function() run_text(buf, text, base_row, base_col) end)
  end
  state.sql_buf = buf

  if not flags.confirmed and opts.confirm ~= false then
    local confirmed_flags = vim.tbl_extend('force', flags, { confirmed = true })
    local protected = M.is_protected(conn)
    return driver_of(conn, function(driver)
      local reason = guard.assess(text, protected, driver)
      if not reason then return run_text(buf, text, base_row, base_col, confirmed_flags) end
      local first = guard.first_code_line(text)
      -- "Cancel" is first so a reflexive <CR> is the safe answer.
      vim.ui.select({ 'Cancel', 'Run anyway' }, {
        prompt = ('tradar [%s]: %s\n  %s'):format(conn, reason, first:sub(1, 70)),
      }, function(choice)
        if choice == 'Run anyway' then run_text(buf, text, base_row, base_col, confirmed_flags) end
      end)
    end)
  end

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
      if guard.changes_schema(text) then state.schema_cache[conn] = nil end
      show_result(r, { conn = conn, query = text })
      if flags.after then flags.after() end
    end, function(err, data)
      finish()
      if err == 'query cancelled' then
        state.last = 'cancelled'
        return notify('cancelled')
      end
      if err:find('not connected', 1, true) and not flags.retried then
        -- The server restarted since we connected: reconnect once, quietly.
        state.connected[conn] = nil
        return run_text(buf, text, base_row, base_col, vim.tbl_extend('force', flags, { retried = true }))
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
function M.run(range_start, range_end, all, transform)
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
      -- The binding comment (`-- tradar: x`) can come back as a statement of
      -- its own; it is not something to run.
      statements = vim.tbl_filter(function(st) return not guard.is_comment_only(st.text) end, statements)
      if #statements == 0 then return notify('nothing to run', vim.log.levels.WARN) end
      local function position(s)
        local row, col = offset_to_pos(lines, s.start)
        return row, col
      end
      if all then
        local function go()
          -- Sequentially: each run starts when the previous one finished,
          -- and the batch stops at the first error or cancel.
          local i = 0
          local function next_statement()
            i = i + 1
            local st = statements[i]
            if not st then return end
            local row, col = position(st)
            run_text(buf, st.text, row, col, { confirmed = true })
            local timer = uv.new_timer()
            timer:start(50, 50, vim.schedule_wrap(function()
              if not state.running then
                timer:stop()
                timer:close()
                if state.last ~= 'error' and state.last ~= 'cancelled' then next_statement() end
              end
            end))
          end
          next_statement()
        end

        -- One question for the whole batch, not one per statement. The
        -- driver is needed first: Mongo/Redis/Elasticsearch have their own
        -- notion of "risky", and the lookup is async the first time.
        driver_of(conn, function(driver)
          local risky = {}
          for _, st in ipairs(statements) do
            local why = opts.confirm ~= false and guard.assess(st.text, M.is_protected(conn), driver)
            if why then risky[#risky + 1] = why end
          end
          if #risky == 0 then return go() end
          vim.ui.select({ 'Cancel', 'Run all' }, {
            prompt = ('tradar [%s]: %d of %d statements need a second look\n  %s'):format(
              conn, #risky, #statements, table.concat(risky, '; '):sub(1, 90)),
          }, function(choice) if choice == 'Run all' then go() end end)
        end)
        return
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
      run_text(buf, transform and transform(chosen.text) or chosen.text, srow, scol)
    end)
  end, function(err) notify(err, vim.log.levels.ERROR) end)
end

--- Stops the background server (it restarts itself on the next command).
--- For picking up a rebuilt binary: a long-running server keeps the old one.
function M.restart()
  if not rpc.connected() then return notify('no server is running; the next command starts a fresh one') end
  rpc.request('shutdown', nil, function()
    rpc.close()
    state.connected, state.connecting, state.drivers, state.schema_cache = {}, {}, nil, {}
    notify('server stopped; the next command starts a fresh one')
  end)
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

--- Whether `node` (a schema or table node from `render.schema_tree`) is
--- currently open, by the exact same rule `schema_tree` itself uses to
--- decide -- a schema folder defaults open, a table defaults closed (see
--- `render.schema_tree`'s own doc comment for why).
local function node_is_open(node)
  local current = state.schema_expanded[node.key]
  if current ~= nil then return current end
  return node.kind == 'schema'
end

--- Navigator: a tree of schema/keyspace/database groups (when the driver
--- has the concept), then tables, then columns. `<Tab>` expands/collapses
--- the schema or table under the cursor; `<CR>` inserts a table/column
--- name into the SQL buffer you came from, or -- on a schema header, which
--- has no name worth inserting -- also toggles.
function M.schema()
  M.schema_entries(function(entries, sql_buf)
    local buf = scratch('tradar://schema', state.schema_buf)
    state.schema_buf, state.schema_entries, state.sql_buf = buf, entries, sql_buf

    local function redraw()
      local win = vim.fn.bufwinid(buf)
      local cursor = win ~= -1 and vim.api.nvim_win_get_cursor(win) or nil
      local lines, nodes = render.schema_tree(state.schema_entries, state.schema_expanded)
      state.schema_nodes = nodes
      set_lines(buf, lines)
      if cursor and win ~= -1 then
        vim.api.nvim_win_set_cursor(win, { math.min(cursor[1], math.max(#lines, 1)), cursor[2] })
      end
    end
    redraw()

    local function node_at_cursor() return state.schema_nodes[vim.api.nvim_win_get_cursor(0)[1]] end
    local function toggle(node)
      state.schema_expanded[node.key] = not node_is_open(node)
      redraw()
    end
    vim.keymap.set('n', '<CR>', function()
      local node = node_at_cursor()
      if not node then return end
      if node.insert then
        if state.sql_buf and vim.api.nvim_buf_is_valid(state.sql_buf) then M.insert_text(state.sql_buf, node.insert) end
      elseif node.key then
        toggle(node)
      end
    end, { buffer = buf, desc = 'tradar: insert name into the SQL buffer, or toggle a schema folder' })
    vim.keymap.set('n', '<Tab>', function()
      local node = node_at_cursor()
      if node and node.key then toggle(node) end
    end, { buffer = buf, desc = 'tradar: expand/collapse the schema or table under the cursor' })
    vim.keymap.set('n', 'q', '<cmd>close<CR>', { buffer = buf, desc = 'close' })
    if vim.fn.bufwinid(buf) == -1 then
      vim.cmd('topleft 40vsplit')
      vim.api.nvim_win_set_buf(0, buf)
      vim.wo.wrap = false
    end
  end)
end

-- ── schema-aware navigation: hover, go-to, follow a foreign key ──────────

--- Schema for `conn`, cached for a minute and dropped when a DDL statement
--- runs from here -- hover must feel instant, and schemas rarely move.
local function get_schema(conn, cb)
  local hit = state.schema_cache[conn]
  if hit and (uv.hrtime() - hit.at) < 60e9 then return cb(hit.entries) end
  ensure_connected(conn, function()
    call('schema', { connection = conn }, function(entries)
      state.schema_cache[conn] = { at = uv.hrtime(), entries = entries }
      cb(entries)
    end)
  end, function(err) notify(err, vim.log.levels.ERROR) end)
end

--- `public."Users"` -> `users` (last segment, unquoted, lowercase): how a
--- table is compared across the ways a driver and a user may spell it.
local function bare(name)
  local last = tostring(name):match('([^.]+)$') or tostring(name)
  return (last:gsub('^[`"%[]', ''):gsub('[`"%]]$', '')):lower()
end

local function find_table(entries, name)
  local want = bare(name)
  for _, e in ipairs(entries) do
    if e.name:lower() == want or bare(e.name) == want then return e end
  end
end

local function quote_ident(name, driver)
  local q = (driver == 'mysql' or driver == 'clickhouse') and '`' or '"'
  local parts = {}
  for part in tostring(name):gmatch('[^.]+') do
    if part:match('^[a-z_][a-z0-9_]*$') then
      parts[#parts + 1] = part
    else
      parts[#parts + 1] = q .. part:gsub(q, q .. q) .. q
    end
  end
  return table.concat(parts, '.')
end

--- Always a string literal: Postgres, SQLite and MySQL all coerce `'1'` to
--- a number where the column is numeric, so one shape covers every type
--- (the same choice the TUI's row edit makes).
local function quote_literal(value, driver)
  local v = tostring(value):gsub("'", "''")
  if driver == 'mysql' then v = v:gsub('\\', '\\\\') end
  return "'" .. v .. "'"
end

local function word_under_cursor()
  local line = vim.api.nvim_get_current_line()
  local col = vim.api.nvim_win_get_cursor(0)[2] + 1
  local s, e = col, col
  while s > 1 and line:sub(s - 1, s - 1):match('[%w_$]') do s = s - 1 end
  while e <= #line and line:sub(e, e):match('[%w_$]') do e = e + 1 end
  local word = line:sub(s, e - 1)
  if word == '' then return nil end
  local qualifier = line:sub(1, s - 1):match('([%w_$]+)%.$')
  return word, qualifier
end

--- The table an alias stands for, read from `FROM x a` / `FROM x AS a` /
--- `JOIN x a` anywhere in `text`. Text-level, like the TUI's own alias
--- resolution -- and just as happy to be wrong about a pathological query,
--- since the worst outcome is a hover that lists every table with that column.
local STOP = { on = true, where = true, join = true, inner = true, left = true, right = true, full = true,
  outer = true, cross = true, group = true, order = true, limit = true, having = true, union = true, using = true,
  set = true, values = true, select = true, ['and'] = true, ['or'] = true }

local function resolve_alias(text, alias)
  local lower, want = text:lower(), alias:lower()
  for _, kw in ipairs({ 'from', 'join' }) do
    for tbl, rest in lower:gmatch('%f[%w_]' .. kw .. '%s+([%w_."`]+)%s+([%w_]+)') do
      local word = rest
      if word == 'as' then
        -- `x AS a`: the alias is the word after AS, which this pass didn't capture
        local after = lower:match('%f[%w_]' .. kw .. '%s+' .. vim.pesc(tbl) .. '%s+as%s+([%w_]+)')
        word = after or word
      end
      if word == want and not STOP[word] then return tbl end
    end
  end
end

local function fallback(name)
  if name == 'hover' then vim.lsp.buf.hover() else vim.lsp.buf.definition() end
end

--- `K`: what the schema knows about the table or column under the cursor,
--- in a float. Falls back to the LSP's own hover when it knows nothing.
function M.hover()
  local conn = M.connection_for(0)
  local word, qualifier = word_under_cursor()
  if not (conn and word) then return fallback('hover') end
  get_schema(conn, function(entries)
    local lines
    local tbl = find_table(entries, word)
    if tbl then
      lines = render.entry(tbl)
    else
      local want = word:lower()
      local qual
      if qualifier then
        local buf_text = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), '\n')
        local aliased = resolve_alias(buf_text, qualifier)
        if aliased then
          qual = bare(aliased)
        elseif find_table(entries, qualifier) then
          qual = bare(qualifier)
        end -- else: unknown qualifier, don't restrict
      end
      lines = {}
      for _, e in ipairs(entries) do
        if not qual or bare(e.name) == qual then
          for _, c in ipairs(e.columns or {}) do
            if c.name:lower() == want then
              local marks = (c.primary_key and '  pk' or '') .. (c.indexed and '  idx' or '')
              local fk = c.foreign_key and ('  → ' .. c.foreign_key.table .. '.' .. c.foreign_key.column) or ''
              lines[#lines + 1] = ('%s.%s  %s%s%s'):format(e.name, c.name, c.type, marks, fk)
            end
          end
        end
      end
      if #lines == 0 then lines = nil end
    end
    if not lines then return fallback('hover') end
    vim.lsp.util.open_floating_preview(lines, 'text', { border = 'rounded', focus_id = 'tradar_hover', max_width = 90 })
  end)
end

--- `gd` on a table name: show its rows. Anything else: the LSP's own.
function M.goto_table()
  local buf = vim.api.nvim_get_current_buf()
  local conn = M.connection_for(buf)
  local word = word_under_cursor()
  if not (conn and word) then return fallback('definition') end
  get_schema(conn, function(entries)
    local tbl = find_table(entries, word)
    if tbl then M.open_table(buf, tbl) else fallback('definition') end
  end)
end

--- In the results buffer, `gd` on a cell of a foreign-key column runs the
--- `SELECT` for the row it points at. Works for a single-table query (the
--- same limit the TUI's row edit has: the driver must be able to say which
--- table the rows come from) and, since this only needs to name *this one
--- column*'s table rather than a whole row's, also for a `JOIN` result
--- when that column was explicitly qualified in the SELECT list
--- (`alias.column`/`table.column`) -- `edit.source`'s `column_sources`,
--- parallel to `cols`, carries that per column when `src.table` alone
--- can't answer for the whole query.
function M.follow_fk()
  local r = state.result
  local row, col = cell_at()
  if not (r and r.conn and row) then return notify('put the cursor on a cell of a table result', vim.log.levels.WARN) end
  local cols, rows = tabular(r)
  local name, value = cols[col], rows[row][col]
  call('edit.source', { connection = r.conn, query = r.query, columns = cols }, function(src)
    local table_name = src.table
    if not table_name and src.column_sources then
      local source = src.column_sources[col]
      if source ~= vim.NIL then table_name = source end
    end
    if not table_name then
      return notify(
        'cannot tell which table this column comes from (a single-table SELECT, or a JOIN with `alias.column` explicitly in the SELECT list)',
        vim.log.levels.WARN)
    end
    get_schema(r.conn, function(entries)
      local tbl = find_table(entries, table_name)
      local column
      for _, c in ipairs(tbl and tbl.columns or {}) do
        if c.name:lower() == name:lower() then column = c break end
      end
      if not (column and column.foreign_key) then
        return notify(('`%s` is not a foreign key'):format(name), vim.log.levels.WARN)
      end
      if value == nil or value == 'NULL' then return notify('NULL references nothing', vim.log.levels.WARN) end
      local fk = column.foreign_key
      driver_of(r.conn, function(driver)
        local sql = ('SELECT * FROM %s WHERE %s = %s LIMIT 100;'):format(
          quote_ident(fk.table, driver), quote_ident(fk.column, driver), quote_literal(value, driver))
        local buf = state.sql_buf
        if not (buf and vim.api.nvim_buf_is_valid(buf)) then buf = vim.api.nvim_get_current_buf() end
        run_text(buf, sql, 0, 0, { confirmed = true })
      end)
    end)
  end, function(err) notify(err, vim.log.levels.ERROR) end)
end

-- ── editing rows from the results buffer ─────────────────────────────────

--- Works out what the cursor's row *is*: the table it came from and the
--- primary-key values that name exactly that row. Read-only (with a reason)
--- when it can't be done -- the same rule as the TUI's row edit: a statement
--- that might hit several rows is not one to write on the user's behalf.
local function edit_target(cb)
  local r = state.result
  local row, col = cell_at()
  if not (r and r.conn and row) then
    return notify('put the cursor on a cell of a table result', vim.log.levels.WARN)
  end
  call('edit.source', { connection = r.conn, query = r.query }, function(src)
    if not src.table then
      return notify('read-only: cannot tell which table these rows come from (single-table SELECTs only)', vim.log.levels.WARN)
    end
    local function with_keys(keys)
      if not keys or #keys == 0 then
        return notify(('read-only: `%s` has no primary key, so a row cannot be addressed'):format(src.table), vim.log.levels.WARN)
      end
      local key = {}
      local cols, rows = tabular(r)
      for _, k in ipairs(keys) do
        local idx
        for i, c in ipairs(cols) do
          if c:lower() == k:lower() then idx = i break end
        end
        if not idx then
          return notify(('the key column `%s` is not in this result — select it too'):format(k), vim.log.levels.WARN)
        end
        local v = rows[row][idx]
        if v == nil or v == 'NULL' then
          return notify(('cannot address the row: key `%s` is NULL'):format(k), vim.log.levels.WARN)
        end
        key[k] = v
      end
      cb(r, row, col, src, key)
    end
    if src.key_columns and #src.key_columns > 0 then return with_keys(src.key_columns) end
    get_schema(r.conn, function(entries)
      local tbl = find_table(entries, src.table)
      local keys = {}
      for _, c in ipairs(tbl and tbl.columns or {}) do
        if c.primary_key then keys[#keys + 1] = c.name end
      end
      with_keys(keys)
    end)
  end, function(err) notify(err, vim.log.levels.ERROR) end)
end

--- Shows the generated statement and runs it only on an explicit "Run";
--- then re-runs the original query so the grid shows the change, with the
--- cursor where it was.
local function confirm_and_run(r, sql)
  if not sql then return notify('this connector cannot edit rows (read-only)', vim.log.levels.WARN) end
  local win = vim.fn.bufwinid(state.results_buf)
  local pos = win ~= -1 and vim.api.nvim_win_get_cursor(win) or nil
  local buf = state.sql_buf
  if not (buf and vim.api.nvim_buf_is_valid(buf)) then buf = vim.api.nvim_get_current_buf() end
  vim.ui.select({ 'Cancel', 'Run' }, {
    prompt = ('tradar [%s] run this?\n  %s'):format(r.conn, sql),
  }, function(choice)
    if choice ~= 'Run' then return end
    run_text(buf, sql, 0, 0, {
      confirmed = true, -- this prompt *is* the confirmation
      after = function()
        run_text(buf, r.query, 0, 0, {
          confirmed = true,
          after = function()
            local w = vim.fn.bufwinid(state.results_buf)
            if w ~= -1 and pos then
              local lines = vim.api.nvim_buf_line_count(state.results_buf)
              pcall(vim.api.nvim_win_set_cursor, w, { math.min(pos[1], lines), pos[2] })
            end
          end,
        })
      end,
    })
  end)
end

--- `i` on a cell: type the new value (the literal `NULL` sets NULL, as the
--- grid shows it), see the `UPDATE`, confirm.
function M.edit_cell()
  edit_target(function(r, row, col, src, key)
    local cols, rows = tabular(r)
    local column, current = cols[col], rows[row][col]
    vim.ui.input({ prompt = ('%s.%s = '):format(src.table, column), default = current }, function(value)
      if value == nil then return end
      if value == current then return notify('unchanged') end
      call('edit.sql', {
        connection = r.conn, table = src.table, key = key,
        change = { set = { column = column, value = value } },
      }, function(res) confirm_and_run(r, res.sql) end)
    end)
  end)
end

--- `dd` on a row: the `DELETE` for exactly that row, shown, confirmed.
function M.delete_row()
  edit_target(function(r, _, _, src, key)
    call('edit.sql', { connection = r.conn, table = src.table, key = key, change = 'delete' },
      function(res) confirm_and_run(r, res.sql) end)
  end)
end

--- Plans the statement under the cursor (`EXPLAIN`, or SQLite's
--- `EXPLAIN QUERY PLAN`). Never `ANALYZE`: that would *execute* it.
function M.explain()
  local conn = M.connection_for(0)
  if not conn then return M.connect(nil, M.explain) end
  driver_of(conn, function(driver)
    -- `EXPLAIN <statement>` is SQL; Mongo/Redis/Elasticsearch have no such prefix.
    local prefixes = {
      postgres = 'EXPLAIN ', mysql = 'EXPLAIN ', clickhouse = 'EXPLAIN ', sqlite = 'EXPLAIN QUERY PLAN ',
    }
    local prefix = prefixes[driver]
    if not prefix then
      return notify(('EXPLAIN is only available for SQL connections (this one is %s)'):format(driver or 'unknown'), vim.log.levels.WARN)
    end
    M.run(nil, nil, false, function(text) return prefix .. text end)
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

--- Called for every SQL buffer (and any other buffer that names a connection
--- in a `-- tradar: name` / `// tradar: name` / `# tradar: name` line): keymaps, omnifunc, and a quiet background
--- connect so the first completion/run has nothing to wait for.
function M.attach(buf)
  buf = resolve_buf(buf)
  vim.b[buf].tradar_attached = true
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
    map('n', 'e', M.explain, 'explain statement under cursor')
  end
  M.attach_nav(buf)
  local conn = M.connection_for(buf)
  if conn and not state.connected[conn] then
    ensure_connected(conn, function() end, function(err)
      notify(('could not connect `%s`: %s'):format(conn, err), vim.log.levels.WARN)
    end)
  end
end

--- `K`/`gd` on SQL buffers. Split from `attach` because an LSP's own
--- `on_attach` (sqlls, here) sets the same keys after FileType and would
--- win: the LspAttach hook in plugin/tradar.lua calls this again afterwards.
function M.attach_nav(buf)
  buf = resolve_buf(buf)
  if opts.keymaps == false then return end
  vim.keymap.set('n', 'K', M.hover, { buffer = buf, desc = 'tradar: table/column info' })
  vim.keymap.set('n', 'gd', M.goto_table, { buffer = buf, desc = 'tradar: open table under cursor' })
end

--- Attaches `buf` if it is SQL or declares a connection in its first lines --
--- how a Mongo (`.mongo`), Redis, Elasticsearch or any other file opts in.
function M.maybe_attach(buf)
  buf = resolve_buf(buf)
  if vim.b[buf].tradar_attached then return end
  if vim.bo[buf].filetype == 'sql' or modeline(buf) then M.attach(buf) end
end

function M.setup(user)
  opts = user or {}
  local ok, wk = pcall(require, 'which-key')
  if ok and wk.add then wk.add({ { opts.prefix or '<leader>r', group = 'database (tradar)' } }) end
  M.maybe_attach(0)
end

return M
