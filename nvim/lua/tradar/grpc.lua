-- gRPC: connect, browse service/method via server reflection (or a loaded
-- `.proto` when the target doesn't support reflection), invoke -- unary
-- through `grpc.invoke`, server-/client-streaming and bidi through
-- `grpc.stream.*` over the push channel. Request metadata is the leading
-- `# name: value` lines of the request buffer. Talks to the `grpc.*`
-- methods added to tradar-server
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
      format_item = function(m)
        local tag = (m.client_streaming and m.server_streaming and '  [bidi stream]')
          or (m.client_streaming and '  [client stream]')
          or (m.server_streaming and '  [server stream]')
          or ''
        return ('%s(%s) -> %s%s'):format(m.name, m.input, m.output, tag)
      end,
    }, function(m) if m then M.open_request(conn, service, m.name, m) end end)
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
  if vim.bo[resp_buf].filetype ~= 'json' then vim.bo[resp_buf].filetype = 'json' end
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

--- Splits a request buffer into `metadata` and the JSON body: leading
--- `# name: value` lines are request headers (gRPC metadata), other leading
--- `#`/blank lines are comments; the body starts at the first other line.
--- Returns `nil` for `metadata` when there is none (an empty Lua table would
--- encode as `[]`, not `{}`). The one special name is `deadline` (`5s`,
--- `500ms`, `2m`, or a bare number of seconds): it is not sent as a header
--- but becomes the call's `timeout_ms`.
local function parse_deadline(text)
  local n, unit = text:match('^(%d+%.?%d*)%s*(%a*)$')
  if not n then return nil end
  local factor = ({ ms = 1, s = 1000, [''] = 1000, m = 60000 })[unit:lower()]
  return factor and math.floor(tonumber(n) * factor) or nil
end

local function parse_request(buf)
  local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  local metadata, first, timeout_ms = {}, 1, nil
  while first <= #lines do
    local line = lines[first]
    local key, value = line:match('^#%s*([%w%-_%.]+)%s*:%s*(.-)%s*$')
    if key and key:lower() == 'deadline' then
      timeout_ms = parse_deadline(value)
      if not timeout_ms or timeout_ms <= 0 then
        return nil, nil, nil, ('bad deadline "%s" -- use e.g. 5s, 500ms, 2m'):format(value)
      end
    elseif key then
      metadata[key] = value
    elseif not (line:match('^%s*$') or line:match('^#')) then
      break
    end
    first = first + 1
  end
  -- Whole-line `//` comments are the skeleton's hints, not JSON.
  local body_lines = {}
  for i = first, #lines do
    if not lines[i]:match('^%s*//') then table.insert(body_lines, lines[i]) end
  end
  local text = table.concat(body_lines, '\n')
  local ok, body = pcall(vim.json.decode, text, { luanil = { object = true, array = true } })
  if not ok then return nil, nil, nil, 'request is not valid JSON: ' .. tostring(body) end
  return body, next(metadata) and metadata or nil, timeout_ms
end

--- `# name: value` lines for a metadata object (response headers/trailers),
--- sorted so the order is stable.
local function metadata_lines(label, metadata)
  local keys = vim.tbl_keys(metadata or {})
  table.sort(keys)
  local lines = {}
  for _, key in ipairs(keys) do
    table.insert(lines, ('# %s%s: %s'):format(label, key, metadata[key]))
  end
  return lines
end

local function error_suffix(data)
  return data and data.grpc_code and (' (grpc_code=' .. tostring(data.grpc_code) .. ')') or ''
end

--- subscription -> request buffer, for `grpc.stream.message` pushes.
local streams = {}
local stream_listener
--- Pushes that arrive before `grpc.stream.start` has answered (the server may
--- send the first reply ahead of the response to the start request), by
--- subscription id; replayed once the id is known.
local early = {}
local last_start = 0

