-- tradar.nvim: the editor is a real Neovim buffer, tradar-server does the
-- querying. See "Server headless" in docs/architecture.md.
local rpc = require('tradar.rpc')
local render = require('tradar.render')

local M = {}

local opts = {}
local state = {
  connection = nil, -- name of the active saved connection
  keywords = {},
  names = {}, -- table/column names, for omnifunc
  sql_buf = nil, -- last buffer a query was run from; where the navigator inserts into
  cursor = nil, -- { id, total, shown, kind, columns }
  results_buf = nil,
  schema_buf = nil,
  schema_targets = {},
}

local function notify(msg, level) vim.notify('tradar: ' .. msg, level or vim.log.levels.INFO) end

local function call(method, params, cb)
  rpc.ensure(opts, function(err)
    if err then return notify(err, vim.log.levels.ERROR) end
    rpc.request(method, params, function(e, result)
      if e then return notify(e, vim.log.levels.ERROR) end
      cb(result)
    end)
  end)
end

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
  if vim.fn.bufwinid(buf) == -1 then
    vim.cmd(('botright %dsplit'):format(height or 15))
    vim.api.nvim_win_set_buf(0, buf)
    vim.wo.wrap = false
  end
end

local function set_lines(buf, lines)
  vim.bo[buf].modifiable = true
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
  vim.bo[buf].modifiable = false
end

local function require_connection()
  if not state.connection then
    notify('no active connection -- :TradarConnect first', vim.log.levels.WARN)
    return false
  end
  return true
end

local function results_title(r, shown)
  local text = ('%d/%d rows%s'):format(shown, r.total, r.truncated and ' (truncated at server cap)' or '')
  return text .. (shown < r.total and '  — :TradarMore for the next page' or '')
end

local function show_result(r)
  local buf = scratch('tradar://results', state.results_buf)
  state.results_buf = buf
  if r.kind == 'affected' then
    state.cursor = nil
    set_lines(buf, { ('%d row(s) affected'):format(r.rows) })
    return show(buf, 4)
  end
  state.cursor = { id = r.cursor, total = r.total, shown = #r.rows, kind = r.kind, columns = r.columns }
  local lines = r.kind == 'table' and render.table(r.columns, r.rows) or render.documents(r.rows)
  lines[#lines + 1] = ''
  lines[#lines + 1] = results_title(r, #r.rows)
  set_lines(buf, lines)
  show(buf)
end

--- Opens a picker over saved connections, or connects to `name` directly.
function M.connect(name)
  local function go(chosen)
    call('connect', { connection = chosen }, function()
      state.connection = chosen
      call('keywords', { connection = chosen }, function(k) state.keywords = k end)
      call('schema', { connection = chosen }, function(entries)
        local names = {}
        for _, e in ipairs(entries) do
          names[#names + 1] = e.name
          for _, c in ipairs(e.columns or {}) do names[#names + 1] = c.name end
        end
        state.names = names
        notify(('connected to %s (%d objects)'):format(chosen, #entries))
      end)
    end)
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

local function run_text(text)
  if not require_connection() then return end
  state.sql_buf = vim.api.nvim_get_current_buf()
  call('execute', { connection = state.connection, query = text, page_size = opts.page_size or 200 }, show_result)
end

--- Runs the visual selection, else the statement under the cursor, else
--- (`all`) every statement in the buffer -- statement boundaries are the
--- driver's, not a regex here.
function M.run(range_start, range_end, all)
  if not require_connection() then return end
  local buf = vim.api.nvim_get_current_buf()
  if range_start then
    local lines = vim.api.nvim_buf_get_lines(buf, range_start - 1, range_end, false)
    return run_text(table.concat(lines, '\n'))
  end
  local text = table.concat(vim.api.nvim_buf_get_lines(buf, 0, -1, false), '\n')
  call('split', { connection = state.connection, text = text }, function(statements)
    if all then
      for _, s in ipairs(statements) do run_text(s.text) end
      return
    end
    -- Byte offset of the cursor in `text`.
    local row, col = unpack(vim.api.nvim_win_get_cursor(0))
    local offset = col
    for _, l in ipairs(vim.api.nvim_buf_get_lines(buf, 0, row - 1, false)) do offset = offset + #l + 1 end
    for _, s in ipairs(statements) do
      if offset >= s.start and offset <= s['end'] then return run_text(s.text) end
    end
    local last = statements[#statements]
    if last then run_text(last.text) else notify('nothing to run', vim.log.levels.WARN) end
  end)
end

--- Appends the next page to the results buffer.
function M.more()
  local c = state.cursor
  if not c or c.shown >= c.total then return notify('no more rows') end
  call('fetch', { cursor = c.id, offset = c.shown, limit = opts.page_size or 200 }, function(page)
    local buf = state.results_buf
    local lines = c.kind == 'table' and render.table(c.columns, page.rows) or render.documents(page.rows)
    if c.kind == 'table' then table.remove(lines, 1) table.remove(lines, 1) end -- header + rule already shown
    c.shown = c.shown + #page.rows
    local count = vim.api.nvim_buf_line_count(buf)
    vim.bo[buf].modifiable = true
    -- Replace the trailing blank + title with the new rows and a fresh title.
    vim.api.nvim_buf_set_lines(buf, count - 2, count, false, vim.list_extend(lines, { '', results_title({ total = c.total }, c.shown) }))
    vim.bo[buf].modifiable = false
  end)
end

--- Navigator: tables and columns of the active connection; <CR> inserts the
--- name under the cursor into the SQL buffer you came from.
function M.schema()
  if not require_connection() then return end
  call('schema', { connection = state.connection }, function(entries)
    local lines, targets = render.schema(entries)
    local buf = scratch('tradar://schema', state.schema_buf)
    state.schema_buf, state.schema_targets = buf, targets
    set_lines(buf, lines)
    vim.keymap.set('n', '<CR>', function()
      local name = state.schema_targets[vim.api.nvim_win_get_cursor(0)[1]]
      local target = state.sql_buf
      if not (name and target and vim.api.nvim_buf_is_valid(target)) then return end
      local win = vim.fn.bufwinid(target)
      if win == -1 then return notify('the SQL window is closed', vim.log.levels.WARN) end
      vim.api.nvim_set_current_win(win)
      vim.api.nvim_put({ name }, 'c', false, true)
    end, { buffer = buf, desc = 'insert name into the SQL buffer' })
    state.sql_buf = state.sql_buf or vim.api.nvim_get_current_buf()
    vim.cmd('topleft 40vsplit')
    vim.api.nvim_win_set_buf(0, buf)
    vim.wo.wrap = false
  end)
end

--- `omnifunc` / `completefunc`: keywords of the driver's own language plus
--- schema names. Context-aware ranking (FK joins, aliases) is a later step.
function M.omnifunc(findstart, base)
  if findstart == 1 then
    local col = vim.api.nvim_win_get_cursor(0)[2]
    local line = vim.api.nvim_get_current_line():sub(1, col)
    return col - #line:match('[%w_]*$')
  end
  local out, lower = {}, base:lower()
  local function add(list, kind)
    for _, w in ipairs(list) do
      if w:lower():sub(1, #lower) == lower then out[#out + 1] = { word = w, menu = kind } end
    end
  end
  add(state.names, '[schema]')
  add(state.keywords, '[kw]')
  return out
end

function M.setup(user)
  opts = user or {}
end

return M
