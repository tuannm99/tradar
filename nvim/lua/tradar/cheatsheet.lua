-- The connection's cheat sheet: ready-made queries in the driver's own
-- language (what is in the database, what is running, how big things are,
-- the usual shapes of a query), kept by the server. Picked from a list with
-- a preview and put into the buffer; `${table}`-style placeholders are asked
-- for when the query runs (see `params.lua`).
local ctx = require('tradar.ctx')
local M = ctx.M
local notify = ctx.notify
local call = ctx.call

local TEMPLATE = {
  '# tradar cheat sheet -- your own queries, shown first in :TradarCheatsheet',
  '# next to the built-in ones. Read on every use: saving is all it takes.',
  '#',
  '# [[entry]]',
  '# driver = "postgres"        # optional: only connections of this driver',
  '# connection = "prod"        # optional: only this connection',
  '# category = "Mine"          # optional, default "Mine"',
  '# title = "Orders in the last hour"',
  '# text = """',
  "# select * from orders where created_at > now() - interval '1 hour'",
  '# """',
  '# # ${name} in the text is asked for when the query runs.',
  '',
}

--- `:TradarCheatsheetEdit`: opens `cheatsheet.toml` (beside
--- `connections.toml`) -- a commented template when it does not exist yet.
function M.edit_cheatsheet()
  call('connections.path', nil, function(res)
    if not res.path then return notify('the server has no config directory', vim.log.levels.WARN) end
    local path = res.path:gsub('[^/\\]*$', 'cheatsheet.toml')
    vim.fn.mkdir(vim.fn.fnamemodify(path, ':h'), 'p')
    vim.cmd.edit(vim.fn.fnameescape(path))
    local buf = vim.api.nvim_get_current_buf()
    if vim.fn.filereadable(path) == 0 and vim.api.nvim_buf_line_count(buf) == 1
        and vim.api.nvim_buf_get_lines(buf, 0, 1, false)[1] == '' then
      vim.api.nvim_buf_set_lines(buf, 0, -1, false, TEMPLATE)
    end
    if vim.bo[buf].filetype == '' then vim.bo[buf].filetype = 'toml' end
  end)
end

local function put(buf, entry, run)
  ctx.insert_statement(buf, entry.text)
  if run then M.run() end
end

--- `:TradarCheatsheet` / `<leader>rk`: pick a query for this buffer's
--- connection. `<CR>` puts it in the buffer, `<C-r>` puts it there and runs
--- it. Without telescope (or `setup{ cheatsheet_picker = 'ui' }`) a plain
--- list, which only inserts.
function M.cheatsheet()
  local buf = vim.api.nvim_get_current_buf()
  if require('tradar.restbench').applies(buf) then return require('tradar.restbench').cheatsheet(buf) end
  local conn = M.block_connection(buf, vim.api.nvim_win_get_cursor(0)[1])
  if not conn then return notify('no connection for this buffer -- :TradarConnect', vim.log.levels.WARN) end
  ctx.ensure_connected(conn, function()
    call('cheatsheet', { connection = conn }, function(r)
      local entries = r.entries or {}
      for _, w in ipairs(r.warnings or {}) do notify(w, vim.log.levels.WARN) end
      if #entries == 0 then return notify(('no cheat sheet for the `%s` driver yet'):format(M.cached_driver(conn) or conn), vim.log.levels.WARN) end
      local tele = require('tradar.telescope')
      if ctx.opts.cheatsheet_picker ~= 'ui' and tele.available() then
        return tele.cheatsheet(entries, M.cached_driver(conn), function(entry, run) put(buf, entry, run) end, { on_edit = M.edit_cheatsheet })
      end
      vim.ui.select(entries, {
        prompt = 'tradar cheat sheet',
        format_item = function(e) return ('%-12s %s'):format((e.user and '★ ' or '') .. e.category, e.title) end,
      }, function(e)
        if e then put(buf, e, false) end
      end)
    end)
  end, function(err) notify(err, vim.log.levels.ERROR) end)
end

-- ── the whole sheet in one buffer ────────────────────────────────────────

local FENCE = { mongo = 'mongo', elasticsearch = 'esq', redis = 'redis', http = 'http', grpc = 'grpc' }

