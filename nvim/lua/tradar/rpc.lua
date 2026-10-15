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

local function default_socket()
  local base = vim.env.XDG_RUNTIME_DIR or vim.fn.fnamemodify(vim.fn.tempname(), ':h')
  return base .. '/tradar/server.sock'
end

--- Where to find the server binary: explicit option, then PATH, then a
--- build sitting next to this plugin (`<repo>/target/{release,debug}`).
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
    vim.schedule(function() cb(msg.error and msg.error.message or nil, msg.result, msg.error and msg.error.data or nil) end)
  end
end

local function on_data(err, chunk)
  if err or not chunk then
    -- Server went away: fail whatever was waiting rather than hang forever.
    local waiting = pending
    if pipe then pipe:close() end
    pending, pipe, buffer = {}, nil, ''
    for _, cb in pairs(waiting) do
      vim.schedule(function() cb('tradar-server closed the connection') end)
    end
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

--- Connects to the socket, starting the server once if nothing answers.
function M.ensure(opts, done)
  if pipe then return done(nil) end
  local path = opts.socket or default_socket()

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
    vim.fn.jobstart({ cmd, path }, { detach = true })
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

--- `cb(err, result, data)`, always on the main loop.
function M.request(method, params, cb)
  if not pipe then return cb('not connected to tradar-server') end
  next_id = next_id + 1
  pending[next_id] = cb
  pipe:write(vim.json.encode({ jsonrpc = '2.0', id = next_id, method = method, params = params }) .. '\n')
end

--- Blocking variant for callers that must answer synchronously. Bounded,
--- and only ever talks to a local socket; returns `err, result`.
function M.request_sync(method, params, timeout_ms)
  local done, err, result = false, nil, nil
  M.request(method, params, function(e, r) done, err, result = true, e, r end)
  if not vim.wait(timeout_ms or 500, function() return done end, 5) then return 'timeout' end
  return err, result
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

function M.connected() return pipe ~= nil end

function M.close()
  if pipe then pipe:close() end
  pipe, buffer, pending = nil, '', {}
end

return M
