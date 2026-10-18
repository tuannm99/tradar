-- gRPC: connect, browse service/method via server reflection (or a loaded
-- `.proto` when the target doesn't support reflection), invoke -- unary
-- only, per the v1 scope (see docs/architecture.md, "Chốt lại scope v1"
-- under gRPC). Talks to the `grpc.*` methods added to tradar-server
-- (docs/backlog/grpc-rpc-methods-2026-10-14.md); this module owns no
-- connection state of its own beyond "which connection did we last pick",
-- the server already keeps the channel + schema.
--
-- Unlike `init.lua`'s SQL/document buffers, a gRPC request has no source
-- file it's attached to -- a service/method is picked from a list, not
-- typed into a buffer tradar is watching. So this module writes its own
-- small scratch-buffer helpers rather than reusing `init.lua`'s (which are
-- wired to the "one persistent results buffer per Neovim instance" model
-- query screens use); a request buffer here is disposable, one per
-- method, same spirit as why Kafka/RabbitMQ/HTTP/Socket in the TUI don't
-- reuse `QueryScreenComponent`.
local rpc = require('tradar.rpc')

local M = {}

local state = { current = nil, next_id = 0 }

--- `connection/service/method` -> request buffer already open for it, so
--- re-picking the same method jumps back to that buffer instead of piling
--- up another disposable tab every time (a UX gap found reviewing the
--- plugin -- browsing several methods in a row used to leave one tab per
--- look, none of them closed automatically).
local open_buffers = {}

local function notify(msg, level) vim.notify('tradar-grpc: ' .. msg, level or vim.log.levels.INFO) end

local function call(method, params, cb)
  rpc.ensure(require('tradar').rpc_opts(), function(err)
    if err then return notify(err, vim.log.levels.ERROR) end
    rpc.request(method, params, function(e, result, data)
      if e then
        local suffix = data and data.grpc_code and (' (grpc_code=' .. tostring(data.grpc_code) .. ')') or ''
        return notify(e .. suffix, vim.log.levels.ERROR)
      end
      cb(result)
    end)
  end)
end

--- The connection a `services()`/`methods()` call with no explicit name
--- should act on: the request buffer under the cursor's own connection
--- first (so re-browsing from inside a request buffer for "staging" never
--- silently acts on "prod" just because that was connected most recently
--- elsewhere), then the last connection picked anywhere, then a prompt.
local function implicit_connection()
  local info = vim.b[0] and vim.b[0].tradar_grpc
  return (info and info.connection) or state.current
end

--- Picks a saved connection with `driver == "grpc"` if `name` is omitted,
--- then connects (idempotent server-side: a second `grpc.connect` on an
--- already-open connection just answers `already: true`, so every public
--- function below calls this first instead of tracking "is it live" here).
local function with_connection(name, cb)
  if name and name ~= '' then
    state.current = name
    return call('grpc.connect', { connection = name }, function() cb(name) end)
  end
  local implicit = implicit_connection()
  if implicit then
    state.current = implicit
    return call('grpc.connect', { connection = implicit }, function() cb(implicit) end)
  end
  call('connections.list', nil, function(list)
    local usable = vim.tbl_filter(function(c) return c.driver == 'grpc' end, list)
    if #usable == 0 then
      return notify('no saved connection with driver = "grpc" -- add one with :TradarConnections', vim.log.levels.WARN)
    end
    vim.ui.select(usable, {
      prompt = 'tradar gRPC connection',
      format_item = function(c) return ('%s  (%s)%s'):format(c.name, c.target, c.connected and '  ●' or '') end,
    }, function(c)
      if not c then return end
      state.current = c.name
      call('grpc.connect', { connection = c.name }, function() cb(c.name) end)
    end)
  end)
end

function M.connect(name) with_connection(name, function(conn) notify('connected to ' .. conn) end) end