--- Folds of the overview buffer: `## category` is level 1, `### entry` level 2.
function M.foldexpr(lnum)
  local line = vim.fn.getline(lnum)
  if line:match('^## ') then return '>1' end
  if line:match('^### ') then return '>2' end
  return '='
end

--- The cheat sheet as the text of a `.tdb`: a heading per category, a
--- heading and a fenced block per entry -- so the block under the cursor runs
--- with `<leader>rr` on the connection named in its fence.
local function render(entries, conn, driver)
  local lang = FENCE[driver] or 'sql'
  local p = ctx.opts.prefix or '<leader>r'
  local lines = {
    ('# Cheat sheet -- %s (%s)'):format(conn or driver, driver),
    ('# %srr runs the block under the cursor · zM folds to the headings, zR opens all · / searches · :w path.tdb keeps a copy'):format(p),
    '',
  }
  local order, groups = {}, {}
  for _, e in ipairs(entries) do
    if not groups[e.category] then
      groups[e.category] = {}
      order[#order + 1] = e.category
    end
    table.insert(groups[e.category], e)
  end
  for _, category in ipairs(order) do
    lines[#lines + 1] = '## ' .. category
    lines[#lines + 1] = ''
    for _, e in ipairs(groups[category]) do
      lines[#lines + 1] = '### ' .. (e.user and '★ ' or '') .. e.title
      lines[#lines + 1] = '```' .. lang .. (conn and (' ' .. conn) or '')
      vim.list_extend(lines, vim.split(e.text, '\n', { plain = true }))
      lines[#lines + 1] = '```'
      lines[#lines + 1] = ''
    end
  end
  return lines
end

local function show_sheet(entries, conn, driver)
  local name = 'tradar://cheatsheet/' .. (conn or driver)
  local buf = vim.fn.bufnr(name)
  local fresh = buf == -1 or not vim.api.nvim_buf_is_valid(buf)
  if fresh then
    buf = vim.api.nvim_create_buf(true, true)
    vim.bo[buf].bufhidden = 'hide'
    vim.bo[buf].swapfile = false
    vim.api.nvim_buf_set_name(buf, name)
  end
  -- Edits are kept: only a sheet nobody touched is drawn again.
  if fresh or not vim.bo[buf].modified then
    vim.api.nvim_buf_set_lines(buf, 0, -1, false, render(entries, conn, driver))
    vim.bo[buf].modified = false
  end
  vim.bo[buf].filetype = 'tdb'
  local win = vim.fn.bufwinid(buf)
  if win == -1 then
    vim.cmd.tabnew()
    win = vim.api.nvim_get_current_win()
    vim.api.nvim_win_set_buf(win, buf)
  else
    vim.api.nvim_set_current_win(win)
  end
  vim.wo[win].foldmethod = 'expr'
  vim.wo[win].foldexpr = "v:lua.require'tradar.cheatsheet'.foldexpr(v:lnum)"
  vim.wo[win].foldlevel = 99
  vim.api.nvim_win_set_cursor(win, { 1, 0 })
  return buf
end

--- `:TradarCheatsheetFile` / `<leader>rK`: the whole cheat sheet of this
--- connection in one buffer to read, search and fold -- every entry under its
--- category, each a block that runs from there (`${name}` asked as usual). The
--- same text as the picker, your own entries (★) included.
function M.cheatsheet_file()
  local buf = vim.api.nvim_get_current_buf()
  local rest = require('tradar.restbench')
  if rest.applies(buf) then
    local target = rest.describe(buf)
    return show_sheet(require('tradar.reqcheats').for_driver(target.driver), target.conn, target.driver)
  end
  local conn = M.block_connection(buf, vim.api.nvim_win_get_cursor(0)[1])
  if not conn then return notify('no connection for this buffer -- :TradarConnect', vim.log.levels.WARN) end
  ctx.ensure_connected(conn, function()
    call('cheatsheet', { connection = conn }, function(r)
      for _, w in ipairs(r.warnings or {}) do notify(w, vim.log.levels.WARN) end
      local entries = r.entries or {}
      local driver = M.cached_driver(conn) or 'sql'
      if #entries == 0 then return notify(('no cheat sheet for the `%s` driver yet'):format(driver), vim.log.levels.WARN) end
      show_sheet(entries, conn, driver)
    end)
  end, function(err) notify(err, vim.log.levels.ERROR) end)
end

return M
