-- The request workbench for HTTP and gRPC connections: one editable buffer
-- per connection holding any number of requests, a `###` line between them
-- (format and parsing: `reqparse.lua`). `<leader>rr` runs the block under
-- the cursor; the response lands in the shared result tabs (`results.lua`,
-- kind `text`) -- one tab per method + path / gRPC method, with the earlier
-- runs of it one `[R` away, `<leader>rD` to diff two of them.
--
-- HTTP goes through `http.send`; gRPC through `grpc.invoke` (unary) or
-- `grpc.stream.*` (streaming, replies pushed to the transcript tab).
local rpc = require('tradar.rpc')
local reqparse = require('tradar.reqparse')
local jsonfmt = require('tradar.jsonfmt')
local ctx = require('tradar.ctx')

local M = {}

local benches = {} -- connection -> its workbench buffer
local last = {} -- driver -> connection last used
local running = {} -- slot key -> { kind = 'http'|'unary'|'stream', ... }
local streams = {} -- subscription -> { entry, key }
local early = {} -- stream pushes that beat the answer to `grpc.stream.start`
local last_start = 0
local stream_listener
local methods_cache = {} -- connection .. service -> grpc.methods answer

local function notify(msg, level) vim.notify('tradar: ' .. msg, level or vim.log.levels.INFO) end

local function prefix() return require('tradar').rpc_opts().prefix or '<leader>r' end

local function call(method, params, cb, on_error)
  rpc.ensure(require('tradar').rpc_opts(), function(err)
    if err then return (on_error or function(e) notify(e, vim.log.levels.ERROR) end)(err) end
    rpc.request(method, params, function(e, result, data)
      if e then
        local suffix = data and data.grpc_code and (' (grpc_code=' .. tostring(data.grpc_code) .. ')') or ''
        return (on_error or function(m) notify(m, vim.log.levels.ERROR) end)(e .. suffix)
      end
      cb(result)
    end)
  end)
end

local function split(text)
  local lines = vim.split(text, '\n', { plain = true })
  for i, l in ipairs(lines) do lines[i] = l:gsub('\r$', '') end
  return lines
end

-- ── the buffer ───────────────────────────────────────────────────────────

local TEMPLATE = {
  http = {
    '# One request per block; a line starting with ### begins the next one.',
    '# <leader>rr sends the block under the cursor, the response opens in the result tabs.',
    '# @name = value defines a variable, used as {{name}} ({{$uuid}}, {{$timestamp}}, {{$env.NAME}} too).',
    '',
    '### example',
    'GET /',
    'Accept: application/json',
  },
  grpc = {
    '# One request per block; a line starting with ### begins the next one.',
    '# :TradarGrpcServices picks a method and adds its block, with a request skeleton, here.',
    '# Then: GRPC service/Method, metadata lines (name: value), deadline: 5s, a blank line, the JSON body.',
    '# @name = value defines a variable, used as {{name}} ({{$uuid}}, {{$timestamp}}, {{$env.NAME}} too).',
  },
}

local function focus(buf)
  local win = vim.fn.bufwinid(buf)
  if win ~= -1 then
    vim.api.nvim_set_current_win(win)
  else
    vim.cmd.tabnew()
    vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), buf)
  end
end

local function help(buf)
  local p = prefix()
  vim.lsp.util.open_floating_preview({
    '# tradar: request workbench (HTTP / gRPC)',
    ('- `%sr` / `:TradarHttpSend` / `:TradarGrpcSend` run the block under the cursor; the response opens in the result tabs'):format(p),
    ('- `%sx` stop waiting (HTTP) / cancel the call / stop the stream; `%sc` close the send side of a client / bidi stream'):format(p, p),
    ('- `%sy` copy the block as a curl / grpcurl command'):format(p),
    '- `###` (optional title) starts a block; the text before the first one is a block too',
    '- HTTP block: `METHOD url` (a bare url is a GET; a relative url is joined to the connection\'s base), `Name: value` headers, a blank line, the body',
    '- gRPC block: `GRPC service/Method`, `name: value` metadata, `deadline: 5s`, a blank line, the JSON body (`// …` lines are ignored)',
    '- `@name = value` defines a variable for the whole file; `{{name}}`, `{{$uuid}}`, `{{$timestamp}}`, `{{$isoTimestamp}}`, `{{$env.NAME}}` expand in the url, headers and body',
    ('- on a response: `[R` / `]R` earlier runs, `%sD` diff against the previous run, `%sH` show the headers, `%sR` send it again'):format(p, p, p),
    '- the buffer is scratch: `:w path` saves the requests to a file',
  }, 'markdown', { border = 'rounded', focus_id = 'tradar_help', max_width = 110 })