function M.load_proto(name, path)
  if not path or path == '' then return notify('usage: :TradarGrpcLoadProto <path/to/file.proto>', vim.log.levels.WARN) end
  with_connection(name, function(conn)
    call('grpc.load_proto', { connection = conn, path = vim.fn.fnamemodify(path, ':p') }, function(result)
      notify(('%s: added %d service(s): %s'):format(conn, #result.added, table.concat(result.added, ', ')))
    end)
  end)
end

--- Lists a service's methods and lets the user pick one to open a request
--- buffer for -- the one piece reused by both `TradarGrpcServices` (after
--- picking a service) and `TradarGrpcMethods` (service named directly).
local function pick_method(conn, service)
  call('grpc.methods', { connection = conn, service = service }, function(methods)
    if #methods == 0 then return notify(service .. ' has no methods', vim.log.levels.WARN) end
    vim.ui.select(methods, {
      prompt = service,
      format_item = function(m) return ('%s(%s) -> %s'):format(m.name, m.input, m.output) end,
    }, function(m) if m then M.open_request(conn, service, m.name) end end)
  end)
end

function M.methods(name, service)
  if not service or service == '' then return notify('usage: :TradarGrpcMethods <service>', vim.log.levels.WARN) end
  with_connection(name, function(conn) pick_method(conn, service) end)
end

function M.services(name)
  with_connection(name, function(conn)
    call('grpc.services', { connection = conn }, function(services)
      if #services == 0 then
        return notify(
          conn .. ' has no known services yet -- the server may not support reflection; try :TradarGrpcLoadProto',
          vim.log.levels.WARN
        )
      end
      vim.ui.select(services, { prompt = conn }, function(s) if s then pick_method(conn, s) end end)
    end)
  end)
end

local function scratch(name)
  local buf = vim.api.nvim_create_buf(false, true)
  vim.bo[buf].bufhidden = 'wipe'
  vim.bo[buf].swapfile = false
  -- Several invocations of the same method would collide on the exact same
  -- buffer name; nvim_create_buf -> set_name with a unique suffix avoids
  -- ever having to reuse (and therefore stomp on) someone's in-progress
  -- edit the way `init.lua`'s one shared results buffer deliberately does.
  pcall(vim.api.nvim_buf_set_name, buf, name .. ' [' .. buf .. ']')
  return buf
end

--- One scratch window a response (or a batch of responses) to a request
--- buffer can land in while the user keeps editing/re-sending that buffer
--- -- not shared with the SQL/document results buffer, same reasoning as
--- the module doc comment.
local function show_response(request_buf, text)
  local resp_buf = vim.b[request_buf].tradar_grpc_response
  if not (resp_buf and vim.api.nvim_buf_is_valid(resp_buf)) then
    resp_buf = scratch('tradar://grpc-response')
    vim.bo[resp_buf].filetype = 'json'
    vim.b[request_buf].tradar_grpc_response = resp_buf
    -- Only this window closes on `q` -- the request buffer (and its tab)
    -- stays, since the point of the split is to keep editing/re-sending
    -- the same request, same as the SQL results buffer's own `q`.
    vim.keymap.set('n', 'q', function()
      local win = vim.fn.bufwinid(resp_buf)
      if win ~= -1 then vim.api.nvim_win_close(win, true) end
    end, { buffer = resp_buf, desc = 'tradar: close gRPC response' })
  end
  vim.bo[resp_buf].modifiable = true
  vim.api.nvim_buf_set_lines(resp_buf, 0, -1, false, vim.split(text, '\n', { plain = true }))
  vim.bo[resp_buf].modifiable = false
  local win = vim.fn.bufwinid(resp_buf)
  if win == -1 then
    local origin = vim.api.nvim_get_current_win()
    vim.cmd('botright 15split')
    win = vim.api.nvim_get_current_win()
    vim.api.nvim_win_set_buf(win, resp_buf)
    vim.api.nvim_set_current_win(origin)
  end
end

--- Parses the request buffer's own text as JSON and invokes -- no
--- "sinh khung" (skeleton) from the method's input shape yet (see
--- backlog's "Chưa làm"), so the buffer opens on a plain `{}` the user
--- fills in by hand, same way grpcurl's own `-d` argument has to be typed
--- out in full by the caller.
function M.send(buf)
  buf = buf or vim.api.nvim_get_current_buf()
  local info = vim.b[buf].tradar_grpc
  if not info then return notify('not a tradar gRPC request buffer', vim.log.levels.ERROR) end
  if vim.b[buf].tradar_grpc_query_id then
    return notify('a request is already running on this buffer -- <leader>rx to cancel it first', vim.log.levels.WARN)
  end
  local text = table.concat(vim.api.nvim_buf_get_lines(buf, 0, -1, false), '\n')
  local ok, request = pcall(vim.json.decode, text, { luanil = { object = true, array = true } })
  if not ok then return notify('request is not valid JSON: ' .. tostring(request), vim.log.levels.ERROR) end

  state.next_id = state.next_id + 1
  local query_id = ('grpc-%d'):format(state.next_id)
  vim.b[buf].tradar_grpc_query_id = query_id
  local function clear() if vim.api.nvim_buf_is_valid(buf) then vim.b[buf].tradar_grpc_query_id = nil end end

  rpc.ensure(require('tradar').rpc_opts(), function(err)
    if err then
      clear()
      return notify(err, vim.log.levels.ERROR)
    end
    rpc.request('grpc.invoke', {
      connection = info.connection,
      service = info.service,
      method = info.method,
      request = request,
      query_id = query_id,
    }, function(e, result, data)
      clear()
      if e then
        local suffix = data and data.grpc_code and (' (grpc_code=' .. tostring(data.grpc_code) .. ')') or ''
        return notify(e .. suffix, vim.log.levels.ERROR)
      end
      -- Indented (and nested keys sorted) rather than one compact line: a real
      -- response is nested, and unreadable as a single line.
      show_response(buf, require('tradar.render').pretty(result.response))
    end)
  end)
end

--- Cancels the in-flight `grpc.invoke` started from this buffer, if any --
--- best-effort the same way `M.cancel`/`:TradarCancel` is for SQL: a gRPC
--- call has no server-side "stop processing" analogue to a DB's own
--- cancel, so this only drops the client's own wait (see `tradar-server`'s
--- `cancel` method, shared across both).
function M.cancel(buf)
  buf = buf or vim.api.nvim_get_current_buf()
  local query_id = vim.b[buf].tradar_grpc_query_id
  if not query_id then return notify('nothing running on this buffer', vim.log.levels.WARN) end
  call('cancel', { query_id = query_id }, function(result)
    if result.cancelled then vim.b[buf].tradar_grpc_query_id = nil end
    notify(result.cancelled and 'cancelled' or 'already finished')
  end)
end

function M.open_request(connection, service, method)
  local key = table.concat({ connection, service, method }, '/')
  local existing = open_buffers[key]
  if existing and vim.api.nvim_buf_is_valid(existing) then
    local win = vim.fn.bufwinid(existing)
    if win ~= -1 then
      vim.api.nvim_set_current_win(win)
    else
      vim.cmd.tabnew()
      vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), existing)
    end
    return notify(('%s/%s -- already open'):format(service, method))
  end

  local buf = scratch('tradar-grpc://' .. key)
  open_buffers[key] = buf
  vim.bo[buf].filetype = 'json'
  vim.b[buf].tradar_grpc = { connection = connection, service = service, method = method }
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, { '{}' })
  vim.api.nvim_buf_create_user_command(buf, 'TradarGrpcSend', function() M.send(buf) end, {})
  vim.api.nvim_buf_create_user_command(buf, 'TradarGrpcCancel', function() M.cancel(buf) end, {})
  vim.keymap.set('n', '<leader>rr', function() M.send(buf) end, { buffer = buf, desc = 'tradar: send gRPC request' })
  vim.keymap.set('n', '<leader>rx', function() M.cancel(buf) end, { buffer = buf, desc = 'tradar: cancel gRPC request' })
  -- No `q` here: unlike the response split/results buffer (both
  -- `modifiable = false`, where `q` shadowing vim's own macro-record is
  -- free), this buffer is a real editable JSON buffer -- stealing `q`
  -- would cost a genuinely useful binding for a UX nicety. `:q`/`:tabclose`
  -- already work fine (the buffer always lives in its own tab).
  vim.keymap.set('n', 'g?', function()
    vim.lsp.util.open_floating_preview({
      '# tradar: gRPC request buffer',
      '- `<leader>rr` / `:TradarGrpcSend` send this request',
      '- `<leader>rx` / `:TradarGrpcCancel` cancel if still running',
      '- `:q`/`:tabclose` close (this buffer is editable -- `q` is left as vim\'s own macro-record)',
    }, 'markdown', { border = 'rounded', focus_id = 'tradar_help', max_width = 90 })
  end, { buffer = buf, desc = 'tradar: show keymap help' })
  vim.cmd.tabnew()
  vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), buf)
  notify(('%s/%s -- edit the JSON body, then <leader>rr or :TradarGrpcSend'):format(service, method))
end

return M
