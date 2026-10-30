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

return M