--- The response split as an append-only transcript (streams): `‹` a reply,
--- `›` a message we sent, `#` headers/trailers/status.
local function stream_append(buf, lines)
  local resp = vim.b[buf].tradar_grpc_response
  if not (resp and vim.api.nvim_buf_is_valid(resp)) then return end
  require('tradar.tail').append(resp, lines)
end

local function transcript_lines(prefix, text)
  local lines = vim.split(text, '\n', { plain = true })
  for i, line in ipairs(lines) do
    lines[i] = (i == 1 and prefix or '  ') .. line
  end
  return lines
end

local function on_stream_message(params)
  local buf = streams[params.subscription]
  if not buf then
    if (vim.uv or vim.loop).now() - last_start < 3000 then
      early[params.subscription] = early[params.subscription] or {}
      if #early[params.subscription] < 1000 then table.insert(early[params.subscription], params) end
    end
    return
  end
  local kind = params.kind
  if kind == 'headers' or kind == 'trailers' then
    local label = kind == 'trailers' and 'trailer ' or ''
    stream_append(buf, metadata_lines(label, params.metadata))
  elseif kind == 'message' then
    stream_append(buf, transcript_lines('‹ ', require('tradar.render').pretty(params.message)))
  elseif kind == 'sent' then
    stream_append(buf, transcript_lines('› ', vim.json.encode(params.message)))
  else
    -- `end` or `error`: the call is over either way.
    if kind == 'error' then
      stream_append(buf, { ('# error: %s (grpc_code=%s)'):format(params.message or '?', tostring(params.code)) })
    else
      stream_append(buf, { '# end of stream' })
    end
    streams[params.subscription] = nil
    if vim.api.nvim_buf_is_valid(buf) then vim.b[buf].tradar_grpc_stream = nil end
    rpc.request('grpc.stream.stop', { subscription = params.subscription }, function() end)
  end
end

--- A fresh, empty response split for a stream's transcript.
local function reset_response(buf)
  show_response(buf, '')
  local resp = vim.b[buf].tradar_grpc_response
  vim.bo[resp].filetype = ''
  vim.bo[resp].modifiable = true
  vim.api.nvim_buf_set_lines(resp, 0, -1, false, {})
  vim.bo[resp].modifiable = false
end

--- Messages (and the final close) for one stream go out strictly one at a
--- time: the server runs requests of a connection concurrently, so two
--- `grpc.stream.send` in flight together could arrive swapped -- wrong for a
--- stream, where order is the point.
local send_queues = {}

local function pump_sends(buf)
  local queue = send_queues[buf]
  if not queue or queue.busy then return end
  local item = table.remove(queue.items, 1)
  if not item then return end
  queue.busy = true
  local method = item.close and 'grpc.stream.close' or 'grpc.stream.send'
  rpc.request(method, { subscription = item.subscription, message = item.body }, function(e)
    queue.busy = false
    if e then
      queue.items = {}
      return notify(e, vim.log.levels.ERROR)
    end
    if item.close then stream_append(buf, { '# send side closed' }) end
    pump_sends(buf)
  end)
end

local function queue_send(buf, item)
  send_queues[buf] = send_queues[buf] or { items = {}, busy = false }
  table.insert(send_queues[buf].items, item)
  pump_sends(buf)
end

--- Sends the buffer's JSON as one more message on the open stream.
local function stream_send(buf, info, body)
  queue_send(buf, { subscription = vim.b[buf].tradar_grpc_stream, body = body })
end

local function stream_start(buf, info, body, metadata, timeout_ms)
  if not stream_listener then stream_listener = rpc.on('grpc.stream.message', on_stream_message) end
  reset_response(buf)
  local sends = info.client_streaming
  vim.b[buf].tradar_grpc_starting = true
  last_start = (vim.uv or vim.loop).now()
  rpc.request('grpc.stream.start', {
    connection = info.connection,
    service = info.service,
    method = info.method,
    request = (not sends) and body or nil,
    metadata = metadata,
    timeout_ms = timeout_ms,
  }, function(e, result)
    if vim.api.nvim_buf_is_valid(buf) then vim.b[buf].tradar_grpc_starting = nil end
    if e then
      early = {}
      return notify(e, vim.log.levels.ERROR)
    end
    streams[result.subscription] = buf
    vim.b[buf].tradar_grpc_stream = result.subscription
    local queued = early[result.subscription] or {}
    early = {}
    for _, params in ipairs(queued) do on_stream_message(params) end
    if sends and streams[result.subscription] then stream_send(buf, info, body) end
  end)
