local ctx = require('tradar.ctx')
local M = ctx.M
local gridedit = ctx.gridedit
local render = ctx.render
local opts = ctx.opts
local state = ctx.state
local notify = ctx.notify
local call = ctx.call

-- ── editing rows in the results buffer ───────────────────────────────────
--
-- The grid is edited like any buffer. `:w` diffs its text against the rows
-- it was painted from (`gridedit.diff`: rows are matched by their key
-- cells, so dd / yyp / :g / :sort / cc all just work), shows every
-- UPDATE / DELETE / INSERT the server generates in a preview, and runs them
-- only when that preview is confirmed -- one commit for the whole batch.

--- Works out, once per result, whether its rows can be addressed: the one
--- table they came from and the primary-key columns (which must be in the
--- result). Until it answers -- or if it never can -- the grid stays
--- read-only with `r.edit_reason` saying why, the same rule as the TUI's
--- row edit: a statement that might hit several rows is not one to write
--- on the user's behalf.
--- Indices in `cols` of the key columns `keys` (case-insensitive), or nil if
--- one is missing. Looked up at each write: the flattened columns of a
--- documents grid can change as pages load.
local function key_indices(cols, keys)
  local idx = {}
  for _, k in ipairs(keys) do
    local found
    for i, c in ipairs(cols) do
      if c:lower() == k:lower() then found = i break end
    end
    if not found then return nil end
    idx[#idx + 1] = found
  end
  return idx
end

ctx.resolve_edit = function(r)
  if (r.kind ~= 'table' and r.kind ~= 'documents') or not r.conn then
    r.edit_reason = 'this result cannot be edited'
    return
  end
  local function fail(reason)
    if state.result == r then r.edit, r.edit_reason = nil, reason end
  end
  call('edit.source', { connection = r.conn, query = r.query }, function(src)
    if state.result ~= r then return end
    if not src.table then
      return fail('read-only: cannot tell which table these rows come from (single-table SELECTs only)')
    end
    local function with_keys(keys)
      if state.result ~= r then return end
      if not keys or #keys == 0 then
        return fail(('read-only: `%s` has no primary key, so a row cannot be addressed'):format(src.table))
      end
      -- Documents in the JSON view have no columns yet: checked at `:w`.
      local cols = ctx.grid_of(r)
      if cols and not key_indices(cols, keys) then
        return fail(('read-only: a key column (%s) is not in this result — select it too'):format(table.concat(keys, ', ')))
      end
      r.edit, r.edit_reason = { table = src.table, keys = keys }, nil
      state.edit_cache = { conn = r.conn, query = r.query, edit = r.edit }
      ctx.sync_editable(r)
    end
    if src.key_columns and #src.key_columns > 0 then return with_keys(src.key_columns) end
    ctx.get_schema(r.conn, function(entries)
      local tbl = ctx.find_table(entries, src.table)
      local keys = {}
      for _, c in ipairs(tbl and tbl.columns or {}) do
        if c.primary_key then keys[#keys + 1] = c.name end
      end
      with_keys(keys)
    end)
  end, function(err) fail(err) end)
end

--- `<leader>ru`: drops the unsaved edits, repainting the grid from the rows.
function M.revert()
  local r = state.result
  if not r then return notify('no result', vim.log.levels.WARN) end
  ctx.paint_results()
  notify('grid reverted')
end

--- Asks the server for the statement of every change in `d` (a
--- `gridedit.diff`), in the order they should run -- updates, deletes,
--- inserts, one `UPDATE` per changed row -- and calls
--- `cb(statements, counts)`; nothing is run.
local function build_statements(r, d, cols, rows, key_idx, cb)
  local e = r.edit
  local jobs, counts = {}, { update = 0, delete = 0, insert = 0 }
  local function key_of(row)
    local key = {}
    for j, idx in ipairs(key_idx) do
      local v = rows[row][idx]
      if v == nil or v == 'NULL' then return nil, e.keys[j] end
      key[e.keys[j]] = v
    end
    return key
  end
  for _, u in ipairs(d.updates) do
    local key, bad = key_of(u.row)
    if not key then return notify(('cannot address row %d: key `%s` is NULL'):format(u.row, bad), vim.log.levels.ERROR) end
    local changed = vim.tbl_keys(u.cells)
    table.sort(changed)
    -- A row's changed cells are one statement: the row changes atomically,
    -- and a single cell keeps the original (older-server-friendly) shape.
    local set
    if #changed == 1 then
      set = { column = cols[changed[1]], value = u.cells[changed[1]] }
    else
      set = {}
      for _, c in ipairs(changed) do set[#set + 1] = { column = cols[c], value = u.cells[c] } end
    end
    counts.update = counts.update + 1
    jobs[#jobs + 1] = { connection = r.conn, table = e.table, key = key, change = { set = set } }
  end
  for _, x in ipairs(d.deletes) do
    local key, bad = key_of(x.row)
    if not key then return notify(('cannot address row %d: key `%s` is NULL'):format(x.row, bad), vim.log.levels.ERROR) end
    counts.delete = counts.delete + 1
    jobs[#jobs + 1] = { connection = r.conn, table = e.table, key = key, change = 'delete' }
  end
  for _, ins in ipairs(d.inserts) do
    -- A JSON object has no order, so the grid's column order travels
    -- separately and keeps the generated INSERT in the table's own order.
    local values, order = {}, {}
    for c = 1, #cols do
      if ins.values[c] ~= nil then
        values[cols[c]] = ins.values[c]
        order[#order + 1] = cols[c]
      end
    end
    counts.insert = counts.insert + 1
    jobs[#jobs + 1] = { connection = r.conn, table = e.table, insert = values, insert_columns = order }
  end
  local sqls, left, failed = {}, #jobs, false
  for i, params in ipairs(jobs) do
    call('edit.sql', params, function(res)
      if failed then return end
      if not res.sql or res.sql == vim.NIL then
        failed = true
        return notify('this connector cannot ' .. (params.insert and 'insert rows' or 'edit rows') .. ' (read-only)', vim.log.levels.WARN)
      end
      sqls[i] = res.sql
      left = left - 1
      if left == 0 then cb(sqls, counts) end
    end, function(err)
      if not failed then notify(err, vim.log.levels.ERROR) end
      failed = true
    end)
  end
end

local preview_buf = nil

--- Drivers that hold a manual transaction (BEGIN/COMMIT/ROLLBACK) on their
--- one connection -- see `transaction_control` in the connector crates.
local TRANSACTIONAL = { postgres = true, mysql = true, sqlite = true }

--- The commit preview: every statement, one per line, in a split. `:w` or
--- `<CR>` runs them in order (stopping at the first error, then showing
--- the ones that did not run); `q` cancels. `edit` = { conn, query, sqls,
--- sql_buf, pos, note }.
local function open_preview(edit)
  local buf = preview_buf
  if not (buf and vim.api.nvim_buf_is_valid(buf)) then
    buf = vim.api.nvim_create_buf(false, true)
    preview_buf = buf
    vim.bo[buf].buftype = 'acwrite'
    vim.bo[buf].bufhidden = 'wipe'
    vim.bo[buf].swapfile = false
    vim.bo[buf].filetype = 'sql'
    pcall(vim.api.nvim_buf_set_name, buf, 'tradar://commit')
  end
  local function close()
    local win = vim.fn.bufwinid(buf)
    if win ~= -1 then vim.api.nvim_win_close(win, true) end
  end
  local function apply()
    close()
    -- Confirmed: from here the grid's text is no longer "unsaved".
    if state.results_buf and vim.api.nvim_buf_is_valid(state.results_buf) then vim.bo[state.results_buf].modified = false end
    local total = #edit.sqls
    local function reload(after_reload)
      ctx.run_text(edit.sql_buf, edit.query, 0, 0, {
        confirmed = true,
        after = function()
          local w = vim.fn.bufwinid(state.results_buf)
          if w ~= -1 and edit.pos then
            local lines = vim.api.nvim_buf_line_count(state.results_buf)
            pcall(vim.api.nvim_win_set_cursor, w, { math.min(edit.pos[1], lines), edit.pos[2] })
          end
          if after_reload then after_reload() end
        end,
        on_error = after_reload,
      })
    end
    --- Runs the statements in order; `wrapped` brackets them in
    --- BEGIN ... COMMIT (the driver holds that transaction on its one
    --- connection), so a failure rolls everything back.
    local function run(wrapped)
      local function fail(i, err)
        local first = err:gsub('\n.*', '')
        if wrapped then
          -- Roll back whatever ran, show the real table again, and offer the
          -- whole batch back so a fix can be re-run from the start.
          ctx.run_text(edit.sql_buf, 'ROLLBACK', 0, 0, {
            confirmed = true,
            after = function()
              reload(function()
                open_preview(vim.tbl_extend('force', edit, {
                  note = ('-- rolled back, nothing was applied; statement %d of %d failed: %s'):format(i, total, first),
                }))
              end)
            end,
            on_error = function() notify('ROLLBACK failed — check the connection (status line shows an open transaction)', vim.log.levels.ERROR) end,
          })
        else
          open_preview(vim.tbl_extend('force', edit, {
            sqls = vim.list_slice(edit.sqls, i),
            note = ('-- %d of %d applied (not in a transaction); stopped at the first statement below: %s'):format(i - 1, total, first),
          }))
        end
      end
      local function step(i)
        if i > total then
          if not wrapped then
            notify(('committed %d statement(s)'):format(total))
            return reload()
          end
          return ctx.run_text(edit.sql_buf, 'COMMIT', 0, 0, {
            confirmed = true,
            after = function()
              notify(('committed %d statement(s) in one transaction'):format(total))
              reload()
            end,
            on_error = function(err) fail(total, err) end,
          })
        end
        ctx.run_text(edit.sql_buf, edit.sqls[i], 0, 0, {
          confirmed = true, -- this preview *is* the confirmation
          after = function() step(i + 1) end,
          on_error = function(err) fail(i, err) end,
        })
      end
      if not wrapped then return step(1) end
      ctx.run_text(edit.sql_buf, 'BEGIN', 0, 0, { confirmed = true, after = function() step(1) end })
    end
    -- One transaction wherever the driver can hold one -- unless the user is
    -- already inside their own (a second BEGIN would be a no-op, and our
    -- COMMIT/ROLLBACK would then settle *their* work): then run inside it.
    ctx.driver_of(edit.conn, function(driver)
      if not TRANSACTIONAL[driver] then return run(false) end
      call('status', { connection = edit.conn }, function(st)
        if st.in_transaction then
          notify('a transaction is already open on this connection — running inside it (not committing it for you)', vim.log.levels.WARN)
          return run(false)
        end
        run(true)
      end)
    end)
  end
  local lines = { edit.note or ('-- tradar [%s]: %d statement(s) — :w or <CR> to run, q to cancel'):format(edit.conn, #edit.sqls) }
  for _, sql in ipairs(edit.sqls) do lines[#lines + 1] = sql .. ';' end
  vim.bo[buf].modifiable = true
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
  vim.bo[buf].modifiable = false
  vim.bo[buf].modified = false
  local win = vim.fn.bufwinid(buf)
  if win == -1 then
    vim.cmd(('botright %dsplit'):format(math.min(#lines + 1, 15)))
    win = vim.api.nvim_get_current_win()
    vim.api.nvim_win_set_buf(win, buf)
    vim.wo[win].wrap = false
  else
    vim.api.nvim_set_current_win(win)
  end
  pcall(vim.api.nvim_win_set_cursor, win, { 1, 0 })
  if not vim.b[buf].tradar_preview_attached then
    vim.b[buf].tradar_preview_attached = true
    vim.keymap.set('n', 'q', close, { buffer = buf, desc = 'tradar: cancel the commit' })
    vim.keymap.set('n', '<CR>', function() vim.b[buf].tradar_apply() end, { buffer = buf, desc = 'tradar: run these statements' })
    vim.api.nvim_create_autocmd('BufWriteCmd', { buffer = buf, callback = function() vim.b[buf].tradar_apply() end })
  end
  vim.b[buf].tradar_apply = apply
end

--- `:w` in the results buffer: diffs the grid against the rows it shows and
--- opens the commit preview. Writes nothing itself.
function M.write()
  local r = state.result
  local buf = state.results_buf
  local cols, rows, widths, aligns = ctx.grid_of(r)
  if not cols then return notify('nothing to write here', vim.log.levels.WARN) end
  if not r.edit then return notify(r.edit_reason or 'this result cannot be edited', vim.log.levels.WARN) end
  local key_idx = key_indices(cols, r.edit.keys)
  if not key_idx then
    return notify(('a key column (%s) is not in this result — select it too'):format(table.concat(r.edit.keys, ', ')), vim.log.levels.WARN)
  end
  local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  local head = render.header_lines(cols, widths, aligns)
  local n = #lines
  if n < 4 or lines[1] ~= head[1] or lines[2] ~= head[2] or lines[n - 1] ~= '' or lines[n] ~= ctx.results_title(r) then
    return notify('only the data rows can change — the header, rule or footer line was edited (' .. (opts.prefix or '<leader>r') .. 'u reverts)', vim.log.levels.ERROR)
  end
  local d = gridedit.diff(cols, rows, widths, key_idx, vim.list_slice(lines, 3, n - 2))
  if #d.errors > 0 then return notify(table.concat(d.errors, '\n'), vim.log.levels.ERROR) end
  if #d.updates + #d.deletes + #d.inserts == 0 then
    vim.bo[buf].modified = false
    return notify('no changes')
  end
  local win = vim.fn.bufwinid(buf)
  local sql_buf = state.sql_buf
  if not (sql_buf and vim.api.nvim_buf_is_valid(sql_buf)) then sql_buf = vim.api.nvim_get_current_buf() end
  build_statements(r, d, cols, rows, key_idx, function(sqls, counts)
    ctx.driver_of(r.conn, function(driver)
      open_preview({
        conn = r.conn, query = r.query, sqls = sqls, sql_buf = sql_buf,
        pos = win ~= -1 and vim.api.nvim_win_get_cursor(win) or nil,
        note = ('-- tradar [%s]: %d update(s), %d delete(s), %d insert(s)%s — :w or <CR> to run, q to cancel'):format(
          r.conn, counts.update, counts.delete, counts.insert,
          TRANSACTIONAL[driver] and ', all in one transaction' or ' (no transaction on this connector)'),
      })
    end)
  end)
end

--- `gK`: the cursor's row with one field per line -- the `\x` view, for when
--- a wide row does not fit the grid.
function M.row_view()
  local r = state.result
  local row = ctx.cell_at()
  if not row then return notify('put the cursor on a row of the table view', vim.log.levels.WARN) end
  local cols, rows = ctx.tabular(r)
  local width = 0
  for _, c in ipairs(cols) do width = math.max(width, vim.fn.strdisplaywidth(c)) end
  local lines = {}
  for i, c in ipairs(cols) do
    local value = tostring(rows[row][i] == nil and 'NULL' or rows[row][i])
    local parts = vim.split(value, '\n', { plain = true })
    lines[#lines + 1] = c .. string.rep(' ', width - vim.fn.strdisplaywidth(c)) .. ' │ ' .. parts[1]
    for j = 2, #parts do lines[#lines + 1] = string.rep(' ', width) .. ' │ ' .. parts[j] end
  end
  vim.lsp.util.open_floating_preview(lines, '', { border = 'rounded', focus_id = 'tradar_help', max_width = 120 })
end

--- Plans the statement under the cursor (`EXPLAIN`, or SQLite's
--- `EXPLAIN QUERY PLAN`). Never `ANALYZE`: that would *execute* it.
function M.explain()
  local cursor_row = vim.api.nvim_win_get_cursor(0)[1]
  local conn = M.block_connection(0, cursor_row)
  if not conn then return M.connect(nil, M.explain) end
  ctx.driver_of(conn, function(driver)
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
