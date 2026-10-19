local ctx = require('tradar.ctx')
local M = ctx.M
local render = ctx.render
local uv = ctx.uv
local state = ctx.state
local notify = ctx.notify
local show_help = ctx.show_help
local call = ctx.call

-- ── navigation helpers (also used by the telescope pickers) ──────────────

--- `cb(entries, buf)` for the connection under the cursor (in a `.tdb`, the
--- fenced block's own).
function M.schema_entries(cb)
  local buf = vim.api.nvim_get_current_buf()
  local conn = M.block_connection(buf, vim.api.nvim_win_get_cursor(0)[1])
  if not conn then
    return M.connect(nil, function() M.schema_entries(cb) end)
  end
  ctx.ensure_connected(conn, function()
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
  local win = vim.fn.bufwinid(buf)
  local conn = M.block_connection(buf, win ~= -1 and vim.api.nvim_win_get_cursor(win)[1] or 1)
  if not conn then return end
  call('snippet', { connection = conn, name = entry.name, schema = entry.schema, op = 'read' }, function(r)
    if not r.text then return notify('this connector has no snippet for that', vim.log.levels.WARN) end
    ctx.run_text(buf, r.text, 0, 0)
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
  M.schema_entries(function(entries, sql_buf, conn)
    local buf = ctx.scratch('tradar://schema', state.schema_buf)
    state.schema_buf, state.schema_entries, state.sql_buf = buf, entries, sql_buf
    state.schema_conn = conn

    local function redraw()
      local win = vim.fn.bufwinid(buf)
      local cursor = win ~= -1 and vim.api.nvim_win_get_cursor(win) or nil
      local lines, nodes = render.schema_tree(state.schema_entries, state.schema_expanded)
      state.schema_nodes = nodes
      ctx.set_lines(buf, lines)
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
    vim.keymap.set('n', 'o', function()
      local node = node_at_cursor()
      if not (node and node.entry) then return notify('put the cursor on a table', vim.log.levels.WARN) end
      if state.sql_buf and vim.api.nvim_buf_is_valid(state.sql_buf) then M.open_table(state.sql_buf, node.entry) end
    end, { buffer = buf, desc = 'tradar: open the table under the cursor (show its rows)' })
    vim.keymap.set('n', 'r', function()
      local c = state.schema_conn
      if not c then return end
      state.schema_cache[c] = nil
      call('schema', { connection = c }, function(fresh)
        state.schema_entries = fresh
        redraw()
        notify(('schema of %s refreshed (%d objects)'):format(c, #fresh))
      end)
    end, { buffer = buf, desc = 'tradar: refresh the schema' })
    vim.keymap.set('n', 'q', '<cmd>close<CR>', { buffer = buf, desc = 'close' })
    vim.keymap.set('n', 'g?', function()
      show_help({
        '# tradar: schema panel',
        '- `<CR>` insert name into the SQL buffer, or toggle a folder/table',
        '- `<Tab>` expand/collapse the schema or table under the cursor',
        '- `o` open the table under the cursor (runs its "show rows" statement)',
        '- `r` refresh the schema (after a migration / DDL elsewhere)',
        '- `/` searches the tree like any buffer',
        '- `q` close',
      })
    end, { buffer = buf, desc = 'tradar: ctx.show keymap help' })
    if vim.fn.bufwinid(buf) == -1 then
      vim.cmd('topleft 40vsplit')
      vim.api.nvim_win_set_buf(0, buf)
      vim.wo.wrap = false
    end
    local win = vim.fn.bufwinid(buf)
    if win ~= -1 and conn then vim.wo[win].winbar = ' schema · ' .. conn end
  end)
end

-- ── schema-aware navigation: hover, go-to, follow a foreign key ──────────

--- Schema for `conn`, cached for a minute and dropped when a DDL statement
--- runs from here -- hover must feel instant, and schemas rarely move.
local function get_schema(conn, cb)
  local hit = state.schema_cache[conn]
  if hit and (uv.hrtime() - hit.at) < 60e9 then return cb(hit.entries) end
  ctx.ensure_connected(conn, function()
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
  local conn = M.block_connection(0, vim.api.nvim_win_get_cursor(0)[1])
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
  local conn = M.block_connection(buf, vim.api.nvim_win_get_cursor(0)[1])
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
  local row, col = ctx.cell_at()
  if not (r and r.conn and row) then return notify('put the cursor on a cell of a table result', vim.log.levels.WARN) end
  local cols, rows = ctx.tabular(r)
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
      ctx.driver_of(r.conn, function(driver)
        local sql = ('SELECT * FROM %s WHERE %s = %s LIMIT 100;'):format(
          quote_ident(fk.table, driver), quote_ident(fk.column, driver), quote_literal(value, driver))
        local buf = state.sql_buf
        if not (buf and vim.api.nvim_buf_is_valid(buf)) then buf = vim.api.nvim_get_current_buf() end
        ctx.run_text(buf, sql, 0, 0, { confirmed = true })
      end)
    end)
  end, function(err) notify(err, vim.log.levels.ERROR) end)
end

ctx.find_table = find_table
ctx.get_schema = get_schema
