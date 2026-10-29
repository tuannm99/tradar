-- gRPC: connect, browse services / methods via server reflection (or a loaded
-- `.proto` when the target has no reflection), and add a method's request
-- block, with a skeleton, to the request workbench. Running requests -- unary,
-- server-/client-streaming, bidi, metadata, deadline -- is `restbench.lua`
-- (format: `reqparse.lua`). Talks to the `grpc.*` methods of tradar-server
-- (docs/backlog/grpc-rpc-methods-2026-10-14.md); this module keeps no
-- connection state beyond "which connection did we last pick", the server
-- already holds the channel and the schema.
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

--- The connection a `services()`/`methods()` call with no explicit name
--- should act on: the request buffer under the cursor's own connection
--- first (so re-browsing from inside a request buffer for "staging" never
--- silently acts on "prod" just because that was connected most recently
--- elsewhere), then the last connection picked anywhere, then a prompt.
local function implicit_connection()
  local info = vim.b[0] and vim.b[0].tradar_rest
  return (info and info.driver == 'grpc' and info.connection) or state.current
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
      require('tradar.restbench').forget_methods(conn)
      notify(('%s: added %d service(s): %s'):format(conn, #result.added, table.concat(result.added, ', ')))
    end)
  end)
end

--- Lists a service's methods and lets the user pick one to add a request
--- block for to the workbench -- the one piece reused by both `TradarGrpcServices` (after
--- picking a service) and `TradarGrpcMethods` (service named directly).
local function pick_method(conn, service)
  call('grpc.methods', { connection = conn, service = service }, function(methods)
    if #methods == 0 then return notify(service .. ' has no methods', vim.log.levels.WARN) end
    vim.ui.select(methods, {
      prompt = service,
      format_item = function(m)
        local tag = (m.client_streaming and m.server_streaming and '  [bidi stream]')
          or (m.client_streaming and '  [client stream]')
          or (m.server_streaming and '  [server stream]')
          or ''
        return ('%s(%s) -> %s%s'):format(m.name, m.input, m.output, tag)
      end,
    }, function(m) if m then require('tradar.restbench').insert_grpc(conn, service, m.name, m) end end)
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

return M
