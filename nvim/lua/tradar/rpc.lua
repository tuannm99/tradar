-- Newline-delimited JSON-RPC client over the tradar-server unix socket.
-- Replies are matched by id: the server answers concurrently, so a slow
-- query never holds up the `cancel` meant for it.
local uv = vim.uv or vim.loop

local M = {}

local pipe, buffer, pending, next_id = nil, '', {}, 0
--- method -> list of callbacks, for a pushed notification (no `id`, see
--- docs/backlog/push-channel-design-2026-10-14.md) rather than a reply to
--- a request this client made -- `kafka.tail.message` today.
local notification_handlers = {}
--- Callbacks run when the connection to the server is lost (not on a
--- deliberate `M.close`) -- a tail buffer uses it to resubscribe.
local disconnect_handlers = {}

--- Kept in sync by hand with `tradar_server::protocol::PROTOCOL_VERSION.0`
--- -- see that constant's own doc comment for the bump convention and why
--- a mismatch only warns, never blocks (docs/backlog/protocol-versioning-2026-10-18.md).
local CLIENT_PROTOCOL_MAJOR = 1
M.CLIENT_PROTOCOL_MAJOR = CLIENT_PROTOCOL_MAJOR
--- Copy of `tradar_server::protocol::METHOD_NOT_FOUND` -- a `version` call
--- answering this code means a pre-versioning server (implicit major 0,
--- since no build ever shipped a `version` method before this existed).
local METHOD_NOT_FOUND = -32601
local warned_version = false

--- Longest unix socket path the OS accepts (`sun_path` is 108 bytes on
--- Linux, 104 on macOS/BSD, one of them the terminating NUL).
function M.max_socket_path()
  return vim.fn.has('linux') == 1 and 107 or 103
end

--- `$XDG_RUNTIME_DIR/tradar/server.sock`, unless that is too long for a
--- unix socket (a deep runtime dir, a long `$TMPDIR`): then
--- `/tmp/tradar-<uid>/server.sock`. The server creates that directory `0700`
--- and refuses to share it, so a short path in a shared `/tmp` is as private
--- as the long one. An explicit `setup{ socket }` is never rewritten.
local function default_socket()
  local base = vim.env.XDG_RUNTIME_DIR or vim.fn.fnamemodify(vim.fn.tempname(), ':h')
  local path = base .. '/tradar/server.sock'
  if #path <= M.max_socket_path() then return path end
  local info = (uv.os_get_passwd and uv.os_get_passwd()) or {}
  return ('/tmp/tradar-%s/server.sock'):format(info.uid or vim.env.USER or 'user')
end

--- Where the plugin talks to the server: `setup{ socket }`, else the default.
function M.socket_path(opts) return opts.socket or default_socket() end

