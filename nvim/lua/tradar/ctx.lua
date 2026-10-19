-- Shared context for the tradar plugin modules: the public table `M` that
-- `require('tradar')` returns, the `setup{}` options, the one `state` table,
-- and the few helpers every module needs. Sections that were one 2000-line
-- file live in sibling modules (`conn`, `results`, `running`, ...); a function
-- one module needs from another is reached as `ctx.name` at call time.
local rpc = require('tradar.rpc')
local gridedit = require('tradar.gridedit')
local render = require('tradar.render')
local guard = require('tradar.guard')
local blocks = require('tradar.blocks')
local uv = vim.uv or vim.loop

local ctx = {}
local M = {}
local opts = {}
local ns = vim.api.nvim_create_namespace('tradar')

--- Rows fetched per page (`setup{ page_size = N }`); the next page is an
--- explicit `<leader>rm`, never a side effect of scrolling.
local DEFAULT_PAGE = 20
local SPINNER = { '⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏' }
local HISTORY_MAX = 200

local state = {
  default = nil, -- last connection chosen explicitly; fallback for unbound buffers
  connected = {}, -- name -> true once the server confirmed `connect`
  connecting = {}, -- name -> callbacks waiting on an in-flight `connect`
  alive = {}, -- name -> bool, from the background `status` poll
  in_tx = {}, -- name -> bool
  history = nil, -- lazily loaded
  running = nil, -- { id, conn, started }
  last = nil, -- summary of the last finished query, for the statusline
  counter = 0,
  spinner = nil, -- uv timer while a query runs
  poller = nil, -- uv timer for status polling
  result = nil, -- the cursor currently shown in the results buffer
  results_buf = nil,
  schema_buf = nil,
  schema_entries = nil, -- cached so a fold toggle can re-render without another `schema` call
  schema_nodes = {}, -- parallel to the schema buffer's lines, from the last render
  -- Keyed by a node's own key (see `render.schema_tree`), not by connection
  -- -- the schema buffer is shared across connections (same name, reused),
  -- so switching connections can coincidentally show a same-named table
  -- pre-expanded from a previous one. Cosmetic only, not worth a
  -- per-connection key for.
  schema_expanded = {},
  sql_buf = nil, -- buffer the navigator inserts into
  drivers = nil, -- connection name -> driver id (from connections.list)
  schema_cache = {}, -- name -> { at, entries }
  edit_cache = nil, -- last successful `resolve_edit`, so re-running the same query is editable at once
}

local function notify(msg, level) vim.notify('tradar: ' .. msg, level or vim.log.levels.INFO) end

--- `g?`: a floating cheat-sheet of the current buffer's own tradar keymap
--- -- the bare `g`-prefixed/vim-like keys here (`gyc`, `i`, `dd`, the
--- schema panel's `<CR>`/`<Tab>`...) have no which-key entry the way the
--- `<leader>r*` family does (`M.setup`'s `wk.add` only covers that one
--- prefix), so a first-time user otherwise has no way to discover them
--- short of reading the README.
local function show_help(lines)
  vim.lsp.util.open_floating_preview(lines, 'markdown', { border = 'rounded', focus_id = 'tradar_help', max_width = 90 })
end

local function call(method, params, cb, on_error)
  local function attempt(retried)
    rpc.ensure(opts, function(err)
      if err then
        if on_error then return on_error(err) end
        return notify(err, vim.log.levels.ERROR)
      end
      rpc.request(method, params, function(e, result, data)
        if e then
          -- The server was restarted (or crashed) behind our back: it no
          -- longer knows a connection we still believe in. Connect again and
          -- repeat the call once, instead of making the user retry by hand.
          local conn = type(params) == 'table' and params.connection or nil
          if conn and not retried and method ~= 'connect' and state.connected[conn]
              and e:find('not connected', 1, true) then
            state.connected[conn] = nil
            return ctx.ensure_connected(conn, function() attempt(true) end, on_error or function(err2) notify(err2, vim.log.levels.ERROR) end)
          end
          if on_error then return on_error(e, data) end
          return notify(e, vim.log.levels.ERROR)
        end
        cb(result)
      end)
    end)
  end
  attempt(false)
end

local function resolve_buf(buf)
  if buf == nil or buf == 0 then return vim.api.nvim_get_current_buf() end
  return buf
end

ctx.M = M
ctx.rpc, ctx.gridedit, ctx.render, ctx.guard, ctx.blocks, ctx.uv = rpc, gridedit, render, guard, blocks, uv
ctx.opts, ctx.ns, ctx.state = opts, ns, state
ctx.DEFAULT_PAGE, ctx.SPINNER, ctx.HISTORY_MAX = DEFAULT_PAGE, SPINNER, HISTORY_MAX
ctx.notify, ctx.show_help, ctx.call, ctx.resolve_buf = notify, show_help, call, resolve_buf

return ctx