end

--- Parses the request buffer (metadata + JSON) and invokes. Streaming
--- methods go through `grpc.stream.*` instead: `<leader>rr` starts the call
--- (server-streaming) or starts it and sends the buffer as a message
--- (client/bidi; run it again to send another), `<leader>rc` half-closes,
--- `<leader>rx` stops.
function M.send(buf)
  buf = buf or vim.api.nvim_get_current_buf()
  local info = vim.b[buf].tradar_grpc
  if not info then return notify('not a tradar gRPC request buffer', vim.log.levels.ERROR) end
  local body, metadata, timeout_ms, parse_err = parse_request(buf)
  if parse_err then return notify(parse_err, vim.log.levels.ERROR) end

  if info.client_streaming or info.server_streaming then
    if vim.b[buf].tradar_grpc_starting then return notify('the stream is still starting', vim.log.levels.WARN) end
    if vim.b[buf].tradar_grpc_stream then
      if info.client_streaming then return stream_send(buf, info, body) end
      return notify('this stream is still running -- <leader>rx to stop it first', vim.log.levels.WARN)
    end
    return rpc.ensure(require('tradar').rpc_opts(), function(err)
      if err then return notify(err, vim.log.levels.ERROR) end
      stream_start(buf, info, body, metadata, timeout_ms)
    end)
  end

  if vim.b[buf].tradar_grpc_query_id then
    return notify('a request is already running on this buffer -- <leader>rx to cancel it first', vim.log.levels.WARN)
  end
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
      request = body,
      metadata = metadata,
      timeout_ms = timeout_ms,
      query_id = query_id,
    }, function(e, result, data)
      clear()
      if e then return notify(e .. error_suffix(data), vim.log.levels.ERROR) end
      -- Indented (and nested keys sorted) rather than one compact line: a real
      -- response is nested, and unreadable as a single line.
      local text = require('tradar.render').pretty(result.response)
      local headers = metadata_lines('', result.headers)
      if #headers > 0 then text = table.concat(headers, '\n') .. '\n' .. text end
      show_response(buf, text)
    end)
  end)
end

--- Half-closes a client-/bidi-streaming call: no more messages, replies keep
--- arriving until the server ends the stream.
function M.close(buf)
  buf = buf or vim.api.nvim_get_current_buf()
  local subscription = vim.b[buf].tradar_grpc_stream
  if not subscription then return notify('no open stream on this buffer', vim.log.levels.WARN) end
  queue_send(buf, { subscription = subscription, close = true })
end

--- Cancels the in-flight `grpc.invoke` (or stops the open stream) started
--- from this buffer, if any. A unary cancel is best-effort the same way
--- `M.cancel`/`:TradarCancel` is for SQL: a gRPC call has no server-side
--- "stop processing" analogue to a DB's own cancel, so this only drops the
--- client's own wait (see `tradar-server`'s `cancel` method).
function M.cancel(buf)
  buf = buf or vim.api.nvim_get_current_buf()
  local subscription = vim.b[buf].tradar_grpc_stream
  if subscription then
    streams[subscription] = nil
    vim.b[buf].tradar_grpc_stream = nil
    return call('grpc.stream.stop', { subscription = subscription }, function()
      stream_append(buf, { '# stopped' })
    end)
  end
  local query_id = vim.b[buf].tradar_grpc_query_id
  if not query_id then return notify('nothing running on this buffer', vim.log.levels.WARN) end
  call('cancel', { query_id = query_id }, function(result)
    if result.cancelled then vim.b[buf].tradar_grpc_query_id = nil end
    notify(result.cancelled and 'cancelled' or 'already finished')
  end)