end

--- The workbench buffer of `conn` (created on first use), with its keys.
local function bench_buffer(conn, driver, target)
  local existing = benches[conn]
  if existing and vim.api.nvim_buf_is_valid(existing) then return existing, false end
  local buf = vim.api.nvim_create_buf(true, true)
  benches[conn] = buf
  vim.bo[buf].bufhidden = 'hide'
  vim.bo[buf].swapfile = false
  vim.bo[buf].filetype = 'http'
  pcall(vim.api.nvim_buf_set_name, buf, ('tradar-%s://%s [%d]'):format(driver, conn, buf))
  vim.b[buf].tradar_rest = { connection = conn, driver = driver, target = target }
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, TEMPLATE[driver] or TEMPLATE.http)
  local p = prefix()
  local function map(lhs, fn, desc) vim.keymap.set('n', p .. lhs, fn, { buffer = buf, desc = 'tradar: ' .. desc }) end
  map('r', function() M.run(buf) end, 'run the request block under the cursor')
  map('x', function() M.cancel(buf) end, 'cancel the request / stop the stream')
  map('c', function() M.close_send(buf) end, 'close the send side of a client / bidi stream')
  map('y', function() M.copy(buf) end, 'copy the block as a curl / grpcurl command')
  vim.keymap.set('n', 'g?', function() help(buf) end, { buffer = buf, desc = 'tradar: show keymap help' })
  local send = driver == 'grpc' and 'TradarGrpcSend' or 'TradarHttpSend'
  vim.api.nvim_buf_create_user_command(buf, send, function() M.run(buf) end, {})
  vim.api.nvim_buf_create_user_command(buf, 'TradarGrpcCancel', function() M.cancel(buf) end, {})
  vim.api.nvim_buf_create_user_command(buf, 'TradarGrpcClose', function() M.close_send(buf) end, {})
  vim.api.nvim_create_autocmd('BufWipeout', {
    buffer = buf,
    once = true,
    callback = function()
      benches[conn] = nil
      for sub, st in pairs(streams) do
        if st.buf == buf then
          streams[sub] = nil
          rpc.request('grpc.stream.stop', { subscription = sub }, function() end)
        end
      end
    end,
  })
  return buf, true
end

--- A saved connection of one of `drivers` (`name` if given and usable, else
--- the one used last, else a pick): `cb(connection_record)`.
local function pick_connection(name, drivers, cb)
  call('connections.list', nil, function(list)
    local usable = vim.tbl_filter(function(c) return vim.tbl_contains(drivers, c.driver) end, list)
    if name and name ~= '' then
      for _, c in ipairs(list) do
        if c.name == name then
          if not vim.tbl_contains(drivers, c.driver) then
            return notify(('"%s" is a %s connection, not %s'):format(name, c.driver, table.concat(drivers, '/')), vim.log.levels.WARN)
          end
          return cb(c)
        end
      end
      return notify(('no saved connection named "%s"'):format(name), vim.log.levels.WARN)
    end
    local info = vim.b[0] and vim.b[0].tradar_rest
    for _, c in ipairs(usable) do
      if info and c.name == info.connection then return cb(c) end
    end
    for _, driver in ipairs(drivers) do
      for _, c in ipairs(usable) do
        if c.name == last[driver] and c.driver == driver then return cb(c) end
      end
    end
    if #usable == 0 then
      return notify(('no saved connection with driver = "%s" -- add one with :TradarAddConnection'):format(table.concat(drivers, '" or "')), vim.log.levels.WARN)
    end
    if #usable == 1 then return cb(usable[1]) end
    vim.ui.select(usable, {
      prompt = 'tradar connection',
      format_item = function(c) return ('%s  (%s)%s'):format(c.name, c.target, c.connected and '  ●' or '') end,
    }, function(c) if c then cb(c) end end)
  end)
