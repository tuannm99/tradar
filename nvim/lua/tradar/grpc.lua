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
-- invocation, same spirit as why Kafka/RabbitMQ/HTTP/Socket in the TUI
-- don't reuse `QueryScreenComponent`.
local rpc = require('tradar.rpc')

local M = {}

local state = { current = nil }

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

--- Picks a saved connection with `driver == "grpc"` if `name` is omitted,
--- then connects (idempotent server-side: a second `grpc.connect` on an
--- already-open connection just answers `already: true`, so every public
--- function below calls this first instead of tracking "is it live" here).
local function with_connection(name, cb)
  if name and name ~= '' then
    state.current = name
    return call('grpc.connect', { connection = name }, function() cb(name) end)
  end
  if state.current then
    return call('grpc.connect', { connection = state.current }, function() cb(state.current) end)
  end
  call('connections.list', nil, function(list)
    local usable = vim.tbl_filter(function(c) return c.driver == 'grpc' end, list)
    if #usable == 0 then
      return notify('no saved connection with driver = "grpc" -- add one to connections.toml', vim.log.levels.WARN)
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
  local text = table.concat(vim.api.nvim_buf_get_lines(buf, 0, -1, false), '\n')
  local ok, request = pcall(vim.json.decode, text, { luanil = { object = true, array = true } })
  if not ok then return notify('request is not valid JSON: ' .. tostring(request), vim.log.levels.ERROR) end
  call('grpc.invoke', {
    connection = info.connection,
    service = info.service,
    method = info.method,
    request = request,
  }, function(result) show_response(buf, vim.json.encode(result.response)) end)
end

function M.open_request(connection, service, method)
  local buf = scratch(('tradar-grpc://%s/%s/%s'):format(connection, service, method))
  vim.bo[buf].filetype = 'json'
  vim.b[buf].tradar_grpc = { connection = connection, service = service, method = method }
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, { '{}' })
  vim.api.nvim_buf_create_user_command(buf, 'TradarGrpcSend', function() M.send(buf) end, {})
  vim.keymap.set('n', '<leader>rr', function() M.send(buf) end, { buffer = buf, desc = 'tradar: send gRPC request' })
  vim.cmd.tabnew()
  vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), buf)
  notify(('%s/%s -- edit the JSON body, then <leader>rr or :TradarGrpcSend'):format(service, method))
end

return M
