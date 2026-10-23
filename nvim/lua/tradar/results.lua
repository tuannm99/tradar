local ctx = require('tradar.ctx')
local M = ctx.M
local rpc = ctx.rpc
local render = ctx.render
local opts = ctx.opts
local DEFAULT_PAGE = ctx.DEFAULT_PAGE
local state = ctx.state
local notify = ctx.notify
local call = ctx.call

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
  if win ~= -1 then
    -- A thin strip (the "running…" note, a one-line message) is replaced by
    -- the real layout once a full result arrives, instead of staying 4 rows.
    if height == nil and vim.w[win].tradar_strip and #vim.api.nvim_tabpage_list_wins(0) > 1 then
      vim.api.nvim_win_close(win, true)
    else
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
  return win
end

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

local function results_title(r)
  local text = position_text(r) .. ('%d/%d rows%s'):format(r.shown, r.total, r.truncated and (' (stopped at %d rows; setup{ max_rows } reads more)'):format(r.total) or '')
  if r.took_ms then text = text .. ' · ' .. took_text(r.took_ms) end
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

--- The grid behind a result as `columns, rows` of strings -- a SQL table, or
--- documents shown as a table -- or nil when it is raw JSON (or not rows).
local function tabular(r)
  if not r then return nil end
  if r.kind == 'table' and r.view ~= 'json' then return r.columns, r.rows end
  if r.kind == 'documents' and r.view == 'table' then return r.tcols, r.trows end
end

--- What the editable grid is made of: `columns, rows, widths` for a SQL
--- table or documents shown as a table; nil in the JSON view / other kinds.
local function grid_of(r)
  if not r then return nil end
  if r.kind == 'table' and r.view ~= 'json' then return r.columns, r.rows, r.widths, r.aligns end
  if r.kind == 'documents' and r.view == 'table' then return r.tcols, r.trows, r.twidths, r.taligns end
end