end

--- Connects `c` (idempotent on the server) and hands over its workbench buffer.
local function with_bench(name, drivers, cb)
  pick_connection(name, drivers, function(c)
    last[c.driver] = c.name
    call(c.driver .. '.connect', { connection = c.name }, function()
      local buf, fresh = bench_buffer(c.name, c.driver, c.target)
      cb(buf, fresh, c)
    end)
  end)
end

--- `:TradarHttpSend [conn]` / `:TradarGrpcSend [conn]` / `:TradarRequests [conn]`:
--- inside a workbench it runs the block; elsewhere it opens the workbench.
function M.open(name, driver)
  if (name == nil or name == '') and vim.b.tradar_rest then return M.run(vim.api.nvim_get_current_buf()) end
  with_bench(name, driver and { driver } or { 'http', 'grpc' }, function(buf, fresh, c)
    focus(buf)
    if fresh and c.driver == 'http' then pcall(vim.api.nvim_win_set_cursor, 0, { 6, 0 }) end
  end)
end

--- The `GRPC service/Method` block of a method in the workbench of `conn`
--- (added, with a request skeleton, when it is not there yet), cursor in its body.
function M.insert_grpc(conn, service, method, kind)
  with_bench(conn, { 'grpc' }, function(buf)
    focus(buf)
    local want = vim.pesc(service .. '/' .. method)
    local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
    for i, l in ipairs(lines) do
      if l:match('^%s*[Gg][Rr][Pp][Cc]%s+' .. want .. '%s*$') then
        pcall(vim.api.nvim_win_set_cursor, 0, { i, 0 })
        return notify(('%s/%s is already in the workbench'):format(service, method))
      end
    end
    local tag = (kind and kind.client_streaming and kind.server_streaming and ' [bidi stream]')
      or (kind and kind.client_streaming and ' [client stream]')
      or (kind and kind.server_streaming and ' [server stream]')
      or ''
    rpc.request('grpc.skeleton', { connection = conn, service = service, method = method, comments = true }, function(e, result)
      local body = (not e and result and result.skeleton) or '{}'
      local block = { '', ('### %s%s'):format(method, tag), ('GRPC %s/%s'):format(service, method), '' }
      vim.list_extend(block, vim.split(body, '\n', { plain = true }))
      local at = vim.api.nvim_buf_line_count(buf)
      vim.api.nvim_buf_set_lines(buf, at, at, false, block)
      vim.bo[buf].modified = false
      pcall(vim.api.nvim_win_set_cursor, vim.fn.bufwinid(buf), { at + 5, 0 })
      notify(('%s/%s added -- edit the JSON body, then %sr'):format(service, method, prefix()))
    end)
  end)
end

-- ── running a block ──────────────────────────────────────────────────────

local function cursor_row(buf)
  local win = vim.fn.bufwinid(buf)
  if win ~= -1 then return vim.api.nvim_win_get_cursor(win)[1] end
  return 1
end

--- The request of the block under the cursor, or nil (already told why).
local function request_at(buf)
  local info = vim.b[buf].tradar_rest
  if not info then
    notify('not a tradar request workbench (open one with :TradarHttpSend or :TradarGrpcSend)', vim.log.levels.WARN)
    return
  end
  local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  local block = reqparse.block_at(lines, cursor_row(buf))
  local req, err = reqparse.parse(lines, block)
  if not req then
    notify(err, vim.log.levels.WARN)
    return
  end
  if req.kind ~= info.driver then
    notify(
      req.kind == 'grpc' and ('"%s" is an HTTP connection -- a GRPC block needs a gRPC one'):format(info.connection)
        or ('"%s" is a gRPC connection -- this block is an HTTP request'):format(info.connection),
      vim.log.levels.WARN
    )
    return
  end
  return req, info
end

