-- Newline-delimited JSON-RPC client over the tradar-server unix socket.
local uv = vim.uv or vim.loop

local M = {}

local pipe, buffer, pending, next_id = nil, '', {}, 0

local function default_socket()
  local base = vim.env.XDG_RUNTIME_DIR or vim.fn.fnamemodify(vim.fn.tempname(), ':h')
  return base .. '/tradar/server.sock'
end

local function on_line(line)
  local ok, msg = pcall(vim.json.decode, line, { luanil = { object = true } })
  if not ok or type(msg) ~= 'table' then return end
  local cb = pending[msg.id]
  pending[msg.id] = nil
  if cb then vim.schedule(function() cb(msg.error and msg.error.message or nil, msg.result) end) end
end

local function on_data(err, chunk)
  if err or not chunk then
    -- Server went away: fail whatever was waiting rather than hang forever.
    local waiting = pending
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

--- Connects to the socket, starting `tradar-server` once if nothing answers.
function M.ensure(opts, done)
  if pipe then return done(nil) end
  local path = opts.socket or default_socket()

  local function attempt(cb)
    local p = uv.new_pipe(false)
    p:connect(path, function(err)
      if err then
        p:close()
        return cb(err)
      end
      pipe = p
      p:read_start(on_data)
      cb(nil)
    end)
  end

  attempt(function(err)
    if not err then return vim.schedule(function() done(nil) end) end
    if opts.autostart == false or vim.fn.executable(opts.server_cmd or 'tradar-server') == 0 then
      return vim.schedule(function() done('cannot reach tradar-server at ' .. path) end)
    end
    vim.fn.jobstart({ opts.server_cmd or 'tradar-server', path }, { detach = true })
    local tries = 0
    local timer = uv.new_timer()
    timer:start(100, 100, function()
      tries = tries + 1
      attempt(function(e)
        if not e then
          timer:close()
          vim.schedule(function() done(nil) end)
        elseif tries >= 30 then
          timer:close()
          vim.schedule(function() done('tradar-server did not come up at ' .. path) end)
        end
      end)
    end)
  end)
end

--- `cb(err, result)`, always on the main loop.
function M.request(method, params, cb)
  if not pipe then return cb('not connected to tradar-server') end
  next_id = next_id + 1
  pending[next_id] = cb
  pipe:write(vim.json.encode({ jsonrpc = '2.0', id = next_id, method = method, params = params }) .. '\n')
end

--- Blocking variant for callers that must answer synchronously (omnifunc).
--- Bounded, and only ever talks to a local socket; returns `err, result`.
function M.request_sync(method, params, timeout_ms)
  local done, err, result = false, nil, nil
  M.request(method, params, function(e, r) done, err, result = true, e, r end)
  if not vim.wait(timeout_ms or 500, function() return done end, 5) then return 'timeout' end
  return err, result
end

function M.close()
  if pipe then pipe:close() end
  pipe, buffer, pending = nil, '', {}
end

return M
