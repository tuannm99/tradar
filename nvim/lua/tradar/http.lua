-- HTTP over `tradar-server` (`http.*` methods,
-- docs/backlog/http-server-2026-10-17.md): a Postman-style request/response,
-- no push channel needed -- same reasoning as `tradar.rabbitmq`: the server
-- side is a plain `send()` call, not a stream. Not a sidebar-based screen
-- like the TUI's own `HttpScreen`: one command walks method/URL/headers/body
-- through `vim.ui.select`/`vim.ui.input`, same idiom `tradar.rabbitmq`'s
-- `publish` already uses for "prompt a few fields, then send".
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

--- `:TradarHttpSend [connection]` -- walks method (`vim.ui.select`), URL,
--- headers, and body one `vim.ui.input` at a time, same style
--- `tradar.rabbitmq`'s `publish` already uses. Headers are entered as
--- `Key: Value` pairs separated by `;` on the one input line (`vim.ui.input`
--- has no multi-line form) -- split into the newline-per-header shape
--- `http.send`'s own `headers` param expects before sending.
function M.send(name)
  with_connection(name, function(conn)
    vim.ui.select(METHODS, { prompt = 'Method' }, function(method)
      if not method then return end
      vim.ui.input({ prompt = 'URL: ' }, function(url)
        if url == nil then return end
        vim.ui.input({ prompt = 'Headers (Key: Value; Key2: Value2, blank = none): ' }, function(headers_raw)
          if headers_raw == nil then return end
          local headers = table.concat(vim.split(headers_raw, ';', { plain = true, trimempty = true }), '\n')
          vim.ui.input({ prompt = 'Body (blank = none): ' }, function(body)
            if body == nil then return end
            call('http.send', { connection = conn, method = method, url = url, headers = headers, body = body }, function(response)
              local lines = { ('%s %d %s  (%dms)'):format(method, response.status, response.status_text, response.elapsed_ms) }
              for _, h in ipairs(response.headers) do
                lines[#lines + 1] = ('%s: %s'):format(h[1], h[2])
              end
              lines[#lines + 1] = ''
              vim.list_extend(lines, vim.split(response.body, '\n', { plain = true }))
              show_response(('%s: %s %s -> %d'):format(conn, method, url, response.status), lines)
            end)
          end)
        end)
      end)
    end)
  end)
end

return M
