local ctx = require('tradar.ctx')
local M = ctx.M
local rpc = ctx.rpc
local render = ctx.render
local opts = ctx.opts
local DEFAULT_PAGE = ctx.DEFAULT_PAGE
local state = ctx.state
local notify = ctx.notify
local call = ctx.call
local slots = require('tradar.slots')
local gridview = require('tradar.gridview')
local rdiff = require('tradar.rdiff')
local jsonhl = require('tradar.jsonhl')

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

--- The window showing results -- whichever slot (or message) is in it.
local function results_win()
  for _, w in ipairs(vim.api.nvim_tabpage_list_wins(0)) do
    local b = vim.api.nvim_win_get_buf(w)
    if vim.b[b].tradar_results_attached then return w end
  end
end

--- The slot tabs on the results window (see `slots.bar`); while a query runs
--- `conn.lua` owns the winbar and puts these back when it is done.
local function update_bar()
  local win = results_win()
  if win and not state.running then vim.wo[win].winbar = slots.bar(state.result_log, state.result_pos) end
end

local function show(buf, height)
  local win = results_win()
  if win then
    -- A thin strip (the "running…" note, a one-line message) is replaced by
    -- the real layout once a full result arrives, instead of staying 4 rows.
    if height == nil and vim.w[win].tradar_strip and #vim.api.nvim_tabpage_list_wins(0) > 1 then
      vim.api.nvim_win_close(win, true)
      win = nil
    else
      if vim.api.nvim_win_get_buf(win) ~= buf then
        vim.api.nvim_win_set_buf(win, buf)
        vim.wo[win].wrap = false
        vim.wo[win].cursorline = true
      end
      update_bar()
      return win
    end
  end
  local origin = vim.api.nvim_get_current_win()
  -- A full result follows `results_layout`; the short one-line messages
  -- (`height` given) always take a thin strip at the bottom.
  local layout = (height == nil and buf == state.results_buf) and opts.results_layout or 'bottom'
  if layout == 'right' then
    vim.cmd(('botright %dvsplit'):format(opts.results_width or math.floor(vim.o.columns * 0.5)))
  elseif layout == 'tab' then
    vim.cmd('tabnew')
  else
    vim.cmd(('botright %dsplit'):format(height or opts.results_height or 15))
  end
  win = vim.api.nvim_get_current_win()
  vim.api.nvim_win_set_buf(win, buf)
  vim.wo[win].wrap = false
  vim.wo[win].cursorline = true
  vim.w[win].tradar_strip = height ~= nil or nil
  -- Results are for looking at: keep the editor focused, as an IDE would.
  vim.api.nvim_set_current_win(origin)
  update_bar()
  return win
end

ctx.slots_bar = function() return slots.bar(state.result_log, state.result_pos) end

local function set_lines(buf, lines)
  vim.bo[buf].modifiable = true
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
  vim.bo[buf].modifiable = false
end

--- `850ms` / `1.4s` / `2m 5s` -- how long a query took.
local function took_text(ms)
  if ms < 1000 then return ('%dms'):format(math.floor(ms + 0.5)) end
  if ms < 60000 then return ('%.1fs'):format(ms / 1000) end
  local s = math.floor(ms / 1000 + 0.5)
  return ('%dm %ds'):format(math.floor(s / 60), s % 60)
end

