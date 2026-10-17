-- HTTP over `tradar-server` (`http.*` methods,
-- docs/backlog/http-server-2026-10-17.md): a Postman-style request/response,
-- no push channel needed -- same reasoning as `tradar.rabbitmq`: the server
-- side is a plain `send()` call, not a stream. Not a sidebar-based screen
-- like the TUI's own `HttpScreen`: the request is an editable buffer
-- (`METHOD url` / headers / blank line / body, see `M.parse`), like the gRPC
-- request buffer.
local rpc = require('tradar.rpc')

local M = {}

local state = { current = nil }

local function notify(msg, level) vim.notify('tradar-http: ' .. msg, level or vim.log.levels.INFO) end

local function call(method, params, cb)
  rpc.ensure(require('tradar').rpc_opts(), function(err)
    if err then return notify(err, vim.log.levels.ERROR) end
    rpc.request(method, params, function(e, result)
      if e then return notify(e, vim.log.levels.ERROR) end
      cb(result)
    end)
  end)
end

--- Same "last picked, else prompt" precedence as `tradar.rabbitmq`'s
--- `with_connection` -- connects first (idempotent server-side, no
--- liveness probe: a base URL may be empty or not live yet).
local function with_connection(name, cb)
  local function go(conn)
    state.current = conn
    call('http.connect', { connection = conn }, function() cb(conn) end)
  end
  if name and name ~= '' then return go(name) end
  if state.current then return go(state.current) end
  call('connections.list', nil, function(list)
    local usable = vim.tbl_filter(function(c) return c.driver == 'http' end, list)
    if #usable == 0 then
      return notify('no saved connection with driver = "http" -- add one with :TradarConnections', vim.log.levels.WARN)
    end
    vim.ui.select(usable, {
      prompt = 'tradar HTTP connection',
      format_item = function(c) return ('%s  (%s)%s'):format(c.name, c.target, c.connected and '  ●' or '') end,
    }, function(c) if c then go(c.name) end end)
  end)
end

function M.connect(name) with_connection(name, function(conn) notify('connected to ' .. conn) end) end

local METHODS = { 'GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS' }

--- One scratch, non-modifiable, reused split for the last response --
--- same role as `tradar.rabbitmq`'s own browse split.
local response_buf = nil
local function show_response(title, lines)
  if not (response_buf and vim.api.nvim_buf_is_valid(response_buf)) then
    response_buf = vim.api.nvim_create_buf(false, true)
    vim.bo[response_buf].bufhidden = 'hide'
    vim.bo[response_buf].swapfile = false
    pcall(vim.api.nvim_buf_set_name, response_buf, 'tradar://http')
    vim.keymap.set('n', 'q', function()
      local win = vim.fn.bufwinid(response_buf)
      if win ~= -1 then vim.api.nvim_win_close(win, true) end
    end, { buffer = response_buf, desc = 'tradar: close' })
    vim.keymap.set('n', 'g?', function()
      vim.lsp.util.open_floating_preview(
        { '# tradar: HTTP response buffer', '- `q` close' },
        'markdown', { border = 'rounded', focus_id = 'tradar_help', max_width = 90 }
      )
    end, { buffer = response_buf, desc = 'tradar: show keymap help' })
  end
  vim.bo[response_buf].modifiable = true
  vim.api.nvim_buf_set_lines(response_buf, 0, -1, false, lines)
  vim.bo[response_buf].modifiable = false
  local win = vim.fn.bufwinid(response_buf)
  if win == -1 then
    local origin = vim.api.nvim_get_current_win()
    vim.cmd('botright 15split')
    win = vim.api.nvim_get_current_win()
    vim.api.nvim_win_set_buf(win, response_buf)
    vim.api.nvim_set_current_win(origin)
  end
  notify(title)
end