end

function M.open_request(connection, service, method, kind)
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
  vim.b[buf].tradar_grpc = {
    connection = connection,
    service = service,
    method = method,
    client_streaming = kind and kind.client_streaming or false,
    server_streaming = kind and kind.server_streaming or false,
  }
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, { '{}' }) -- until `grpc.skeleton` answers
  vim.api.nvim_buf_create_user_command(buf, 'TradarGrpcSend', function() M.send(buf) end, {})
  vim.api.nvim_buf_create_user_command(buf, 'TradarGrpcCancel', function() M.cancel(buf) end, {})
  vim.keymap.set('n', '<leader>rr', function() M.send(buf) end, { buffer = buf, desc = 'tradar: send gRPC request' })
  vim.keymap.set('n', '<leader>rx', function() M.cancel(buf) end, { buffer = buf, desc = 'tradar: cancel gRPC request / stop stream' })
  if kind and kind.client_streaming then
    vim.api.nvim_buf_create_user_command(buf, 'TradarGrpcClose', function() M.close(buf) end, {})
    vim.keymap.set('n', '<leader>rc', function() M.close(buf) end, { buffer = buf, desc = 'tradar: close the send side of the stream' })
  end
  -- A stream outlives its response split but not its request buffer.
  vim.api.nvim_create_autocmd('BufWipeout', {
    buffer = buf,
    once = true,
    callback = function()
      local subscription = vim.b[buf].tradar_grpc_stream
      if subscription then
        streams[subscription] = nil
        rpc.request('grpc.stream.stop', { subscription = subscription }, function() end)
      end
    end,
  })
  -- No `q` here: unlike the response split/results buffer (both
  -- `modifiable = false`, where `q` shadowing vim's own macro-record is
  -- free), this buffer is a real editable JSON buffer -- stealing `q`
  -- would cost a genuinely useful binding for a UX nicety. `:q`/`:tabclose`
  -- already work fine (the buffer always lives in its own tab).
  vim.keymap.set('n', 'g?', function()
    vim.lsp.util.open_floating_preview({
      '# tradar: gRPC request buffer',
      '- `<leader>rr` / `:TradarGrpcSend` send this request (streaming: start the call; client/bidi: run again to send another message)',
      '- `<leader>rc` / `:TradarGrpcClose` client/bidi stream: close the send side',
      '- `<leader>rx` / `:TradarGrpcCancel` cancel if still running / stop the stream',
      '- whole-line `// ...` comments (the skeleton\'s hints) are ignored when sending',
      '- leading `# name: value` lines are request metadata (headers); `# deadline: 5s` sets the call deadline',
      '- `:q`/`:tabclose` close (this buffer is editable -- `q` is left as vim\'s own macro-record)',
    }, 'markdown', { border = 'rounded', focus_id = 'tradar_help', max_width = 90 })
  end, { buffer = buf, desc = 'tradar: show keymap help' })
  vim.cmd.tabnew()
  vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), buf)
  notify(('%s/%s -- edit the JSON body, then <leader>rr or :TradarGrpcSend'):format(service, method))

  -- Replace the bare `{}` with every field of the input message at its
  -- default. Quiet on failure (an older server has no `grpc.skeleton`), and
  -- never over something the user already started typing.
  rpc.request('grpc.skeleton', { connection = connection, service = service, method = method, comments = true }, function(e, result)
    if e or not result or not vim.api.nvim_buf_is_valid(buf) then return end
    local current = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
    if #current == 1 and vim.trim(current[1]) == '{}' then
      vim.api.nvim_buf_set_lines(buf, 0, -1, false, vim.split(result.skeleton, '\n', { plain = true }))
      vim.bo[buf].modified = false
    end
  end)
end

return M
