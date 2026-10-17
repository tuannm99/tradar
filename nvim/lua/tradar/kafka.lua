-- Kafka tail over `tradar-server`'s push channel (`kafka.*` methods,
-- `docs/backlog/push-channel-design-2026-10-14.md` / `kafka-tail-server-2026-10-15.md`):
-- connect, list topics, tail one from latest or earliest into a scrolling
-- transcript buffer -- same "netcat-style, one scrolling panel" shape as the
-- TUI's own Kafka Topics mode and this plugin's Socket tail (not written
-- yet; Kafka was picked to go first, see the backlog entry). Not a
-- `QueryDriver`/SQL buffer: a tail has no statement to run, so this module
-- (like `tradar.grpc`) owns its own small buffer helpers rather than
-- reusing `init.lua`'s results buffer.
local rpc = require('tradar.rpc')

local M = {}

local state = { current = nil }
--- subscription id -> tail state, so `kafka.tail.message` notifications
--- (routed by `rpc.on`, not by buffer) land in the right one.
local tails = {}  -- (value: the tail state, see `new_tail`)

local function notify(msg, level) vim.notify('tradar-kafka: ' .. msg, level or vim.log.levels.INFO) end

local function call(method, params, cb)
  rpc.ensure(require('tradar').rpc_opts(), function(err)
    if err then return notify(err, vim.log.levels.ERROR) end
    rpc.request(method, params, function(e, result)
      if e then return notify(e, vim.log.levels.ERROR) end
      cb(result)
    end)
  end)
end

--- Same "request buffer's own connection, else last picked, else prompt"
--- precedence as `tradar.grpc`'s `implicit_connection`/`with_connection` --
--- a tail buffer remembers which connection it tails in `vim.b`.
local function with_connection(name, cb)
  if name and name ~= '' then
    state.current = name
    return call('kafka.connect', { connection = name }, function() cb(name) end)
  end
  if state.current then
    local implicit = state.current
    return call('kafka.connect', { connection = implicit }, function() cb(implicit) end)
  end
  call('connections.list', nil, function(list)
    local usable = vim.tbl_filter(function(c) return c.driver == 'kafka' end, list)
    if #usable == 0 then
      return notify('no saved connection with driver = "kafka" -- add one with :TradarConnections', vim.log.levels.WARN)
    end
    vim.ui.select(usable, {
      prompt = 'tradar Kafka connection',
      format_item = function(c) return ('%s  (%s)%s'):format(c.name, c.target, c.connected and '  ●' or '') end,
    }, function(c)
      if not c then return end
      state.current = c.name
      call('kafka.connect', { connection = c.name }, function() cb(c.name) end)
    end)
  end)
end

function M.connect(name) with_connection(name, function(conn) notify('connected to ' .. conn) end) end

local function pick_topic(conn, cb)
  call('kafka.topics', { connection = conn }, function(topics)
    if #topics == 0 then return notify(conn .. ' has no topics', vim.log.levels.WARN) end
    vim.ui.select(topics, { prompt = conn }, function(t) if t then cb(t) end end)
  end)
end

local function pick_from(cb)
  vim.ui.select({ 'latest', 'earliest' }, { prompt = 'tail from' }, function(choice) if choice then cb(choice) end end)
end

local append = require('tradar.tail').append

--- One `kafka.tail.message` line as transcript text -- `key: value`, an
--- empty key omitted, same shape the TUI's own Kafka Topics mode renders.
local function format_message(msg)
  if msg.error then return { '[error] ' .. msg.error } end
  local prefix = ('[p%d@%d]'):format(msg.partition, msg.offset)
  local body = (msg.key and msg.key ~= vim.NIL and msg.key ~= '') and (msg.key .. ': ' .. msg.value) or msg.value
  return vim.split(prefix .. ' ' .. body, '\n', { plain = true })
end

