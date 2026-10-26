-- The connection's cheat sheet: ready-made queries in the driver's own
-- language (what is in the database, what is running, how big things are,
-- the usual shapes of a query), kept by the server. Picked from a list with
-- a preview and put into the buffer; `${table}`-style placeholders are asked
-- for when the query runs (see `params.lua`).
local ctx = require('tradar.ctx')
local M = ctx.M
local notify = ctx.notify
local call = ctx.call

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
  local conn = M.block_connection(buf, vim.api.nvim_win_get_cursor(0)[1])
  if not conn then return notify('no connection for this buffer -- :TradarConnect', vim.log.levels.WARN) end
  ctx.ensure_connected(conn, function()
    call('cheatsheet', { connection = conn }, function(r)
      local entries = r.entries or {}
      if #entries == 0 then return notify(('no cheat sheet for the `%s` driver yet'):format(M.cached_driver(conn) or conn), vim.log.levels.WARN) end
      local tele = require('tradar.telescope')
      if ctx.opts.cheatsheet_picker ~= 'ui' and tele.available() then
        return tele.cheatsheet(entries, M.cached_driver(conn), function(entry, run) put(buf, entry, run) end)
      end
      vim.ui.select(entries, {
        prompt = 'tradar cheat sheet',
        format_item = function(e) return ('%-12s %s'):format(e.category, e.title) end,
      }, function(e)
        if e then put(buf, e, false) end
      end)
    end)
  end, function(err) notify(err, vim.log.levels.ERROR) end)
end

return M