--- `[2/3] select * from p · ` when the results of this run are kept (a
--- `run all` of several statements, or just several runs): which one this is.
local function position_text(entry)
  local log, pos = state.result_log, state.result_pos
  if not log or #log < 2 or not pos or log[pos] ~= entry then return '' end
  local q = (entry.query or ''):gsub('%s+', ' ')
  if #q > 40 then q = q:sub(1, 39) .. '…' end
  return ('[%d/%d] %s · '):format(pos, #log, q)
end

local function affected_text(e)
  return position_text(e) .. ('%d row(s) affected%s'):format(e.rows, e.took_ms and (' · ' .. took_text(e.took_ms)) or '')
end

local function view_active(r) return r.filter ~= nil or r.sort ~= nil end

--- The lines of a text result as `r.filter` shows them: `.data.items[0]`
--- (a jq-style path, see jsonpath.lua) picks values out of a JSON body; any
--- other text keeps the lines that contain it (case does not matter).
--- Returns `lines`, or nil and why the filter does not apply.
local function text_view(r)
  local f = r.filter
  if not f then return r.lines end
  local cache = r.fcache
  if cache and cache.filter == f and cache.n == #r.lines then return cache.lines end
  local out
  if f:match('^%.') or f:match('^%[') then
    local ok, value = pcall(vim.json.decode, table.concat(r.lines, '\n'), { luanil = { object = true, array = true } })
    if not ok then return nil, 'the body is not JSON, so a path does not apply -- use plain text to filter lines' end
    local found, err = require('tradar.jsonpath').query(value, f)
    if not found then return nil, err end
    out = {}
    for _, item in ipairs(found) do vim.list_extend(out, vim.split(render.pretty(item), '\n', { plain = true })) end
  else
    out = {}
    local needle = f:lower()
    for _, line in ipairs(r.lines) do
      if line:lower():find(needle, 1, true) then out[#out + 1] = line end
    end
  end
  r.fcache = { filter = f, n = #r.lines, lines = out }
  return out
end

--- Footer of a text result (an HTTP / gRPC response): status, time, size.
local function text_title(r)
  local text = position_text(r)
  if r.hist then text = text .. ('run %d back of %d (%s) · '):format(r.hist, r.hist_total or r.hist, r.at and os.date('%H:%M:%S', r.at) or '?') end
  text = text .. (r.status or 'response')
  if r.took_ms then text = text .. ' · ' .. took_text(r.took_ms) end
  if r.bytes then text = text .. (' · %d bytes'):format(r.bytes) end
  if r.insecure then text = text .. ' · TLS not verified' end
  text = text .. (' · %d lines'):format(#(r.lines or {}))
  if r.filter then text = text .. (' · filter «%s»: %d lines'):format(r.filter, #(text_view(r) or {})) end
  if r.stream then text = text .. (r.stream_open and ' · streaming (' .. (opts.prefix or '<leader>r') .. 'x stops)' or ' · stream ended') end
  if r.head and #r.head > 0 then text = text .. (' · %sH %s headers'):format(opts.prefix or '<leader>r', r.show_head and 'hides' or 'shows') end
  return text
end

local function results_title(r)
  if r.kind == 'text' then return text_title(r) end
  local text = position_text(r)
  if r.hist then text = text .. ('run %d back of %d (%s) · '):format(r.hist, r.hist_total or r.hist, r.at and os.date('%H:%M:%S', r.at) or '?') end
  text = text .. ('%d/%d rows%s'):format(r.shown, r.total, r.truncated and (' (stopped at %d rows; setup{ max_rows } reads more)'):format(r.total) or '')
  if r.took_ms then text = text .. ' · ' .. took_text(r.took_ms) end
  if view_active(r) then
    local cols = r.kind == 'documents' and r.tcols or r.columns
    local note = {}
    if r.filter then note[#note + 1] = ('filter «%s»: %d of %d loaded'):format(r.filter, #(r.kind == 'documents' and r.tvrows or r.vrows or {}), #(r.kind == 'documents' and r.trows or r.rows or {})) end
    if r.sort and cols and cols[r.sort.col] then note[#note + 1] = ('sorted by %s %s'):format(cols[r.sort.col], r.sort.desc and '↓' or '↑') end
    if #note > 0 then text = text .. ' · ' .. table.concat(note, ' · ') end
    if r.filter and r.shown < r.total and r.kind == 'table' and not r.hist then
      text = text .. (' — %sA asks the server, for all %d rows'):format(opts.prefix or '<leader>r', r.total)
    end
  end
  if r.server_view then
    text = text .. ' · server ' .. ctx.M.describe_server_view(r.server_view.filter, r.server_view.sort, r.server_view.columns or r.columns)
  end
  local w = state.watch
  if w and w.query == r.query and w.conn == r.conn then text = text .. (' · watching every %gs'):format(w.secs) end
  if r.shown < r.total then
    if r.closed then
      text = text .. ('  — %d not loaded: this result was closed when a newer one arrived; re-run it'):format(r.total - r.shown)
    else
      text = text .. ('  — %d more: %sm loads the next page'):format(r.total - r.shown, opts.prefix or '<leader>r')
    end
  end
  return text
end

--- The rows of `rows` that `r.filter` / `r.sort` let through, or nil when no
--- view is on (the grid then shows `rows` as they are).
local function view_rows(r, columns, rows)
  if not view_active(r) then return nil end
  if r.sort and not columns[r.sort.col] then r.sort = nil end
  return gridview.apply(columns, rows, r)
end

--- The grid behind a result as `columns, rows` of strings -- a SQL table, or
--- documents shown as a table -- or nil when it is raw JSON (or not rows).
local function tabular(r)
  if not r then return nil end
  if r.kind == 'table' and r.view ~= 'json' then return r.columns, r.vrows or r.rows end
  if r.kind == 'documents' and r.view == 'table' then return r.tcols, r.tvrows or r.trows end
end

--- What the editable grid is made of: `columns, rows, widths` for a SQL
--- table or documents shown as a table; nil in the JSON view / other kinds.
local function grid_of(r)
  if not r then return nil end
  if r.kind == 'table' and r.view ~= 'json' then return r.columns, r.vrows or r.rows, r.widths, r.aligns end
  if r.kind == 'documents' and r.view == 'table' then return r.tcols, r.tvrows or r.trows, r.twidths, r.taligns end
end


--- The results buffer is a real, editable buffer only while the result is
--- a single-table SELECT whose rows can be addressed (`r.edit`, resolved
--- asynchronously by `resolve_edit`) and is shown as a table; otherwise it
--- stays read-only. A (re)paint is by definition a clean buffer.
local function sync_editable(r)
  local buf = state.results_buf
  if not (buf and vim.api.nvim_buf_is_valid(buf)) then return end
  vim.bo[buf].modifiable = (grid_of(r) ~= nil and r.edit ~= nil and not view_active(r)) or false
  vim.bo[buf].modified = false
end

--- `new_from` (1-based index into `r.rows`), when given, means "only rows
--- from here on are new since the last paint" -- `M.more` passes it after
--- a page lands. For a plain `table` result (the common case: SQL, fixed
--- column set from the query itself) this lets a long scroll session
--- repaint in O(new rows) instead of O(every row loaded so far): if the
--- new page didn't grow any column's width, the already-painted lines'
--- padding is still correct, so only the new rows' own lines get built
--- and appended -- nothing earlier is re-measured or rewritten. A
--- `documents` result shown as a table is **not** eligible: `render.flatten`
--- can introduce a column a later page's documents have and earlier ones
--- never did (its own column set isn't fixed up front the way a SQL
--- result's is), so it always gets a full rebuild below regardless of
--- `new_from`.
local function paint_results(new_from)
  local r = state.result
  local buf = state.results_buf

  if r.kind == 'table' and r.view ~= 'json' and new_from and r.widths and r.aligns and not view_active(r) then
    local new_rows = { unpack(r.rows, new_from) }
    local widths = render.column_widths(r.columns, new_rows, r.widths, r.wide)
    local grew = false
    for i, w in ipairs(widths) do
      if w ~= r.widths[i] then
        grew = true
        break
      end
    end
    -- A column that stopped being all-numbers is aligned differently now.
    local aligns = render.number_columns(r.columns, new_rows, r.aligns)
    for i, a in ipairs(aligns) do
      if a ~= r.aligns[i] then
        grew = true
        break
      end
    end
    if not grew then
      r.spans = render.spans_for(widths)
      local new_lines = render.row_lines(r.columns, widths, new_rows, r.aligns)
      local total = vim.api.nvim_buf_line_count(buf)
      local footer_at = math.max(total - 2, 0)
      vim.list_extend(new_lines, { '', results_title(r) })
      vim.bo[buf].modifiable = true
      vim.api.nvim_buf_set_lines(buf, footer_at, total, false, new_lines)
      vim.bo[buf].modifiable = false
      sync_editable(r)
      return
    end
  end

  local lines
  local body_from = 1 -- first line that is the body (what gets JSON colours)
  if r.kind == 'text' then
    lines = {}
    if r.show_head and r.head and #r.head > 0 then
      vim.list_extend(lines, r.head)
      lines[#lines + 1] = ''
      body_from = #lines + 1
    end
    vim.list_extend(lines, text_view(r) or r.lines)
  elseif r.kind == 'table' and r.view == 'json' then
    lines = vim.split(render.json(r.columns, r.rows), '\n', { plain = true })
    lines[#lines] = nil -- the trailing newline
  elseif r.kind == 'table' then
    r.vrows = view_rows(r, r.columns, r.rows)
    lines, r.spans, r.widths, r.aligns = render.table(r.columns, r.vrows or r.rows, nil, r.wide)
  elseif r.view == 'table' then
    r.tcols, r.trows = render.flatten(r.rows, r.field_order)
    r.tvrows = view_rows(r, r.tcols, r.trows)
    lines, r.spans, r.twidths, r.taligns = render.table(r.tcols, r.tvrows or r.trows, nil, r.twide)
  else
    lines = render.documents(r.rows, r.field_order)
  end
  lines[#lines + 1] = ''
  lines[#lines + 1] = results_title(r)
  local win = vim.fn.bufwinid(buf)
  local pos = win ~= -1 and vim.api.nvim_win_get_cursor(win) or nil
  set_lines(buf, lines)
  -- The JSON view is coloured by `jsonhl` (extmarks), not `syntax json`: the regex
  -- engine is time-limited and dropped colours on big results. No `filetype`, so
  -- no JSON ftplugins/LSP attach to a scratch buffer.
  vim.bo[buf].syntax = ''
  local json_view = (r.kind == 'documents' or r.kind == 'table') and r.view == 'json'
  -- Not the blank line and the footer that close the buffer.
  if r.kind == 'text' then
    jsonhl.apply(buf, r.json and lines or nil, #lines - 2, body_from)
  else
    jsonhl.apply(buf, json_view and lines or nil, #lines - 2)
  end
  if pos then pcall(vim.api.nvim_win_set_cursor, win, { math.min(pos[1], #lines), pos[2] }) end
  sync_editable(r)
end

local function close_cursor(e)
  if not e.id then return end
  rpc.request('cursor.close', { cursor = e.id }, function() end)
  e.id, e.closed = nil, true
  e.edit, e.edit_reason = nil, 'an earlier result — re-run its query to edit it'
end

--- A slot's buffer goes with the slot -- unless it is on screen right now
--- (a message may still be about to replace it).
local function drop_buffer(e)
  local b = e.buf
  e.buf = nil
  if not b then return end
  vim.schedule(function()
    if vim.api.nvim_buf_is_valid(b) and b ~= state.results_buf and vim.fn.bufwinid(b) == -1 then
      pcall(vim.api.nvim_buf_delete, b, { force = true })
    end
  end)
end

--- A result's rows live in the server until its cursor is closed. The
--- `setup{ results_cursors }` (4) most recently used slots keep theirs, so
--- they still page and edit; the others stay viewable from what was already
--- loaded but go read-only -- the server holds a handful of results, not all.
local function release_result(keep)
  local open = {}
  for _, e in ipairs(state.result_log or {}) do
    if e.id and e.id ~= keep then open[#open + 1] = e end
  end
  table.sort(open, function(x, y) return (x.used or 0) > (y.used or 0) end)
  local room = math.max((opts.results_cursors or 4) - (keep and 1 or 0), 0)
  for i, e in ipairs(open) do
    if i > room then close_cursor(e) end
  end
end

local function touch(entry)
  state.result_clock = (state.result_clock or 0) + 1
  entry.used = state.result_clock
end

--- Files `entry` under its slot: a new result for a target that already has
--- one takes that slot (and its buffer, and its place among the tabs) -- the
--- slot's earlier run moves to its history (`[R`/`]R`); a new target gets a
--- new slot, and the least recently used one goes when more than `setup{
--- results_kept }` (10) exist. A pinned slot is never taken over (the new
--- result gets a slot of its own) nor dropped for room. `refresh_of` (the
--- result a refresh repeats) takes that very slot and adds no history.
local function log_result(entry, refresh_of)
  local log = state.result_log
  if not log then
    log = {}
    state.result_log = log
  end
  touch(entry)
  local at
  for i, e in ipairs(log) do
    if refresh_of and e == refresh_of then at = i break end
  end
  if not at then
    for i, e in ipairs(log) do
      if e.key == entry.key and not e.pinned then at = i break end
    end
  end
  if at then
    local e = log[at]
    entry.buf = e.buf
    entry.pinned = e.pinned
    if e.id and e.id ~= entry.id then close_cursor(e) end
    local tabular_run = e.kind ~= 'affected' and entry.kind ~= 'affected'
    local history = {}
    if tabular_run and not refresh_of then history[1] = e end
    for _, old in ipairs(e.history or {}) do history[#history + 1] = old end
    while #history > (opts.results_history or 5) do history[#history] = nil end
    entry.history = #history > 0 and history or nil
    e.history, e.hidx, e.hist = nil, nil, nil
    e.buf = nil
    log[at] = entry
    state.result_pos = at
    return
  end
  log[#log + 1] = entry
  while #log > (opts.results_kept or 10) do
    local lru
    for i = 1, #log - 1 do
      if not log[i].pinned and (not lru or (log[i].used or 0) < (log[lru].used or 0)) then lru = i end
    end
    if not lru then
      lru = 1
      for i = 2, #log - 1 do
        if (log[i].used or 0) < (log[lru].used or 0) then lru = i end
      end
    end
    local gone = table.remove(log, lru)
    close_cursor(gone)
    drop_buffer(gone)
  end
  for i, e in ipairs(log) do
    if e == entry then state.result_pos = i end
  end
end

--- Messages ("running…", an error, "3 row(s) affected") share one buffer
--- of their own, so they never overwrite a slot's grid.
local function message_buffer()
  local buf = scratch('tradar://results', state.message_buf)
  state.message_buf = buf
  state.results_buf = buf
  ctx.attach_results(buf)
  return buf
end

--- The slot's own buffer, named after its target.
local function slot_buffer(e)
  if e.buf and vim.api.nvim_buf_is_valid(e.buf) then return e.buf end
  local buf = vim.api.nvim_create_buf(false, true)
  vim.bo[buf].buftype = 'acwrite' -- so `:w` reaches BufWriteCmd
  vim.bo[buf].bufhidden = 'hide'
  vim.bo[buf].swapfile = false
  local base, n = 'tradar://results/' .. (e.label or 'result'), 1
  local name = base
  while not pcall(vim.api.nvim_buf_set_name, buf, name) do
    n = n + 1
    name = ('%s (%d)'):format(base, n)
  end
  e.buf = buf
  return buf
end

--- Remembers where the cursor is in the slot on screen, to come back to it.
local function save_cursor()
  local win = results_win()
  local r = state.result
  if win and r and r.buf and vim.api.nvim_win_get_buf(win) == r.buf then
    r.cursor = vim.api.nvim_win_get_cursor(win)
  end
end

ctx.show_message = function(text, height)
  save_cursor()
  local buf = message_buffer()
  state.result = nil
  state.result_pos = nil -- showing something that is not in the log
  set_lines(buf, vim.split(text, '\n', { plain = true }))
  show(buf, height or 6)
end

--- Documents (Mongo, Elasticsearch, Redis replies) open as JSON; the table
--- is one `<leader>rv` away, and is what editing needs. `setup{
--- documents_view = 'table' }` makes the table the default instead.
local function default_view()
  return opts.documents_view or 'json'
end

--- The last successful `resolve_edit`, so re-running the same query (after a
--- commit, say) is editable at once instead of read-only until the server answers.

ctx.repaint_results = function()
  if state.result and state.results_buf and vim.api.nvim_buf_is_valid(state.results_buf) and not ctx.dirty() then paint_results() end
end

local function show_result(r, meta)
  local took = r.took_ms
  local refresh = meta and meta.refresh
  if ctx.dirty() then
    notify('discarded unsaved grid edits', vim.log.levels.WARN)
    paint_results() -- puts the grid it was painted from back, so a later visit is clean
  end
  if not refresh then save_cursor() end
  local query = meta and meta.query
  local key = slots.key(meta and meta.conn, query)
  local label = slots.label(query or '')
  -- A filter / sort asked of the server (`serverview`) runs a wrapper around the
  -- statement but stays in the tab of the statement.
  local origin = meta and meta.slot_of
  if origin then key, label = origin.key, origin.label end
  if r.kind == 'affected' then
    state.result = nil
    local entry = { kind = 'affected', rows = r.rows, took_ms = took, conn = meta and meta.conn, query = query, closed = true, key = key, label = label, at = os.time() }
    log_result(entry)
    release_result()
    local buf = message_buffer()
    set_lines(buf, { affected_text(entry) })
    return show(buf, 4)
  end
  local entry = {
    id = r.cursor, total = r.total, shown = #r.rows, kind = r.kind,
    columns = r.columns, rows = r.rows, truncated = r.truncated, fetching = false,
    -- One entry per row in `rows`, parallel to it (see `M.more`, which
    -- extends both in lockstep as pages load) -- `render.flatten`'s own
    -- `order` parameter, `nil` for anything but a `documents` result from
    -- a server new enough to send it.
    field_order = r.field_order,
    view = r.kind == 'documents' and default_view() or 'table',
    -- Row editing: `edit` = { table, keys } once `resolve_edit`
    -- has found the source table; until then (or if it never does) the grid
    -- is read-only, with `edit_reason` saying why.
    edit = state.edit_cache and state.edit_cache.conn == (meta and meta.conn) and state.edit_cache.query == query
      and state.edit_cache.edit or nil,
    edit_reason = 'still checking whether these rows can be edited…',
    conn = meta and meta.conn, query = query,
    took_ms = took, at = os.time(),
    key = key, label = label,
  }
  local prev = refresh and refresh.replacing
  if prev then
    entry.filter, entry.sort = prev.filter, prev.sort
    entry.server_view = prev.server_view
  end
  if meta and meta.view then entry.server_view = meta.view end
  state.result = entry
  log_result(entry, prev or nil)
  release_result(r.cursor)
  local buf = slot_buffer(entry)
  state.results_buf = buf
  ctx.attach_results(buf)
  paint_results()
  local win = show(buf)
  -- A refresh keeps you where you were; a new result starts at the top.
  local keep = refresh and refresh.cursor
  pcall(vim.api.nvim_win_set_cursor, win, keep and { math.min(keep[1], vim.api.nvim_buf_line_count(buf)), keep[2] } or { 1, 0 })
  if not state.result.edit then ctx.resolve_edit(state.result) end
end

--- A text result: an HTTP / gRPC response, or a stream's transcript, built by
--- `restbench`. `entry`: `lines` (the body), `head` (status line + headers),
--- `status`, `json` (colour the body), `took_ms`, `bytes`, `key`, `label`,
--- `conn`, `query`; `rerun` (a function) lets `<leader>rR` repeat it.
local function show_text(entry, refresh_of)
  save_cursor()
  entry.kind = 'text'
  entry.rows = entry.lines
  entry.total, entry.shown = #entry.lines, #entry.lines
  entry.closed = true
  entry.at = entry.at or os.time()
  state.result = entry
  log_result(entry, refresh_of)
  release_result()
  local buf = slot_buffer(entry)
  state.results_buf = buf
  ctx.attach_results(buf)
  paint_results()
  local win = show(buf)
  pcall(vim.api.nvim_win_set_cursor, win, { 1, 0 })
  return win
end

--- Lines pushed onto a text result that is still open (a stream's replies).
--- Repaints are coalesced; the view follows the end only when it was there.
local append_pending = setmetatable({}, { __mode = 'k' })
local function text_append(entry, new)
  vim.list_extend(entry.lines, new)
  local over = #entry.lines - require('tradar.tail').MAX_LINES
  if over > 0 then
    for _ = 1, over do table.remove(entry.lines, 1) end
  end
  entry.total, entry.shown = #entry.lines, #entry.lines
  if append_pending[entry] then return end
  append_pending[entry] = true
  vim.defer_fn(function()
    append_pending[entry] = nil
    if state.result ~= entry or not (entry.buf and vim.api.nvim_buf_is_valid(entry.buf)) then return end
    local win = vim.fn.bufwinid(entry.buf)
    local follow = win ~= -1 and vim.api.nvim_win_get_cursor(win)[1] >= vim.api.nvim_buf_line_count(entry.buf) - 2
    paint_results()
    if win ~= -1 and follow then
      pcall(vim.api.nvim_win_set_cursor, win, { math.max(vim.api.nvim_buf_line_count(entry.buf) - 2, 1), 0 })
    end
  end, 40)
end

--- Repaints the tab on screen (a stream ended, the headers were toggled).
local function text_changed(entry)
  if state.result == entry and entry.buf and vim.api.nvim_buf_is_valid(entry.buf) then paint_results() end
end

--- Shows logged result `i` (read-only unless its cursor is still open, see
--- `release_result`).
local function show_entry(i)
  local e = state.result_log[i]
  save_cursor()
  touch(e)
  e.hidx, e.hist = nil, nil
  state.result_pos = i
  if e.kind == 'affected' then
    state.result = nil
    local buf = message_buffer()
    set_lines(buf, { affected_text(e) })
    return show(buf, 4)
  end
  state.result = e
  local buf = slot_buffer(e)
  state.results_buf = buf
  ctx.attach_results(buf)
  paint_results()
  local win = show(buf)
  local c = e.cursor or { 1, 0 }
  pcall(vim.api.nvim_win_set_cursor, win, { math.min(c[1], vim.api.nvim_buf_line_count(buf)), c[2] })
end

local function results_blocked()
  local log = state.result_log or {}
  if #log == 0 then
    notify('no results kept yet', vim.log.levels.WARN)
    return true
  end
  if ctx.dirty() then
    notify('unsaved grid edits — :w to commit or ' .. (opts.prefix or '<leader>r') .. 'u to revert first', vim.log.levels.WARN)
    return true
  end
end

--- `]r` / `[r`: the next / previous result tab (wrapping around).
function M.step_result(delta)
  if results_blocked() then return end
  local log = state.result_log
  if #log == 1 and state.result_pos == 1 then return notify('this is the only kept result', vim.log.levels.INFO) end
  local from = state.result_pos or (delta > 0 and 0 or #log + 1)
  show_entry((from - 1 + delta) % #log + 1)
end

--- `]R` / `[R`: an earlier / later run of the result on screen (the tab keeps
--- the last `setup{ results_history }` (5) runs of its target). Earlier runs
--- are read-only: their cursors were closed when the newer run replaced them.
function M.step_history(delta)
  if results_blocked() then return end
  local e = state.result_log[state.result_pos or 0]
  if not e or e.kind == 'affected' or not e.history or #e.history == 0 then
    return notify('no earlier run of this result', vim.log.levels.INFO)
  end
  local idx = (e.hidx or 0) + delta
  if idx < 0 then return notify('this is the newest run', vim.log.levels.INFO) end
  if idx > #e.history then return notify('this is the oldest kept run', vim.log.levels.INFO) end
  save_cursor()
  e.hidx = idx ~= 0 and idx or nil
  local target = idx == 0 and e or e.history[idx]
  e.hist, target.hist, target.hist_total = nil, idx ~= 0 and idx or nil, #e.history
  target.buf = e.buf
  state.result = target
  paint_results()
  local win = results_win()
  if win then pcall(vim.api.nvim_win_set_cursor, win, { 1, 0 }) end
end

--- `<leader>rp`: pin / unpin the tab on screen. A pinned tab is not replaced
--- by a later run of the same target (that gets a tab of its own) and is the
--- last to go when there is no room -- keep a result to compare with.
function M.pin_result()
  local e = (state.result_log or {})[state.result_pos or 0]
  if not e then return notify('no result tab to pin', vim.log.levels.WARN) end
  e.pinned = (not e.pinned) or nil
  update_bar()
  notify(e.pinned and ('pinned %s — a new run of it opens its own tab'):format(e.label or 'result') or 'unpinned')
end

--- A result's whole grid as `columns, rows` -- every loaded row, whatever
--- filter or sort is on; documents flattened as in the table view.
local function full_grid(r)
  if r.kind == 'documents' then return render.flatten(r.rows, r.field_order) end
  return r.columns, r.rows
end

local function run_title(r)
  return ('%s%s'):format(r.label or 'result', r.at and (' @' .. os.date('%H:%M:%S', r.at)) or '')
end

local function show_diff(a, b)
  local lines
  if a.kind == 'text' or b.kind == 'text' then
    if a.kind ~= b.kind then return notify('a response can only be compared with another response', vim.log.levels.WARN) end
    local d = vim.diff(table.concat(a.lines, '\n') .. '\n', table.concat(b.lines, '\n') .. '\n', { result_type = 'unified', ctxlen = 3 })
    lines = { ('tradar diff  %s  →  %s'):format(run_title(a), run_title(b)) }
    if d == '' then
      lines[#lines + 1] = 'no differences in the body'
    else
      vim.list_extend(lines, vim.split(d, '\n', { plain = true, trimempty = true }))
    end
  else
    local ca, ra = full_grid(a)
    local cb, rb = full_grid(b)
    local keys = (b.edit and b.edit.keys) or (a.edit and a.edit.keys)
    local d, err = rdiff.compare(ca, ra, cb, rb, keys)
    if not d then return notify(err, vim.log.levels.WARN) end
    local note = (a.shown < a.total or b.shown < b.total) and ' (rows not loaded are not compared)' or ''
    lines = rdiff.lines(d, run_title(a), run_title(b))
    if note ~= '' then lines[2] = lines[2] .. note end
  end
  local buf = scratch('tradar://diff', state.diff_buf)
  state.diff_buf = buf
  set_lines(buf, lines)
  vim.api.nvim_buf_call(buf, function()
    vim.cmd([[syntax match DiffAdd /^+.*/]])
    vim.cmd([[syntax match DiffDelete /^-.*/]])
    vim.cmd([[syntax match DiffChange /^\~.*/]])
    vim.cmd([[syntax match DiffChange /^@@.*/]])
    vim.cmd([[syntax match Comment /^tradar diff.*\|^matched by.*\|^only .*/]])
  end)
  vim.keymap.set('n', 'q', function()
    local w = vim.fn.bufwinid(buf)
    if w ~= -1 then vim.api.nvim_win_close(w, true) end
  end, { buffer = buf, desc = 'tradar: close the diff' })
  if vim.fn.bufwinid(buf) == -1 then
    local origin = vim.api.nvim_get_current_win()
    vim.cmd(('botright %dsplit'):format(math.min(math.max(#lines + 1, 6), 20)))
    vim.api.nvim_win_set_buf(0, buf)
    vim.wo.wrap = false
    vim.api.nvim_set_current_win(origin)
  end
end

--- `<leader>rD` / `:TradarDiff [n]`: what changed between two results of the
--- same shape -- rows added, removed and cells changed, matched by the key
--- column(s). With no argument: the tab's previous run against the latest
--- (or, while looking at an earlier run with `[R`, that run against the
--- latest); with none to compare, pick another tab. `n`: tab `n` (before)
--- against the one on screen (after).
function M.diff_result(arg)
  if results_blocked() then return end
  local log = state.result_log
  local e = log[state.result_pos or 0]
  if not e or e.kind == 'affected' then return notify('no table result on screen to compare', vim.log.levels.WARN) end
  local n = tonumber(arg)
  if n then
    local a = log[n]
    if not a or a.kind == 'affected' or a == e or (a.kind == 'text') ~= (e.kind == 'text') then return notify(('tab %s is not another result of the same kind'):format(arg), vim.log.levels.WARN) end
    return show_diff(a, e)
  end
  if state.result ~= e and state.result and state.result.hist then return show_diff(state.result, e) end
  if e.history and #e.history > 0 then return show_diff(e.history[1], e) end
  local items = {}
  for i, other in ipairs(log) do
    if other ~= e and other.kind ~= 'affected' and (other.kind == 'text') == (e.kind == 'text') then items[#items + 1] = i end
  end
  if #items == 0 then return notify('nothing to compare with yet — run it again, or open another result', vim.log.levels.WARN) end
  vim.ui.select(items, {
    prompt = 'compare with (before)',
    format_item = function(i) return ('%d  %s  (%d rows)'):format(i, log[i].label or '', log[i].total or 0) end,
  }, function(i)
    if i then show_diff(log[i], e) end
  end)
end

--- `<leader>r1`..`9`: result tab `n`.
function M.goto_result(n)
  if results_blocked() then return end
  local log = state.result_log
  if n < 1 or n > #log then return notify(('no result tab %d (%d kept)'):format(n, #log), vim.log.levels.WARN) end
  show_entry(n)
end

--- Clicking a tab on the results window.
_G.__tradar_result_click = function(n, _, button)
  if button == 'l' or button == nil then M.goto_result(n) end
end

--- `<leader>rx`: close the tab on screen (its rows are released on the server).
function M.close_result()
  local log = state.result_log or {}
  local i = state.result_pos
  if not i or not log[i] then return notify('no result tab to close', vim.log.levels.WARN) end
  if ctx.dirty() then
    return notify('unsaved grid edits — :w to commit or ' .. (opts.prefix or '<leader>r') .. 'u to revert first', vim.log.levels.WARN)
  end
  if state.watch and state.result == log[i] and ctx.stop_watch then ctx.stop_watch() end
  local gone = table.remove(log, i)
  close_cursor(gone)
  if #log == 0 then
    state.result, state.result_pos = nil, nil
    local win = results_win()
    drop_buffer(gone)
    if win and #vim.api.nvim_tabpage_list_wins(0) > 1 then vim.api.nvim_win_close(win, true) end
    return
  end
  local nxt = math.min(i, #log)
  state.result_pos = nxt
  show_entry(nxt)
  drop_buffer(gone)
end

--- `:TradarResults`: pick one of the kept results.
function M.pick_result()
  if results_blocked() then return end
  local log = state.result_log
  local items = {}
  for i = #log, 1, -1 do items[#items + 1] = i end
  table.sort(items, function(x, y) return (log[x].used or 0) > (log[y].used or 0) end)
  vim.ui.select(items, {
    prompt = 'tradar results',
    format_item = function(i)
      local e = log[i]
      local q = (e.query or ''):gsub('%s+', ' ')
      if #q > 60 then q = q:sub(1, 59) .. '…' end
      local what = e.kind == 'affected' and ('%d row(s) affected'):format(e.rows) or e.kind == 'text' and (e.status or 'response') or ('%d rows'):format(e.total)
      return ('%d  %s  (%s)  %s%s'):format(i, e.label or '', what, q, i == state.result_pos and '  ◀' or '')
    end,
  }, function(i)
    if i then show_entry(i) end
  end)
end

--- `<leader>rz`: blow the results window up to the whole screen (and focus
--- it), or put it back and return to where you were. With
--- `results_layout = 'tab'` the results already have the whole screen.
function M.zoom()
  local buf = state.results_buf
  local win = buf and vim.api.nvim_buf_is_valid(buf) and vim.fn.bufwinid(buf) or -1
  if win == -1 then return notify('no results window to zoom') end
  if vim.api.nvim_win_get_config(win).relative ~= '' or #vim.api.nvim_tabpage_list_wins(vim.api.nvim_win_get_tabpage(win)) == 1 then
    return notify('the results already fill the screen')
  end
  local zoom = state.zoom
  if zoom and zoom.win == win then
    state.zoom = nil
    vim.api.nvim_win_set_height(win, zoom.height)
    vim.api.nvim_win_set_width(win, zoom.width)
    if zoom.origin and vim.api.nvim_win_is_valid(zoom.origin) then vim.api.nvim_set_current_win(zoom.origin) end
    return
  end
  local origin = vim.api.nvim_get_current_win()
  state.zoom = {
    win = win,
    height = vim.api.nvim_win_get_height(win),
    width = vim.api.nvim_win_get_width(win),
    origin = origin ~= win and origin or nil,
  }
  vim.api.nvim_set_current_win(win)
  vim.cmd('wincmd _ | wincmd |')
end

--- `<leader>rv`: the result as a table (editable) or raw JSON -- documents
--- as they are, SQL rows as an array of objects. Cell operations are off in
--- the JSON view (there is no grid to point at).
--- `zw`: the column under the cursor shows its full value (up to
--- `render.MAX_LAST_COLUMN_WIDTH`) instead of being cut at
--- `render.MAX_COLUMN_WIDTH`; again to cut it back. `all`: every column
--- (`zW`) -- or, when some already are, none.
function M.toggle_column_width(all)
  local r = state.result
  if not r or not grid_of(r) then
    return notify('column width applies to the table view', vim.log.levels.WARN)
  end
  if ctx.dirty() then
    return notify('unsaved grid edits — :w to commit or ' .. (opts.prefix or '<leader>r') .. 'u to revert first', vim.log.levels.WARN)
  end
  local doc_table = r.kind == 'documents'
  local key = doc_table and 'twide' or 'wide'
  local cols = doc_table and r.tcols or r.columns
  local wide = r[key] or {}
  local col = render.column_at(r.spans or {}, vim.fn.virtcol('.'))
  if all then
    local any = next(wide) ~= nil
    wide = {}
    if not any then
      for i = 1, #cols do wide[i] = true end
    end
  elseif col then
    wide[col] = (not wide[col]) or nil
  else
    return
  end
  r[key] = wide
  paint_results()
  -- Stay on the same column: its start moved with the widths around it.
  local span = col and r.spans and r.spans[col]
  if span then pcall(vim.cmd, ('normal! %d|'):format(span[1])) end
end

--- `<leader>rF` / `:TradarFilter [expr]`: show only the loaded rows that match
--- (`status = failed`, `age >= 30`, `city ~ han`, or any text); empty clears.
--- Rows are not fetched -- the rest of a long result is untouched (see
--- `M.more`); exports keep every row. The grid is read-only while it is on.
function M.set_filter(text)
  local r = state.result
  if r and r.kind == 'text' then
    local function apply_text(value)
      if value == nil then return end
      value = vim.trim(value)
      local previous = r.filter
      r.filter = (value ~= '' and value ~= 'off') and value or nil
      local _, why = text_view(r)
      if why then
        r.filter = previous
        return notify(why, vim.log.levels.WARN)
      end
      paint_results()
      local win = results_win()
      if win then pcall(vim.api.nvim_win_set_cursor, win, { 1, 0 }) end
    end
    if text ~= nil then return apply_text(text) end
    return vim.ui.input({ prompt = 'Filter the response (.data.items[0].id for JSON, or text to keep matching lines; empty clears): ', default = r.filter or '' }, apply_text)
  end
  if not r or not grid_of(r) then return notify('filter applies to the table view', vim.log.levels.WARN) end
  if ctx.dirty() then
    return notify('unsaved grid edits — :w to commit or ' .. (opts.prefix or '<leader>r') .. 'u to revert first', vim.log.levels.WARN)
  end
  local function apply(value)
    if value == nil then return end
    value = vim.trim(value)
    r.filter = (value ~= '' and value ~= 'off') and value or nil
    paint_results()
    local win = results_win()
    if win then pcall(vim.api.nvim_win_set_cursor, win, { 1, 0 }) end
  end
  if text ~= nil then return apply(text) end
  vim.ui.input({ prompt = 'Filter rows (col = v, col ~ text, col > 3, or any text; empty clears): ', default = r.filter or '' }, apply)
end

--- `<leader>ro`: sort by the column under the cursor -- ascending, then
--- descending, then back to the query's own order. Loaded rows only.
function M.toggle_sort()
  local r = state.result
  if not r or not grid_of(r) then return notify('sort applies to the table view', vim.log.levels.WARN) end
  if ctx.dirty() then
    return notify('unsaved grid edits — :w to commit or ' .. (opts.prefix or '<leader>r') .. 'u to revert first', vim.log.levels.WARN)
  end
  local col = render.column_at(r.spans or {}, vim.fn.virtcol('.'))
  if not col then return notify('put the cursor on a column to sort by', vim.log.levels.WARN) end
  local s = r.sort
  if s and s.col == col then
    r.sort = (not s.desc) and { col = col, desc = true } or nil
  else
    r.sort = { col = col }
  end
  paint_results()
  local span = r.spans and r.spans[col]
  if span then pcall(vim.cmd, ('normal! %d|'):format(span[1])) end
end

--- `<leader>rH`: show / hide the status line and headers above a response body.
function M.toggle_head()
  local r = state.result
  if not r or r.kind ~= 'text' or not r.head or #r.head == 0 then return notify('no response headers on screen') end
  r.show_head = (not r.show_head) or nil
  paint_results()
end

function M.toggle_view()
  local r = state.result
  if not r or (r.kind ~= 'documents' and r.kind ~= 'table') then return notify('no result to switch views on') end
  if ctx.dirty() then return notify('unsaved grid edits — :w to commit or ' .. (opts.prefix or '<leader>r') .. 'u to revert', vim.log.levels.WARN) end
  r.view = r.view == 'table' and 'json' or 'table'
  paint_results()
end

--- Appends the next page. Called by scrolling to the bottom and by
--- `<leader>rm`; `cb` runs when the rows have landed (or immediately if
--- there is nothing more).
function M.more(cb)
  local r = state.result
  if ctx.dirty() then
    -- Appending a page repaints the grid, which would drop the edits.
    if cb then notify('unsaved grid edits — :w to commit or <leader>ru to revert first', vim.log.levels.WARN) end
    return
  end
  if not r or r.shown >= r.total or r.fetching then
    if cb then cb() end
    return
  end
  if r.closed then
    -- A newer result took the server's cursor. (A caller that loops until
    -- everything is loaded checks `closed` itself.)
    notify('this result was closed when a newer one arrived — re-run its query to load more rows', vim.log.levels.WARN)
    if cb then cb() end
    return
  end
  r.fetching = true
  call('fetch', { cursor = r.id, offset = r.shown, limit = cb and 2000 or (opts.page_size or DEFAULT_PAGE) }, function(page)
    r.fetching = false
    if state.result ~= r then return end
    local new_from = #r.rows + 1
    vim.list_extend(r.rows, page.rows)
    if r.field_order and page.field_order then vim.list_extend(r.field_order, page.field_order) end
    r.shown = #r.rows
    paint_results(new_from)
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
  if r.closed then
    notify(('only the %d rows loaded before a newer result closed this one'):format(r.shown), vim.log.levels.WARN)
    return cb(r)
  end
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
  if ctx.dirty() then
    notify('unsaved grid edits (line numbers no longer match the rows) — :w to commit or <leader>ru to revert', vim.log.levels.WARN)
    return nil
  end
  local row = vim.api.nvim_win_get_cursor(0)[1] - 2
  if row < 1 or row > #rows then return nil end
  return row, render.column_at(r.spans, vim.fn.virtcol('.'))
end

local function export_text(fmt, r)
  if r.kind == 'text' then return table.concat(r.lines, '\n') .. '\n' end
  local columns, rows = r.columns, r.rows
  if r.kind == 'documents' then
    -- Documents keep their real structure as JSON; the flat formats use the
    -- same flattened columns as the table view.
    if fmt == 'json' then
      local text = render.pretty(r.rows)
      if r.field_order then
        -- Rebuild per item so each keeps its stored top-level order.
        local parts = {}
        for i, item in ipairs(r.rows) do parts[i] = render.pretty(item, r.field_order[i], '  ') end
        text = '[\n' .. table.concat(vim.tbl_map(function(p) return '  ' .. p end, parts), ',\n') .. '\n]'
      end
      return text .. '\n'
    end
    columns, rows = render.flatten(r.rows, r.field_order)
  end
  if fmt == 'csv' then return render.csv(columns, rows) end
  if fmt == 'json' then return render.json(columns, rows) end
  if fmt == 'md' or fmt == 'markdown' then return render.markdown(columns, rows) end
  if fmt == 'tsv' then return render.tsv(columns, rows, true) .. '\n' end
  return nil, 'unknown format `' .. tostring(fmt) .. '` (csv, json, md, tsv)'
end

--- " — NOT the whole result …" when the server stopped reading early: an
--- export or yank of a cut-short result must not look complete.
local function cut_note(r)
  if r.closed and r.shown < r.total then
    return (' — NOT the whole result: only the %d of %d rows loaded before a newer result closed it'):format(r.shown, r.total)
  end
  if not r.truncated then return '' end
  return (' — NOT the whole result: stopped at %d rows (setup{ max_rows } reads more)'):format(#r.rows)
end

--- Yanks the whole result (every row, loading what's missing) in `fmt`.
function M.yank_all(fmt)
  all_rows(function(r)
    local text, err = export_text(fmt, r)
    if not text then return notify(err, vim.log.levels.WARN) end
    yank(text, ('%d rows as %s%s'):format(#r.rows, fmt, cut_note(r)))
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
      notify(('wrote %d rows to %s%s'):format(#r.rows, target, cut_note(r)), r.truncated and vim.log.levels.WARN or nil)
    end
    if path and path ~= '' then return write(path) end
    vim.ui.input({ prompt = 'Export to: ', default = 'result.' .. fmt, completion = 'file' }, write)
  end)
end

ctx.show = show
ctx.show_result = show_result
ctx.show_text = show_text
ctx.text_append = text_append
ctx.text_changed = text_changed
ctx.took_text = took_text
ctx.scratch = scratch
ctx.set_lines = set_lines
ctx.tabular = tabular
ctx.cell_at = cell_at
ctx.grid_of = grid_of
ctx.paint_results = paint_results
ctx.results_title = results_title
ctx.sync_editable = sync_editable
ctx.yank = yank