--- `nil` if `path` fits in a unix socket address, else a message saying what
--- to do about it.
function M.check_path(path)
  local limit = M.max_socket_path()
  if #path <= limit then return nil end
  return ('socket path is %d bytes, but a unix socket path can be at most %d: %s -- set setup{ socket = "/tmp/tradar.sock" } (or shorten the directory)')
    :format(#path, limit, path)
end

--- Where to find the server binary: explicit option, then PATH, then a
--- build sitting next to this plugin (`<repo>/target/{release,debug}`).
--- The environment added to the server's own when this plugin starts it, or
--- nil: `setup{ idle_ttl }` as TRADAR_IDLE_TTL and `setup{ server_env = {
--- NO_PROXY = '*', ... } }` as they are. The server also inherits Neovim's
--- environment, so a variable set in the shell Neovim was started from reaches
--- it without this. Only read when the server starts: a server that is already
--- running keeps the environment it had (`:TradarRestart`).
function M.server_env(opts)
  local env
  if opts.idle_ttl ~= nil then
    env = { TRADAR_IDLE_TTL = opts.idle_ttl == false and 'off' or tostring(opts.idle_ttl) }
  end
  for k, v in pairs(opts.server_env or {}) do
    env = env or {}
    env[k] = tostring(v)
  end
  return env
end

function M.server_cmd(opts)
  if opts.server_cmd then return opts.server_cmd end
  if vim.fn.executable('tradar-server') == 1 then return 'tradar-server' end
  local here = debug.getinfo(1, 'S').source:sub(2)
  local repo = vim.fn.fnamemodify(here, ':p:h:h:h:h')
  for _, profile in ipairs({ 'release', 'debug' }) do
    local candidate = repo .. '/target/' .. profile .. '/tradar-server'
    if vim.fn.executable(candidate) == 1 then return candidate end
  end
  return nil
end

local function on_line(line)
  local ok, msg = pcall(vim.json.decode, line, { luanil = { object = true } })
  if not ok or type(msg) ~= 'table' then return end
  -- A notification has no `id` at all (distinct from a response, which
  -- always has one, `null` included on a parse-error reply) -- routed by
  -- `method` to whoever called `M.on`, not matched against `pending`.
  if msg.id == nil and msg.method then
    for _, cb in ipairs(notification_handlers[msg.method] or {}) do
      vim.schedule(function() cb(msg.params) end)
    end
    return
  end
  local cb = pending[msg.id]
  pending[msg.id] = nil
  if cb then
    vim.schedule(function()
      cb(
        msg.error and msg.error.message or nil,
        msg.result,
        msg.error and msg.error.data or nil,
        msg.error and msg.error.code or nil
      )
    end)
  end
end

local function on_data(err, chunk)
  if err or not chunk then
    -- Server went away: fail whatever was waiting rather than hang forever.
    local waiting = pending
    if pipe then pipe:close() end
    pending, pipe, buffer, warned_version = {}, nil, '', false
    for _, cb in pairs(waiting) do
      vim.schedule(function() cb('tradar-server closed the connection') end)
    end
    for _, cb in ipairs(vim.list_slice(disconnect_handlers)) do vim.schedule(cb) end
    return
  end
  buffer = buffer .. chunk
  while true do
    local nl = buffer:find('\n', 1, true)
    if not nl then break end
    local line = buffer:sub(1, nl - 1)
    buffer = buffer:sub(nl + 1)
    if line ~= '' then on_line(line) end
  end
end

--- Fired once per freshly-opened connection, never on every request --
--- advisory only (see `CLIENT_PROTOCOL_MAJOR`'s doc comment): warns and
--- moves on, doesn't hold up or fail the connection it's checking.
local function check_version()
  if warned_version then return end
  M.request('version', nil, function(err, result, _data, code)
    if warned_version then return end
    local server_major = nil
    if err then
      if code == METHOD_NOT_FOUND then server_major = 0 end
    elseif result.major ~= CLIENT_PROTOCOL_MAJOR then
      server_major = result.major
    elseif result.stale then
      -- The binary on disk was rebuilt after this server started: it keeps
      -- running the old code until restarted ("I fixed it, still broken").
      warned_version = true
      vim.notify(
        ('tradar: the running tradar-server (%s) is older than the binary on disk -- :TradarRestart to use the new build'):format(result.exe or '?'),
        vim.log.levels.WARN
      )
    end
    if server_major ~= nil then
      warned_version = true
      vim.notify(
        ('tradar: tradar-server speaks protocol v%d, this plugin expects v%d -- rebuild tradar-server or update the plugin so both match'):format(server_major, CLIENT_PROTOCOL_MAJOR),
        vim.log.levels.WARN
      )
    end
  end)
end

--- Connects to the socket, starting the server once if nothing answers.
function M.ensure(opts, done)
  if pipe then return done(nil) end
  local path = M.socket_path(opts)
  local too_long = M.check_path(path)
  if too_long then return vim.schedule(function() done(too_long) end) end

  local function attempt(cb)
    local p = uv.new_pipe(false)
    -- libuv callbacks run in a "fast" context where most vim.fn calls
    -- (jobstart, ...) are illegal; hop to the main loop before deciding.
    p:connect(path, vim.schedule_wrap(function(err)
      if err then
        p:close()
        return cb(err)
      end
      pipe = p
      p:read_start(on_data)
      check_version()
      cb(nil)
    end))
  end

  attempt(function(err)
    if not err then return done(nil) end
    local cmd = M.server_cmd(opts)
    if opts.autostart == false or not cmd then
      return vim.schedule(function()
        done('cannot reach tradar-server at ' .. path .. (cmd and '' or ' (no tradar-server binary found; see :checkhealth tradar)'))
      end)
    end
    -- `setup{ idle_ttl = ... }` (seconds, "10m", "2h", false/0 = never) reaches the
    -- server as TRADAR_IDLE_TTL; it only takes effect when the server starts.
    vim.fn.jobstart({ cmd, path }, { detach = true, env = M.server_env(opts) })
    -- Poll until the new server is listening (up to ~3s).
    local tries = 0
    local function retry()
      tries = tries + 1
      attempt(function(e)
        if not e then return done(nil) end
        if tries >= 30 then return done('tradar-server did not come up at ' .. path) end
        vim.defer_fn(retry, 100)
      end)
    end
    vim.defer_fn(retry, 100)
  end)
end

local function send(method, params, cb)
  if not pipe then return cb('not connected to tradar-server') end
  next_id = next_id + 1
  pending[next_id] = cb
  pipe:write(vim.json.encode({ jsonrpc = '2.0', id = next_id, method = method, params = params }) .. '\n')
end

--- `cb(err, result, data)`, always on the main loop.
---
--- A `<driver>.*` call (Kafka, RabbitMQ, HTTP, gRPC) whose connection the
--- server no longer has -- closed as idle (`TRADAR_IDLE_TTL`), or the server
--- restarted -- reconnects it with `<driver>.connect` and is sent once more,
--- so a request buffer left open for an hour still just sends.
function M.request(method, params, cb)
  local driver = method:match('^([%w_]+)%.')
  local conn = type(params) == 'table' and params.connection or nil
  if not driver or not conn or method == driver .. '.connect' then return send(method, params, cb) end
  send(method, params, function(err, result, data, code)
    if err and err:find('call `' .. driver .. '.connect` first', 1, true) then
      return send(driver .. '.connect', { connection = conn }, function(cerr)
        if cerr then return cb(err, result, data, code) end
        send(method, params, cb)
      end)
    end
    cb(err, result, data, code)
  end)
end

--- Blocking variant for callers that must answer synchronously. Bounded,
--- and only ever talks to a local socket; returns `err, result`.
function M.request_sync(method, params, timeout_ms)
  local done, err, result, code = false, nil, nil, nil
  M.request(method, params, function(e, r, _d, c) done, err, result, code = true, e, r, c end)
  if not vim.wait(timeout_ms or 500, function() return done end, 5) then return 'timeout' end
  return err, result, code
end

--- Subscribes `cb(params)` to every pushed notification for `method`
--- (`kafka.tail.message`, ...) -- several callers can register for the
--- same method (e.g. two tail buffers), each gets every notification, so a
--- caller filters by its own `params.subscription` itself. Returns an
--- unsubscribe function.
function M.on(method, cb)
  notification_handlers[method] = notification_handlers[method] or {}
  table.insert(notification_handlers[method], cb)
  return function()
    local handlers = notification_handlers[method]
    for i, h in ipairs(handlers or {}) do
      if h == cb then
        table.remove(handlers, i)
        break
      end
    end
  end
end

--- Registers `cb` for a lost connection; returns an unsubscribe function.
function M.on_disconnect(cb)
  table.insert(disconnect_handlers, cb)
  return function()
    for i, h in ipairs(disconnect_handlers) do
      if h == cb then
        table.remove(disconnect_handlers, i)
        break
      end
    end
  end
end

function M.connected() return pipe ~= nil end

function M.close()
  if pipe then pipe:close() end
  pipe, buffer, pending, warned_version = nil, '', {}, false
end

return M