--- Splits a request buffer into `method, url, headers, body`:
---
---     # comment lines (before the blank line) are ignored
---     POST /users
---     Content-Type: application/json
---
---     {"name": "ann"}
---
--- The first non-comment line is `METHOD url` (a bare URL means GET); header
--- lines run to the first blank line; everything after it is the body.
--- `headers` is the newline-separated `Key: Value` text `http.send` takes.
--- Returns `nil, err` for a request line with no URL.
function M.parse(lines)
  local request, headers, i = nil, {}, 1
  while i <= #lines do
    local line = lines[i]
    i = i + 1
    if line:match('^%s*$') then
      if request then break end
    elseif not line:match('^%s*#') then
      if not request then
        request = vim.trim(line)
      else
        headers[#headers + 1] = vim.trim(line)
      end
    end
  end
  if not request then return nil, 'empty request — first line is `METHOD url`' end
  local method, url = request:match('^(%a+)%s+(%S.*)$')
  method = method and method:upper()
  local known = false
  for _, m in ipairs(METHODS) do known = known or m == method end
  if not known then method, url = 'GET', request end
  if not url or url == '' then return nil, 'no URL on the request line' end
  local body = table.concat(vim.list_slice(lines, i), '\n')
  return method, url, table.concat(headers, '\n'), body
end

local TEMPLATE = {
  '# METHOD url, then Key: Value headers, a blank line, then the body.',
  '# <leader>rr (or :TradarHttpSend) sends it.',
  'GET /',
  'Accept: application/json',
  '',
}

--- Sends the request buffer `buf` on its connection; the response lands in
--- the shared response split.
function M.submit(buf)
  local conn = vim.b[buf].tradar_http and vim.b[buf].tradar_http.connection
  if not conn then return notify('not a tradar HTTP request buffer', vim.log.levels.WARN) end
  local method, url, headers, body = M.parse(vim.api.nvim_buf_get_lines(buf, 0, -1, false))
  if not method then return notify(url, vim.log.levels.WARN) end
  call('http.send', { connection = conn, method = method, url = url, headers = headers, body = body }, function(response)
    local lines = { ('%s %d %s  (%dms)'):format(method, response.status, response.status_text, response.elapsed_ms) }
    for _, h in ipairs(response.headers) do
      lines[#lines + 1] = ('%s: %s'):format(h[1], h[2])
    end
    lines[#lines + 1] = ''
    vim.list_extend(lines, vim.split(response.body, '\n', { plain = true }))
    show_response(('%s: %s %s -> %d'):format(conn, method, url, response.status), lines)
  end)
end

local request_buffers = {} -- connection name -> its request buffer

--- Opens (or jumps back to) the editable request buffer for `conn`. One per
--- connection, kept while Neovim runs, so the last request is still there to
--- tweak and resend.
function M.open_request(conn)
  local existing = request_buffers[conn]
  if existing and vim.api.nvim_buf_is_valid(existing) then
    local win = vim.fn.bufwinid(existing)
    if win ~= -1 then
      vim.api.nvim_set_current_win(win)
    else
      vim.cmd.tabnew()
      vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), existing)
    end
    return
  end
  local buf = vim.api.nvim_create_buf(true, true)
  request_buffers[conn] = buf
  vim.bo[buf].bufhidden = 'hide'
  vim.bo[buf].swapfile = false
  vim.bo[buf].filetype = 'http'
  pcall(vim.api.nvim_buf_set_name, buf, ('tradar-http://%s [%d]'):format(conn, buf))
  vim.b[buf].tradar_http = { connection = conn }
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, TEMPLATE)
  local p = require('tradar').rpc_opts().prefix or '<leader>r'
  vim.api.nvim_buf_create_user_command(buf, 'TradarHttpSend', function() M.submit(buf) end, {})
  vim.keymap.set('n', p .. 'r', function() M.submit(buf) end, { buffer = buf, desc = 'tradar: send HTTP request' })
  -- No `q`: this buffer is editable (see the gRPC request buffer).
  vim.keymap.set('n', 'g?', function()
    vim.lsp.util.open_floating_preview({
      '# tradar: HTTP request buffer',
      ('- `%sr` / `:TradarHttpSend` send this request'):format(p),
      '- line 1: `METHOD url` (bare URL = GET; a relative URL is joined to the connection\'s base)',
      '- then `Key: Value` header lines, a blank line, the body',
      '- `#` lines before the blank line are comments',
    }, 'markdown', { border = 'rounded', focus_id = 'tradar_help', max_width = 90 })
  end, { buffer = buf, desc = 'tradar: show keymap help' })
  vim.cmd.tabnew()
  vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), buf)
  vim.api.nvim_win_set_cursor(0, { 3, 4 })
end

--- `:TradarHttpSend [connection]`: inside a request buffer it sends that
--- buffer; elsewhere it opens the connection's request buffer to edit.
function M.send(name)
  if (name == nil or name == '') and vim.b.tradar_http then return M.submit(vim.api.nvim_get_current_buf()) end
  with_connection(name, M.open_request)
end

return M