--- State of one tail buffer. `entries` is every message received (capped),
--- so the filter can be changed and the view rebuilt; `held` collects
--- messages that arrive while paused.
local function new_tail(conn, topic, buf)
  return { conn = conn, topic = topic, buf = buf, entries = {}, held = {}, filter = '', paused = false, status = nil }
end

local function matches(tail, entry)
  return tail.filter == '' or entry.text:lower():find(tail.filter:lower(), 1, true) ~= nil
end

local function update_bar(tail)
  local win = vim.fn.bufwinid(tail.buf)
  if win == -1 then return end
  local parts = { ('tradar kafka %s/%s'):format(tail.conn, tail.topic) }
  if tail.paused then parts[#parts + 1] = ('PAUSED (%d held)'):format(#tail.held) end
  if tail.filter ~= '' then parts[#parts + 1] = ('filter: %s'):format(tail.filter) end
  if tail.status then parts[#parts + 1] = tail.status end
  vim.wo[win].winbar = ' ' .. table.concat(parts, '  │  '):gsub('%%', '%%%%')
end

--- Rebuilds the buffer from `entries` that pass the filter.
local function rerender(tail)
  local lines = {}
  for _, entry in ipairs(tail.entries) do
    if matches(tail, entry) then vim.list_extend(lines, entry.lines) end
  end
  if not vim.api.nvim_buf_is_valid(tail.buf) then return end
  vim.bo[tail.buf].modifiable = true
  vim.api.nvim_buf_set_lines(tail.buf, 0, -1, false, #lines > 0 and lines or { '' })
  vim.bo[tail.buf].modifiable = false
  local win = vim.fn.bufwinid(tail.buf)
  if win ~= -1 then pcall(vim.api.nvim_win_set_cursor, win, { vim.api.nvim_buf_line_count(tail.buf), 0 }) end
  update_bar(tail)
end

--- A message (or note) for `tail`: remembered, and shown unless paused or
--- filtered out.
local function push(tail, lines)
  local entry = { lines = lines, text = table.concat(lines, '\n') }
  table.insert(tail.entries, entry)
  if #tail.entries > require('tradar.tail').MAX_LINES then table.remove(tail.entries, 1) end
  if tail.paused then
    table.insert(tail.held, entry)
    return update_bar(tail)
  end
  if matches(tail, entry) then append(tail.buf, lines) end
end

local function set_paused(tail, paused)
  tail.paused = paused
  if not paused then
    for _, entry in ipairs(tail.held) do
      if matches(tail, entry) then append(tail.buf, entry.lines) end
    end
    tail.held = {}
  end
  update_bar(tail)
end

local function stop_tail(tail)
  if not tail.sub then return end
  tails[tail.sub] = nil
  call('kafka.tail.stop', { subscription = tail.sub }, function() end)
  tail.sub = nil
end

local function attach_subscription(tail, subscription)
  tail.sub = subscription
  tail.status = nil
  tails[subscription] = tail
  vim.b[tail.buf].tradar_kafka_subscription = subscription
  update_bar(tail)
end

--- The server went away (restart, crash): resubscribe from `latest` --
--- messages produced while disconnected are not replayed, and the
--- transcript says so. One automatic try; `R` retries by hand.
local function reconnect(tail, quiet)
  if tail.sub or tail.reconnecting or not vim.api.nvim_buf_is_valid(tail.buf) then return end
  tail.reconnecting = true
  local function fail(err)
    tail.reconnecting = false
    tail.status = 'DISCONNECTED — R to retry'
    update_bar(tail)
    if not quiet then notify(err, vim.log.levels.WARN) end
  end
  rpc.ensure(require('tradar').rpc_opts(), function(err)
    if err then return fail(err) end
    rpc.request('kafka.connect', { connection = tail.conn }, function(e)
      if e then return fail(e) end
      rpc.request('kafka.tail.start', { connection = tail.conn, topic = tail.topic, from = 'latest' }, function(e2, result)
        if e2 then return fail(e2) end
        tail.reconnecting = false
        if not vim.api.nvim_buf_is_valid(tail.buf) then
          return rpc.request('kafka.tail.stop', { subscription = result.subscription }, function() end)
        end
        attach_subscription(tail, result.subscription)
        push(tail, { '[reconnected — messages while disconnected are not shown]' })
      end)
    end)
  end)
end

--- Opens a tail buffer for `topic` on `conn` from `from` ("latest"/"earliest")
--- and starts the subscription -- `kafka.tail.message` notifications for it
--- land here via `rpc.on`, filtered by `params.subscription`.
local function open_tail(conn, topic, from)
  local buf = vim.api.nvim_create_buf(false, true)
  vim.bo[buf].bufhidden = 'wipe'
  vim.bo[buf].swapfile = false
  vim.bo[buf].modifiable = false
  pcall(vim.api.nvim_buf_set_name, buf, ('tradar-kafka://%s/%s [%d]'):format(conn, topic, buf))
  vim.b[buf].tradar_kafka = { connection = conn, topic = topic }
  local tail = new_tail(conn, topic, buf)

  call('kafka.tail.start', { connection = conn, topic = topic, from = from }, function(result)
    attach_subscription(tail, result.subscription)
    local unsubscribe = rpc.on('kafka.tail.message', function(params)
      if tails[params.subscription] == tail then push(tail, format_message(params)) end
    end)
    local unwatch = rpc.on_disconnect(function()
      if tail.sub then tails[tail.sub] = nil end
      tail.sub = nil
      tail.status = 'DISCONNECTED — reconnecting…'
      update_bar(tail)
      vim.defer_fn(function() reconnect(tail, true) end, 1500)
    end)
    vim.api.nvim_create_autocmd('BufWipeout', {
      buffer = buf,
      once = true,
      callback = function()
        unsubscribe()
        unwatch()
        stop_tail(tail)
      end,
    })
  end)

  local function map(lhs, fn, desc) vim.keymap.set('n', lhs, fn, { buffer = buf, desc = 'tradar: ' .. desc }) end
  map('q', function()
    local win = vim.fn.bufwinid(buf)
    if win ~= -1 then vim.api.nvim_win_close(win, true) end
  end, 'stop tail and close')
  map('p', function() set_paused(tail, not tail.paused) end, 'pause / resume the tail')
  map('c', function()
    tail.entries, tail.held = {}, {}
    rerender(tail)
  end, 'clear the transcript')
  map('f', function()
    vim.ui.input({ prompt = 'Filter (substring, blank = none): ', default = tail.filter }, function(value)
      if value == nil then return end
      tail.filter = value
      rerender(tail)
    end)
  end, 'filter messages')
  map('R', function() reconnect(tail) end, 'resubscribe after a lost connection')
  map('g?', function()
    vim.lsp.util.open_floating_preview({
      '# tradar: Kafka tail buffer',
      '- `q` stop this tail and close',
      '- `p` pause / resume (messages keep arriving and are shown on resume)',
      '- `f` filter by substring (case-insensitive); `c` clear',
      '- `R` resubscribe from `latest` if the server connection was lost (one automatic try)',
      '- read-only; follows the newest message while the cursor is on the last line',
      ('- keeps the last %d messages'):format(require('tradar.tail').MAX_LINES),
    }, 'markdown', { border = 'rounded', focus_id = 'tradar_help', max_width = 90 })
  end, 'show keymap help')

  vim.cmd.tabnew()
  vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), buf)
  update_bar(tail)
  notify(('tailing %s/%s from %s'):format(conn, topic, from))
end

--- `:TradarKafkaTail [connection] [topic]` -- prompts for whatever is
--- omitted (connection, topic, and always the from-offset, same as the TUI's
--- own `Enter`-vs-`b` choice for starting a tail).
function M.tail(name, topic)
  with_connection(name, function(conn)
    local function go(t) pick_from(function(from) open_tail(conn, t, from) end) end
    if topic and topic ~= '' then return go(topic) end
    pick_topic(conn, go)
  end)
end

return M
