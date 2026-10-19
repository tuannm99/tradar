-- tradar.nvim: the editor is a real Neovim buffer, tradar-server does the
-- querying. See "Server headless" in docs/architecture.md.
--
-- This file wires the pieces together; the work lives in sibling modules that
-- add their functions to the same public table (`require('tradar.ctx').M`):
--   conn      connection binding, ensure_connected, statusline, spinner
--   history   the per-user query history
--   results   the results buffer: paint, paging, zoom, yank/export
--   resultskeys   the results buffer's keymaps
--   running   running statements (selection, diagnostics, guard)
--   connfile  editing connections.toml
--   nav       schema panel, hover, go-to-table, follow foreign key
--   rowedit   editing rows in the results grid, commit preview, EXPLAIN
--   complete  completion
local ctx = require('tradar.ctx')
local M = ctx.M
local opts = ctx.opts
local state = ctx.state
local notify = ctx.notify
local show_help = ctx.show_help
local resolve_buf = ctx.resolve_buf

require('tradar.conn')
require('tradar.history')
require('tradar.results')
require('tradar.resultskeys')
require('tradar.running')
require('tradar.connfile')
require('tradar.nav')
require('tradar.rowedit')
require('tradar.complete')

-- ── buffer setup ─────────────────────────────────────────────────────────

--- Called for every SQL buffer (and any other buffer that names a connection
--- in a `-- tradar: name` / `// tradar: name` / `# tradar: name` line): keymaps, omnifunc, and a quiet background
--- connect so the first completion/run has nothing to wait for.
function M.attach(buf)
  buf = resolve_buf(buf)
  vim.b[buf].tradar_attached = true
  vim.bo[buf].omnifunc = "v:lua.require'tradar'.omnifunc"
  if opts.keymaps ~= false then
    local p = opts.prefix or '<leader>r'
    local function map(mode, suffix, rhs, desc)
      vim.keymap.set(mode, p .. suffix, rhs, { buffer = buf, desc = 'tradar: ' .. desc })
    end
    map('n', 'r', function() M.run() end, 'run statement under cursor')
    map('x', 'r', function()
      local a, b = vim.fn.line('v'), vim.fn.line('.')
      vim.api.nvim_feedkeys(vim.keycode('<Esc>'), 'nx', false)
      M.run(math.min(a, b), math.max(a, b))
    end, 'run selection')
    map('n', 'a', function() M.run(nil, nil, true) end, 'run all statements')
    map('n', 'x', M.cancel, 'cancel running query')
    map('n', 'c', function() M.connect() end, 'choose connection')
    map('n', 's', M.schema, 'schema panel')
    map('n', 't', function() require('tradar.telescope').tables() end, 'tables (telescope)')
    map('n', 'h', function() require('tradar.telescope').history() end, 'history (telescope)')
    map('n', 'm', function() M.more() end, 'load more rows')
    map('n', 'z', function() M.zoom() end, 'zoom the results window to full screen / restore')
    map('n', 'e', M.explain, 'explain statement under cursor')
    vim.keymap.set('n', 'g?', function()
      show_help({
        '# tradar: SQL buffer',
        ('- `%sr` (n/x) run statement under cursor / selection'):format(p),
        ('- `%sa` / `%sx` run all / cancel'):format(p, p),
        ('- `%sc` / `%ss` choose connection / schema panel'):format(p, p),
        ('- `%st` / `%sh` tables / history (telescope)'):format(p, p),
        ('- `%sm` load the next page of rows (scrolling does not)'):format(p),
        ('- `%sz` zoom the results window to the full screen / restore'):format(p),
        ('- `%se` explain statement under cursor'):format(p),
        '- `K` table/column info under cursor (falls back to LSP hover)',
        '- `gd` open table under cursor (falls back to LSP go-to-definition)',
      })
    end, { buffer = buf, desc = 'tradar: ctx.show keymap help' })
  end
  M.attach_nav(buf)
  local conn = M.connection_for(buf)
  if conn and not state.connected[conn] then
    ctx.ensure_connected(conn, function() end, function(err)
      notify(('could not connect `%s`: %s'):format(conn, err), vim.log.levels.WARN)
    end)
  end
end

--- `K`/`gd` on SQL buffers. Split from `attach` because an LSP's own
--- `on_attach` (sqlls, here) sets the same keys after FileType and would
--- win: the LspAttach hook in plugin/tradar.lua calls this again afterwards.
function M.attach_nav(buf)
  buf = resolve_buf(buf)
  if opts.keymaps == false then return end
  vim.keymap.set('n', 'K', M.hover, { buffer = buf, desc = 'tradar: table/column info' })
  vim.keymap.set('n', 'gd', M.goto_table, { buffer = buf, desc = 'tradar: open table under cursor' })
end

--- Attaches `buf` if it is SQL or declares a connection in its first lines --
--- how a Mongo (`.mongo`), Redis, Elasticsearch or any other file opts in.
function M.maybe_attach(buf)
  buf = resolve_buf(buf)
  if vim.b[buf].tradar_attached then return end
  if vim.bo[buf].filetype == 'sql' or M.is_tdb(buf) or ctx.modeline(buf) then M.attach(buf) end
end

function M.setup(user)
  -- In place: the other modules hold this very table.
  for k in pairs(opts) do opts[k] = nil end
  for k, v in pairs(user or {}) do opts[k] = v end
  local ok, wk = pcall(require, 'which-key')
  if ok and wk.add then wk.add({ { opts.prefix or '<leader>r', group = 'database (tradar)' } }) end
  -- Buffers that were read before setup() ran (a lazy-loaded plugin) missed
  -- their FileType/BufReadPost events.
  for _, b in ipairs(vim.api.nvim_list_bufs()) do
    if vim.api.nvim_buf_is_loaded(b) then M.maybe_attach(b) end
  end
end

--- The `setup{}` table (socket path, `server_cmd`, ...), so other modules
--- that talk to the same `tradar-server` (currently just `tradar.grpc`)
--- connect to the same socket instead of falling back to `rpc.ensure`'s
--- own defaults.
function M.rpc_opts() return opts end

return M