local function metadata_lines(label, metadata)
  local keys = vim.tbl_keys(metadata or {})
  table.sort(keys)
  local out = {}
  for _, k in ipairs(keys) do out[#out + 1] = ('%s%s: %s'):format(label, k, metadata[k]) end
  return out
end

local function method_info(conn, service, method, cb)
  local ck = conn .. '\n' .. service
  local function pick(list)
    for _, m in ipairs(list or {}) do
      if m.name == method then return m end
    end
  end
  if methods_cache[ck] and pick(methods_cache[ck]) then return cb(pick(methods_cache[ck])) end
  rpc.request('grpc.methods', { connection = conn, service = service }, function(e, list)
    if e then return cb(nil) end
    methods_cache[ck] = list
    cb(pick(list))
  end)
end

function M.forget_methods(conn)
  for k in pairs(methods_cache) do
    if k:sub(1, #conn + 1) == conn .. '\n' then methods_cache[k] = nil end
  end
end

local counter = 0

local function new_entry(conn, key, label, query)
  return { conn = conn, key = key, label = label, query = query, at = os.time() }
end

local function show_http(req, info, key, label, query, resp, refresh_of, rerun)
  local pretty = jsonfmt.pretty(resp.body)
  local entry = new_entry(info.connection, key, label, query)
  entry.lines = split(pretty or resp.body)
  entry.json = pretty ~= nil
  entry.status = ('%d %s'):format(resp.status, resp.status_text)
  entry.badge = tostring(resp.status)
  entry.took_ms = resp.elapsed_ms
  entry.bytes = #resp.body
  entry.head = { ('%s %s'):format(req.method, entry.status) }
  for _, h in ipairs(resp.headers) do entry.head[#entry.head + 1] = ('%s: %s'):format(h[1], h[2]) end
  entry.rerun = rerun
  ctx.show_text(entry, refresh_of)
end

local function finish(key) running[key] = nil end

local function run_http(req, info, key, label, query, refresh_of, rerun)
  counter = counter + 1
  local token = counter
  running[key] = { kind = 'http', token = token }
  local headers = {}
  for _, h in ipairs(req.headers) do headers[#headers + 1] = h[1] .. ': ' .. h[2] end
  local function fail(msg)
    if running[key] and running[key].token == token then finish(key) end
    notify(msg, vim.log.levels.ERROR)
  end
  call('http.connect', { connection = info.connection }, function()
    call('http.send', {
      connection = info.connection,
      method = req.method,
      url = req.url,
      headers = table.concat(headers, '\n'),
      body = req.body,
    }, function(resp)
      local r = running[key]
      if not r or r.token ~= token then return end -- stopped waiting
      finish(key)
      show_http(req, info, key, label, query, resp, refresh_of, rerun)
    end, fail)
  end, fail)
end

-- ── gRPC: unary and streams ──────────────────────────────────────────────

local function transcript(prefix_text, text)
  local lines = vim.split(text, '\n', { plain = true })
  for i, line in ipairs(lines) do lines[i] = (i == 1 and prefix_text or '  ') .. line end
  return lines
end

local function on_stream_message(params)
  local st = streams[params.subscription]
  if not st then
    if (vim.uv or vim.loop).now() - last_start < 3000 then
      early[params.subscription] = early[params.subscription] or {}
      if #early[params.subscription] < 1000 then table.insert(early[params.subscription], params) end
    end
    return
  end
  local kind = params.kind
  local lines
  if kind == 'headers' or kind == 'trailers' then
    lines = metadata_lines(kind == 'trailers' and '# trailer ' or '# ', params.metadata)
  elseif kind == 'message' then
    lines = transcript('‹ ', require('tradar.render').pretty(params.message))
  elseif kind == 'sent' then
    lines = transcript('› ', vim.json.encode(params.message))
  else
    lines = { kind == 'error' and ('# error: %s (grpc_code=%s)'):format(params.message or '?', tostring(params.code)) or '# end of stream' }
    st.entry.stream_open = false
    streams[params.subscription] = nil
    finish(st.key)
    rpc.request('grpc.stream.stop', { subscription = params.subscription }, function() end)
  end
  ctx.text_append(st.entry, lines)
  if kind == 'end' or kind == 'error' then ctx.text_changed(st.entry) end
end

--- Messages (and the final close) of one stream go out strictly one at a
--- time: the server runs a connection's requests concurrently, and two
--- `grpc.stream.send` in flight could arrive swapped.
local send_queues = {}

local function pump(sub)
  local queue = send_queues[sub]
  if not queue or queue.busy then return end
  local item = table.remove(queue.items, 1)
  if not item then return end
  queue.busy = true
  rpc.request(item.close and 'grpc.stream.close' or 'grpc.stream.send', { subscription = sub, message = item.body }, function(e)
    queue.busy = false
    local st = streams[sub]
    if e then
      queue.items = {}
      return notify(e, vim.log.levels.ERROR)
    end
    if item.close and st then ctx.text_append(st.entry, { '# send side closed' }) end
    pump(sub)
  end)
end

local function queue_send(sub, item)
  send_queues[sub] = send_queues[sub] or { items = {}, busy = false }
  table.insert(send_queues[sub].items, item)
  pump(sub)
end

local function start_stream(req, info, key, label, query, kind, refresh_of, rerun)
  if not stream_listener then stream_listener = rpc.on('grpc.stream.message', on_stream_message) end
  local entry = new_entry(info.connection, key, label, query)
  entry.lines, entry.head, entry.json = {}, {}, false
  entry.status, entry.badge = 'stream', '⇄'
  entry.stream, entry.stream_open = true, true
  entry.rerun = rerun
  running[key] = { kind = 'stream', entry = entry, client_streaming = kind.client_streaming, starting = true }
  ctx.show_text(entry, refresh_of)
  last_start = (vim.uv or vim.loop).now()
  local sends = kind.client_streaming
  rpc.request('grpc.stream.start', {
    connection = info.connection,
    service = req.service,
    method = req.method,
    request = (not sends) and req.body or nil,
    metadata = req.metadata,
    timeout_ms = req.timeout_ms,
  }, function(e, result, data)
    if e then
      early = {}
      entry.stream_open = false
      finish(key)
      ctx.text_append(entry, { ('# error: %s%s'):format(e, data and data.grpc_code and (' (grpc_code=' .. tostring(data.grpc_code) .. ')') or '') })
      return ctx.text_changed(entry)
    end
    local r = running[key]
    if r then r.sub, r.starting = result.subscription, nil end
    streams[result.subscription] = { entry = entry, key = key, buf = benches[info.connection] }
    local queued = early[result.subscription] or {}
    early = {}
    for _, params in ipairs(queued) do on_stream_message(params) end
    if sends and streams[result.subscription] then queue_send(result.subscription, { body = req.body }) end
  end)
end

local function run_unary(req, info, key, label, query, refresh_of, rerun)
  counter = counter + 1
  local query_id = ('grpc-%d'):format(counter)
  running[key] = { kind = 'unary', query_id = query_id }
  call('grpc.invoke', {
    connection = info.connection,
    service = req.service,
    method = req.method,
    request = req.body,
    metadata = req.metadata,
    timeout_ms = req.timeout_ms,
    query_id = query_id,
  }, function(result)
    local r = running[key]
    if not r or r.query_id ~= query_id then return end
    finish(key)
    local entry = new_entry(info.connection, key, label, query)
    entry.lines = split(require('tradar.render').pretty(result.response))
    entry.json = true
    entry.status, entry.badge = 'OK', 'OK'
    entry.head = { 'OK' }
    vim.list_extend(entry.head, metadata_lines('', result.headers))
    entry.rerun = rerun
    ctx.show_text(entry, refresh_of)
  end, function(msg)
    local r = running[key]
    if r and r.query_id == query_id then finish(key) end
    notify(msg, vim.log.levels.ERROR)
  end)
end

--- Sends `req`; the answer opens in (or replaces) the tab of its method + path.
--- `refresh_of`: the result tab being repeated by `<leader>rR`.
function M.execute(req, info, refresh_of)
  local key, label, query = reqparse.slot(req, info.connection)
  local active = running[key]
  local function rerun(prev) M.execute(req, info, prev) end

  if req.kind == 'http' then
    if active then return notify(('%s is still waiting -- %sx stops waiting'):format(label, prefix()), vim.log.levels.WARN) end
    return run_http(req, info, key, label, query, refresh_of, rerun)
  end

  if active and active.kind == 'stream' then
    if active.starting then return notify('the stream is still starting', vim.log.levels.WARN) end
    if active.client_streaming and active.sub then
      queue_send(active.sub, { body = req.body })
      return
    end
    return notify(('%s is still streaming -- %sx stops it first'):format(label, prefix()), vim.log.levels.WARN)
  end
  if active then return notify(('%s is still running -- %sx cancels it'):format(label, prefix()), vim.log.levels.WARN) end

  call('grpc.connect', { connection = info.connection }, function()
    method_info(info.connection, req.service, req.method, function(kind)
      if kind and (kind.client_streaming or kind.server_streaming) then
        return start_stream(req, info, key, label, query, kind, refresh_of, rerun)
      end
      run_unary(req, info, key, label, query, refresh_of, rerun)
    end)
  end)
end

--- Runs the block under the cursor of workbench `buf`.
function M.run(buf)
  buf = (buf == nil or buf == 0) and vim.api.nvim_get_current_buf() or buf
  local req, info = request_at(buf)
  if req then M.execute(req, info) end
end

--- The running thing of the block under the cursor (or the only one of the
--- workbench), with its key.
local function running_at(buf)
  local info = vim.b[buf].tradar_rest
  if not info then return nil, notify('not a tradar request workbench', vim.log.levels.WARN) end
  local lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  local req = reqparse.parse(lines, reqparse.block_at(lines, cursor_row(buf)))
  if req then
    local key = reqparse.slot(req, info.connection)
    if running[key] then return running[key], key end
  end
  local only, only_key, n = nil, nil, 0
  for key, r in pairs(running) do
    if key:sub(1, #info.connection + 1) == info.connection .. '\n' then only, only_key, n = r, key, n + 1 end
  end
  if n == 1 then return only, only_key end
  notify(n == 0 and 'nothing is running here' or 'several requests are running -- put the cursor in the block to stop', vim.log.levels.WARN)
end

--- `<leader>rx`: HTTP -- stop waiting for the answer (the server still gets
--- the request); unary gRPC -- cancel the call; a stream -- stop it.
function M.cancel(buf)
  buf = (buf == nil or buf == 0) and vim.api.nvim_get_current_buf() or buf
  local r, key = running_at(buf)
  if not r then return end
  if r.kind == 'stream' then
    if r.sub then
      streams[r.sub] = nil
      rpc.request('grpc.stream.stop', { subscription = r.sub }, function() end)
    end
    r.entry.stream_open = false
    finish(key)
    ctx.text_append(r.entry, { '# stopped' })
    return ctx.text_changed(r.entry)
  end
  finish(key)
  if r.kind == 'unary' then
    return call('cancel', { query_id = r.query_id }, function(result)
      notify(result.cancelled and 'cancelled' or 'already finished')
    end)
  end
  notify('stopped waiting for the response')
end

--- `<leader>rc`: no more messages on a client / bidi stream.
function M.close_send(buf)
  buf = (buf == nil or buf == 0) and vim.api.nvim_get_current_buf() or buf
  local r = running_at(buf)
  if not r then return end
  if r.kind ~= 'stream' or not r.client_streaming or not r.sub then
    return notify('no open client / bidi stream here', vim.log.levels.WARN)
  end
  queue_send(r.sub, { close = true })
end

--- `<leader>ry`: the block under the cursor as a `curl` / `grpcurl` command, yanked.
function M.copy(buf)
  buf = (buf == nil or buf == 0) and vim.api.nvim_get_current_buf() or buf
  local req, info = request_at(buf)
  if not req then return end
  local text = req.kind == 'grpc' and reqparse.as_grpcurl(req, info.target) or reqparse.as_curl(req, info.target)
  vim.fn.setreg('"', text)
  pcall(vim.fn.setreg, '+', text)
  notify(('copied as %s'):format(req.kind == 'grpc' and 'grpcurl' or 'curl'))
end

return M
