-- `<leader>rA`: the filter / sort of a SQL result, asked of the database.
--
-- `rF` / `ro` work on the rows already loaded, which for a big result is a
-- page. This wraps the statement that made the result --
--   SELECT * FROM ( <query> ) AS tradar_q WHERE <filter> ORDER BY <sort>
-- -- and runs it into the same result tab (the unfiltered run moves to the tab's
-- history, `[R`). `<leader>rA` with no filter or sort on screen takes the
-- wrapper off again.
local ctx = require('tradar.ctx')
local M = ctx.M
local state = ctx.state
local notify = ctx.notify
local guard = ctx.guard
local gridview = require('tradar.gridview')

--- Drivers whose SQL takes a derived table and `LIKE`.
local SUPPORTED = { postgres = true, mysql = true, sqlite = true, clickhouse = true }

local function ident(name, driver)
  local q = (driver == 'mysql' or driver == 'clickhouse') and '`' or '"'
  return q .. tostring(name):gsub(q, q .. q) .. q
end

local function literal(value, driver)
  local v = tostring(value):gsub("'", "''")
  if driver == 'mysql' or driver == 'clickhouse' then v = v:gsub('\\', '\\\\') end
  return "'" .. v .. "'"
end

--- The `LIKE` pattern for "contains `value`", and its `ESCAPE` clause.
local function like(value, driver)
  if driver == 'clickhouse' then
    -- `LIKE` takes a backslash escape there (which the string literal doubles).
    return literal('%' .. value:gsub('[%%_\\]', '\\%0') .. '%', driver), ''
  end
  return literal('%' .. value:gsub('[!%%_]', '!%0') .. '%', driver), " ESCAPE '!'"
end

--- The statement, or `nil, why`. Pure: `base` is the query text, `columns` the
--- result's column names, `filter` the `rF` text, `sort` the `ro` state
--- (`{ col, desc }`), `driver` the driver id.
function M.build(base, columns, filter, sort, driver)
  if not SUPPORTED[driver] then
    return nil, 'the server can filter and sort SQL results (postgres, mysql, sqlite, clickhouse)'
  end
  local where
  if filter and vim.trim(filter) ~= '' then
    local cond = gridview.condition(filter, columns)
    if not cond then
      return nil, 'to filter on the server write the filter as `column = value` (also !=, >, <, >=, <=, ~ for "contains")'
    end
    local col = ident(cond.name, driver)
    if cond.op == '~' then
      local pattern, escape = like(cond.value, driver)
      where = ('%s %s %s%s'):format(col, driver == 'postgres' and 'ILIKE' or 'LIKE', pattern, escape)
    else
      where = ('%s %s %s'):format(col, cond.op == '!=' and '<>' or cond.op, literal(cond.value, driver))
    end
  end
  local order
  if sort and columns[sort.col] then
    order = ident(columns[sort.col], driver) .. (sort.desc and ' DESC' or '')
  end
  local inner = base:gsub('[%s;]+$', '')
  local sql = 'SELECT * FROM (\n' .. inner .. '\n) AS tradar_q'
  if where then sql = sql .. '\nWHERE ' .. where end
  if order then sql = sql .. '\nORDER BY ' .. order end
  return sql
end

local function describe(filter, sort, columns)
  local parts = {}
  if filter then parts[#parts + 1] = ('filter «%s»'):format(filter) end
  if sort and columns[sort.col] then
    parts[#parts + 1] = ('sorted by %s %s'):format(columns[sort.col], sort.desc and '↓' or '↑')
  end
  return table.concat(parts, ' · ')
end

M.describe_server_view = describe

--- `<leader>rA`.
function M.apply_server_view()
  local r = state.result
  if not r or r.kind ~= 'table' or not (r.conn and r.query) then
    return notify('the server can filter and sort a table result (a SQL query)', vim.log.levels.WARN)
  end
  if ctx.dirty() then
    return notify('unsaved grid edits — :w to commit or ' .. (ctx.opts.prefix or '<leader>r') .. 'u to revert first', vim.log.levels.WARN)
  end
  if state.running then return notify('a query is already running', vim.log.levels.WARN) end
  local base = r.server_view and r.server_view.base or r.query
  local filter, sort = r.filter, r.sort
  if not filter and not sort and not r.server_view then
    return notify(('nothing to apply — set a filter (%sF) or a sort (%so) first'):format(ctx.opts.prefix or '<leader>r', ctx.opts.prefix or '<leader>r'), vim.log.levels.WARN)
  end
  local buf = (state.sql_buf and vim.api.nvim_buf_is_valid(state.sql_buf)) and state.sql_buf or state.results_buf
  ctx.driver_of(r.conn, function(driver)
    if not guard.is_read_only(base, driver) then
      return notify('not applied: this statement may change data, and a wrapper would run it again', vim.log.levels.WARN)
    end
    local sql, view
    if filter or sort then
      local err
      sql, err = M.build(base, r.columns, filter, sort, driver)
      if not sql then return notify(err, vim.log.levels.WARN) end
      view = { base = base, filter = filter, sort = sort and { col = sort.col, desc = sort.desc } or nil, columns = r.columns }
    else
      sql = base -- no view left on screen: back to the statement itself
    end
    ctx.run_text(buf, sql, 0, 0, { confirmed = true, conn = r.conn, slot_of = r, view = view, no_history = true })
  end)
end

return M