--- The results buffer is a real, editable buffer only while the result is
--- a single-table SELECT whose rows can be addressed (`r.edit`, resolved
--- asynchronously by `resolve_edit`) and is shown as a table; otherwise it
--- stays read-only. A (re)paint is by definition a clean buffer.
local function sync_editable(r)
  local buf = state.results_buf
  if not (buf and vim.api.nvim_buf_is_valid(buf)) then return end
  vim.bo[buf].modifiable = (grid_of(r) ~= nil and r.edit ~= nil) or false
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

  if r.kind == 'table' and r.view ~= 'json' and new_from and r.widths and r.aligns then
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
  if r.kind == 'table' and r.view == 'json' then
    lines = vim.split(render.json(r.columns, r.rows), '\n', { plain = true })
    lines[#lines] = nil -- the trailing newline
  elseif r.kind == 'table' then
    lines, r.spans, r.widths, r.aligns = render.table(r.columns, r.rows, nil, r.wide)
  elseif r.view == 'table' then
    r.tcols, r.trows = render.flatten(r.rows, r.field_order)
    lines, r.spans, r.twidths, r.taligns = render.table(r.tcols, r.trows, nil, r.twide)
  else
    lines = render.documents(r.rows, r.field_order)
  end
  lines[#lines + 1] = ''
  lines[#lines + 1] = results_title(r)
  local win = vim.fn.bufwinid(buf)
  local pos = win ~= -1 and vim.api.nvim_win_get_cursor(win) or nil
  set_lines(buf, lines)
  -- JSON highlighting by `syntax` only: no `filetype`, so no JSON ftplugins/LSP attach to a scratch buffer.
  -- Regex syntax highlighting of a big JSON buffer is slow, so it is capped:
  -- only the first 200 columns of a line, and not at all past 3000 lines.
  vim.bo[buf].synmaxcol = 200
  local json_view = (r.kind == 'documents' or r.kind == 'table') and r.view == 'json'
  vim.bo[buf].syntax = (json_view and #lines <= 3000) and 'json' or ''
  if pos then pcall(vim.api.nvim_win_set_cursor, win, { math.min(pos[1], #lines), pos[2] }) end
  sync_editable(r)
end

--- A result's rows live in the server until its cursor is closed. Only the
--- newest result keeps one: older ones stay viewable (`[r`) from what was
--- already loaded, but can no longer page or be edited -- so the server holds
--- one result's rows, as before.
local function release_result(keep)
  for _, e in ipairs(state.result_log or {}) do
    if e.id and e.id ~= keep then
      rpc.request('cursor.close', { cursor = e.id }, function() end)
      e.id, e.closed = nil, true
      e.edit, e.edit_reason = nil, 'an earlier result — re-run its query to edit it'
    end
  end
end

--- Keeps `entry` as the newest of the last `setup{ results_kept }` (10).
local function log_result(entry, replacing)
  local log = state.result_log
  if not log then
    log = {}
    state.result_log = log
  end
  -- A refresh takes the slot of the result it refreshes.
  for i, e in ipairs(replacing and log or {}) do
    if e == replacing then
      log[i] = entry
      state.result_pos = i
      return
    end
  end
  log[#log + 1] = entry
  while #log > (opts.results_kept or 10) do table.remove(log, 1) end
  state.result_pos = #log
end

ctx.show_message = function(text, height)
  local buf = scratch('tradar://results', state.results_buf)
  state.results_buf = buf
  ctx.attach_results(buf)
  release_result()
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
  release_result(r.cursor)
  if ctx.dirty() then notify('discarded unsaved grid edits', vim.log.levels.WARN) end
  local buf = scratch('tradar://results', state.results_buf)
  state.results_buf = buf
  vim.bo[buf].buftype = 'acwrite' -- so `:w` reaches BufWriteCmd
  ctx.attach_results(buf)
  if r.kind == 'affected' then
    state.result = nil
    local entry = { kind = 'affected', rows = r.rows, took_ms = took, conn = meta and meta.conn, query = meta and meta.query, closed = true }
    log_result(entry)
    set_lines(buf, { affected_text(entry) })
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
    view = r.kind == 'documents' and default_view() or 'table',
    -- Row editing: `edit` = { table, keys } once `resolve_edit`
    -- has found the source table; until then (or if it never does) the grid
    -- is read-only, with `edit_reason` saying why.
    edit = state.edit_cache and state.edit_cache.conn == (meta and meta.conn) and state.edit_cache.query == (meta and meta.query)
      and state.edit_cache.edit or nil,
    edit_reason = 'still checking whether these rows can be edited…',
    conn = meta and meta.conn, query = meta and meta.query,
    took_ms = took,
  }
  log_result(state.result, meta and meta.refresh and meta.refresh.replacing)
  paint_results()
  local win = show(buf)
  -- A refresh keeps you where you were; a new result starts at the top.
  local keep = meta and meta.refresh and meta.refresh.cursor
  pcall(vim.api.nvim_win_set_cursor, win, keep and { math.min(keep[1], vim.api.nvim_buf_line_count(buf)), keep[2] } or { 1, 0 })
  if not state.result.edit then ctx.resolve_edit(state.result) end
end

--- Shows logged result `i` (read-only: its cursor is closed, see
--- `release_result`; the newest one is whatever `show_result` left).
local function show_entry(i)
  local e = state.result_log[i]
  local buf = scratch('tradar://results', state.results_buf)
  state.results_buf = buf
  ctx.attach_results(buf)
  state.result_pos = i
  if e.kind == 'affected' then
    state.result = nil
    set_lines(buf, { affected_text(e) })
    return show(buf, 4)
  end
  state.result = e
  paint_results()
  local win = show(buf)
  pcall(vim.api.nvim_win_set_cursor, win, { 1, 0 })
end

--- `[r` / `]r`: the previous / next result kept from this and earlier runs.
function M.step_result(delta)
  local log = state.result_log or {}
  if #log == 0 then return notify('no results kept yet', vim.log.levels.WARN) end
  if ctx.dirty() then
    return notify('unsaved grid edits — :w to commit or ' .. (opts.prefix or '<leader>r') .. 'u to revert first', vim.log.levels.WARN)
  end
  local target = (state.result_pos or #log + 1) + delta
  if target < 1 then return notify('this is the oldest kept result', vim.log.levels.INFO) end
  if target > #log then return notify('this is the newest result', vim.log.levels.INFO) end
  show_entry(target)
end

--- `:TradarResults`: pick one of the kept results.
function M.pick_result()
  local log = state.result_log or {}
  if #log == 0 then return notify('no results kept yet', vim.log.levels.WARN) end
  if ctx.dirty() then
    return notify('unsaved grid edits — :w to commit or ' .. (opts.prefix or '<leader>r') .. 'u to revert first', vim.log.levels.WARN)
  end
  local items = {}
  for i = #log, 1, -1 do items[#items + 1] = i end
  vim.ui.select(items, {
    prompt = 'tradar results',
    format_item = function(i)
      local e = log[i]
      local q = (e.query or ''):gsub('%s+', ' ')
      if #q > 60 then q = q:sub(1, 59) .. '…' end
      local what = e.kind == 'affected' and ('%d row(s) affected'):format(e.rows) or ('%d rows'):format(e.total)
      return ('%d  %s  (%s)%s'):format(i, q, what, i == state.result_pos and '  ◀' or '')
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
