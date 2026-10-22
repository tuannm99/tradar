-- Generate CRUD statements for a table / collection / index: the driver's own
-- language (SQL, Mongo shell, Elasticsearch request, Redis command, CQL),
-- with `<placeholders>` to fill in. The statement is inserted into the SQL
-- buffer, not run -- unlike `open_table`, which runs a SELECT -- because
-- three of the four change data.
local ctx = require('tradar.ctx')
local M = ctx.M
local notify = ctx.notify
local call = ctx.call

local OPS = {
  { op = 'create', label = 'Create', hint = 'insert a new row / document' },
  { op = 'read', label = 'Read', hint = 'select / find' },
  { op = 'update', label = 'Update', hint = 'change existing rows / documents' },
  { op = 'delete', label = 'Delete', hint = 'remove rows / documents' },
}

local function op_title(op)
  for _, o in ipairs(OPS) do
    if o.op == op then return o.label end
  end
  return op
end

--- Asks for columns until "Done": `cb(list)`; an empty list means "all /
--- the driver's default". `nil` if the user cancels.
local function pick_columns(entry, op, cb)
  local columns = {}
  for _, c in ipairs(entry.columns or {}) do columns[#columns + 1] = c.name end
  -- Delete is keyed by the primary key / _id, whatever columns exist.
  if #columns == 0 or op == 'delete' then return cb({}) end
  local chosen = {}
  local function round()
    local items = { { done = true } }
    for _, name in ipairs(columns) do items[#items + 1] = { name = name } end
    vim.ui.select(items, {
      prompt = ('%s %s -- columns (%d picked; none = all/default)'):format(op_title(op), entry.name, vim.tbl_count(chosen)),
      format_item = function(it)
        if it.done then
          return vim.tbl_isempty(chosen) and '▶ Done: all columns / driver default' or ('▶ Done with %d column(s)'):format(vim.tbl_count(chosen))
        end
        return (chosen[it.name] and '[x] ' or '[ ] ') .. it.name
      end,
    }, function(it)
      if not it then return end
      if it.done then
        local list = {}
        for _, name in ipairs(columns) do -- table order, not click order
          if chosen[name] then list[#list + 1] = name end
        end
        return cb(list)
      end
      chosen[it.name] = (not chosen[it.name]) or nil
      round()
    end)
  end
  vim.ui.select({ 'All columns / driver default', 'Pick columns…' }, {
    prompt = ('%s %s'):format(op_title(op), entry.name),
  }, function(choice)
    if not choice then return end
    if choice:sub(1, 3) == 'All' then return cb({}) end
    round()
  end)
end

--- Puts `text` under the cursor line of the SQL window, set off by blank
--- lines, with the cursor on its first line.
local function insert_statement(buf, text)
  local win = vim.fn.bufwinid(buf)
  if win == -1 then return notify('the SQL window is closed', vim.log.levels.WARN) end
  vim.api.nvim_set_current_win(win)
  local row = vim.api.nvim_win_get_cursor(win)[1]
  local lines = vim.split(text, '\n', { plain = true })
  local last = vim.api.nvim_buf_line_count(buf)
  local block = { '' }
  vim.list_extend(block, lines)
  if row < last then block[#block + 1] = '' end
  vim.api.nvim_buf_set_lines(buf, row, row, false, block)
  vim.api.nvim_win_set_cursor(win, { row + 2, 0 })
end

--- Generates `op` for `entry` into `buf` (the SQL buffer). `op` and the
--- column choice are asked for when not given.
function M.crud(buf, entry, op, columns)
  local win = vim.fn.bufwinid(buf)
  local conn = M.block_connection(buf, win ~= -1 and vim.api.nvim_win_get_cursor(win)[1] or 1)
  if not conn then return notify('no connection for this buffer -- :TradarConnect', vim.log.levels.WARN) end
  local function go(chosen_op, cols)
    call('snippet', { connection = conn, name = entry.name, schema = entry.schema, op = chosen_op, columns = cols }, function(r)
      if not r.text then
        return notify(('this connector has no %s statement for `%s`'):format(chosen_op, entry.name), vim.log.levels.WARN)
      end
      insert_statement(buf, r.text)
    end)
  end
  local function with_op(chosen_op)
    if columns then return go(chosen_op, columns) end
    pick_columns(entry, chosen_op, function(cols) go(chosen_op, cols) end)
  end
  if op then return with_op(op) end
  vim.ui.select(OPS, {
    prompt = 'Generate for ' .. entry.name,
    format_item = function(o) return ('%-7s %s'):format(o.label, o.hint) end,
  }, function(o)
    if o then with_op(o.op) end
  end)
end

--- `:TradarCrud [create|read|update|delete]`: for the table named under
--- the cursor, else pick one (telescope when installed).
function M.crud_command(op)
  if op and op ~= '' and not vim.tbl_contains({ 'create', 'read', 'update', 'delete' }, op) then
    return notify('usage: :TradarCrud [create|read|update|delete]', vim.log.levels.WARN)
  end
  op = (op ~= '' and op) or nil
  M.schema_entries(function(entries, buf)
    local here = M.table_at_cursor(entries)
    if here then return M.crud(buf, here, op) end
    if #entries == 0 then return notify('no tables / collections in this connection', vim.log.levels.WARN) end
    local render = ctx.render
    vim.ui.select(entries, {
      prompt = 'Generate for which table?',
      format_item = function(e) return render.qualified(e) end,
    }, function(e)
      if e then M.crud(buf, e, op) end
    end)
  end)
end

return M
