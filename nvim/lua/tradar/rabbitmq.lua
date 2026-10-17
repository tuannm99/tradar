-- RabbitMQ over `tradar-server` (`rabbitmq.*` methods,
-- `docs/backlog/rabbitmq-server-2026-10-16.md`): browse queues/exchanges,
-- peek messages non-destructively, publish. No push channel needed --
-- unlike Kafka/Socket, the Management HTTP API has no streaming endpoint,
-- so this is plain request/response, same shape as `tradar.grpc`. Not a
-- sidebar-based screen like the TUI's own `RabbitScreen`: browsing here
-- goes through `vim.ui.select` (pick a queue/exchange, see what's there),
-- same idiom `tradar.grpc`/`tradar.kafka` already use for picking a
-- resource that isn't a SQL-style result set.
local rpc = require('tradar.rpc')

local M = {}

local state = { current = nil }

local function notify(msg, level) vim.notify('tradar-rabbitmq: ' .. msg, level or vim.log.levels.INFO) end

local function call(method, params, cb)
  rpc.ensure(require('tradar').rpc_opts(), function(err)
    if err then return notify(err, vim.log.levels.ERROR) end
    rpc.request(method, params, function(e, result)
      if e then return notify(e, vim.log.levels.ERROR) end
      cb(result)
    end)
  end)
end

--- Same "last picked, else prompt" precedence as `tradar.kafka`'s
--- `with_connection` -- connects first (idempotent server-side).
local function with_connection(name, cb)
  local function go(conn)
    state.current = conn
    call('rabbitmq.connect', { connection = conn }, function() cb(conn) end)
  end
  if name and name ~= '' then return go(name) end
  if state.current then return go(state.current) end
  call('connections.list', nil, function(list)
    local usable = vim.tbl_filter(function(c) return c.driver == 'rabbitmq' end, list)
    if #usable == 0 then
      return notify('no saved connection with driver = "rabbitmq" -- add one with :TradarConnections', vim.log.levels.WARN)
    end
    vim.ui.select(usable, {
      prompt = 'tradar RabbitMQ connection',
      format_item = function(c) return ('%s  (%s)%s'):format(c.name, c.target, c.connected and '  ●' or '') end,
    }, function(c) if c then go(c.name) end end)
  end)
end

function M.connect(name) with_connection(name, function(conn) notify('connected to ' .. conn) end) end

--- One scratch, non-modifiable, reused split for whatever was last
--- browsed (messages, bindings) -- same role as `tradar.grpc`'s own
--- response split, just for this module's own browsing instead of a
--- request/response pair.
local browse_buf = nil
local browse_refresh = nil
local function show_lines(title, lines, refresh)
  browse_refresh = refresh
  if not (browse_buf and vim.api.nvim_buf_is_valid(browse_buf)) then
    browse_buf = vim.api.nvim_create_buf(false, true)
    vim.bo[browse_buf].bufhidden = 'hide'
    vim.bo[browse_buf].swapfile = false
    pcall(vim.api.nvim_buf_set_name, browse_buf, 'tradar://rabbitmq')
    vim.keymap.set('n', 'q', function()
      local win = vim.fn.bufwinid(browse_buf)
      if win ~= -1 then vim.api.nvim_win_close(win, true) end
    end, { buffer = browse_buf, desc = 'tradar: close' })
    vim.keymap.set('n', 'r', function()
      if browse_refresh then browse_refresh() end
    end, { buffer = browse_buf, desc = 'tradar: refresh' })
    vim.keymap.set('n', 'g?', function()
      vim.lsp.util.open_floating_preview(
        { '# tradar: RabbitMQ browse buffer', '- `r` refresh (re-peek / re-list)', '- `q` close' },
        'markdown', { border = 'rounded', focus_id = 'tradar_help', max_width = 90 }
      )
    end, { buffer = browse_buf, desc = 'tradar: show keymap help' })
  end
  vim.bo[browse_buf].modifiable = true
  vim.api.nvim_buf_set_lines(browse_buf, 0, -1, false, lines)
  vim.bo[browse_buf].modifiable = false
  local win = vim.fn.bufwinid(browse_buf)
  if win == -1 then
    local origin = vim.api.nvim_get_current_win()
    vim.cmd('botright 15split')
    win = vim.api.nvim_get_current_win()
    vim.api.nvim_win_set_buf(win, browse_buf)
    vim.api.nvim_set_current_win(origin)
  end
  notify(title)
end

--- `:TradarRabbitmqPeek [connection] [queue]` -- picks a queue to peek if
--- omitted; peeking is a one-shot snapshot (`ackmode=ack_requeue_true`
--- server-side, same as the TUI), not a live tail -- RabbitMQ over this
--- server deliberately has no `*.tail.*` method, see the module doc comment.
function M.peek(name, queue)
  with_connection(name, function(conn)
    local function go(q)
      call('rabbitmq.peek', { connection = conn, queue = q }, function(messages)
        if #messages == 0 then return notify(('%s: no messages right now'):format(q)) end
        local lines = {}
        for _, m in ipairs(messages) do
          local prefix = m.redelivered and '[redelivered] ' or ''
          lines[#lines + 1] = ('%s%s -> %s'):format(prefix, m.exchange ~= '' and m.exchange or '(default)', m.routing_key)
          vim.list_extend(lines, vim.split(m.payload, '\n', { plain = true }))
          lines[#lines + 1] = ''
        end
        show_lines(('%s: %d message(s) peeked from %s'):format(conn, #messages, q), lines, function() go(q) end)
      end)
    end
    if queue and queue ~= '' then return go(queue) end
    call('rabbitmq.queues', { connection = conn }, function(queues)
      if #queues == 0 then return notify(conn .. ' has no queues', vim.log.levels.WARN) end
      vim.ui.select(queues, {
        prompt = conn,
        format_item = function(q)
          return ('%s  ready=%d unacked=%d consumers=%d'):format(q.name, q.messages_ready, q.messages_unacknowledged, q.consumers)
        end,
      }, function(q) if q then go(q.name) end end)
    end)
  end)
end

--- `:TradarRabbitmqBindings [connection] [exchange]` -- picks an exchange
--- to browse if omitted (the nameless default exchange is filtered out
--- server-side, same as the TUI -- nothing to browse bindings for there).
function M.bindings(name, exchange)
  with_connection(name, function(conn)
    local function go(ex)
      call('rabbitmq.bindings', { connection = conn, exchange = ex }, function(bindings)
        if #bindings == 0 then return notify(('%s: no bindings'):format(ex)) end
        local lines = {}
        for _, b in ipairs(bindings) do
          lines[#lines + 1] = ('%s  (routing key: %s)'):format(b.destination, b.routing_key ~= '' and b.routing_key or '(none)')
        end
        show_lines(('%s: %d binding(s) from %s'):format(conn, #bindings, ex), lines, function() go(ex) end)
      end)
    end
    if exchange and exchange ~= '' then return go(exchange) end
    call('rabbitmq.exchanges', { connection = conn }, function(exchanges)
      if #exchanges == 0 then return notify(conn .. ' has no exchanges', vim.log.levels.WARN) end
      vim.ui.select(exchanges, {
        prompt = conn,
        format_item = function(e) return ('%s  (%s)'):format(e.name, e.type) end,
      }, function(e) if e then go(e.name) end end)
    end)
  end)
end

--- Splits a publish buffer into `exchange, routing_key, payload`:
---
---     exchange: orders          (blank / missing = the default exchange)
---     routing-key: orders.new   (for the default exchange: the queue name)
---
---     {"id": 1}
---
--- Lines before the first blank line are `key: value` (`#` lines ignored);
--- everything after it is the payload, as many lines as it needs.
function M.parse_publish(lines)
  local fields, i = {}, 1
  while i <= #lines do
    local line = lines[i]
    i = i + 1
    if line:match('^%s*$') then break end
    if not line:match('^%s*#') then
      local k, v = line:match('^%s*([%w_-]+)%s*:%s*(.-)%s*$')
      if k then fields[k:lower():gsub('_', '-')] = v end
    end
  end
  return fields['exchange'] or '', fields['routing-key'] or '', table.concat(vim.list_slice(lines, i), '\n')
end

local PUBLISH_TEMPLATE = {
  '# <leader>rr (or :TradarRabbitmqPublish) publishes. Payload: everything after the blank line.',
  'exchange:',
  'routing-key:',
  '',
  '',
}

local publish_buffers = {} -- connection name -> its publish buffer

function M.submit(buf)
  local conn = vim.b[buf].tradar_rabbitmq and vim.b[buf].tradar_rabbitmq.connection
  if not conn then return notify('not a tradar RabbitMQ publish buffer', vim.log.levels.WARN) end
  local exchange, routing_key, payload = M.parse_publish(vim.api.nvim_buf_get_lines(buf, 0, -1, false))
  if exchange == '' and routing_key == '' then
    return notify('set `routing-key:` (the queue name for the default exchange)', vim.log.levels.WARN)
  end
  call('rabbitmq.publish', { connection = conn, exchange = exchange, routing_key = routing_key, payload = payload }, function()
    notify(('published to %s%s'):format(exchange ~= '' and exchange or '(default) ', routing_key))
  end)
end

--- Opens (or jumps back to) the editable publish buffer for `conn` -- one
--- per connection, so the last message is there to tweak and publish again.
function M.open_publish(conn)
  local existing = publish_buffers[conn]
  if existing and vim.api.nvim_buf_is_valid(existing) then
    local win = vim.fn.bufwinid(existing)
    if win ~= -1 then
      vim.api.nvim_set_current_win(win)
    else
      vim.cmd.tabnew()
      vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), existing)
    end
    return
  end
  local buf = vim.api.nvim_create_buf(true, true)
  publish_buffers[conn] = buf
  vim.bo[buf].bufhidden = 'hide'
  vim.bo[buf].swapfile = false
  pcall(vim.api.nvim_buf_set_name, buf, ('tradar-rabbitmq://%s/publish [%d]'):format(conn, buf))
  vim.b[buf].tradar_rabbitmq = { connection = conn }
  vim.api.nvim_buf_set_lines(buf, 0, -1, false, PUBLISH_TEMPLATE)
  local p = require('tradar').rpc_opts().prefix or '<leader>r'
  vim.api.nvim_buf_create_user_command(buf, 'TradarRabbitmqPublish', function() M.submit(buf) end, {})
  vim.keymap.set('n', p .. 'r', function() M.submit(buf) end, { buffer = buf, desc = 'tradar: publish message' })
  vim.keymap.set('n', 'g?', function()
    vim.lsp.util.open_floating_preview({
      '# tradar: RabbitMQ publish buffer',
      ('- `%sr` / `:TradarRabbitmqPublish` publish this message'):format(p),
      '- `exchange:` (blank = default) and `routing-key:` (the queue name for the default exchange)',
      '- the payload is everything after the blank line',
    }, 'markdown', { border = 'rounded', focus_id = 'tradar_help', max_width = 90 })
  end, { buffer = buf, desc = 'tradar: show keymap help' })
  vim.cmd.tabnew()
  vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), buf)
  vim.api.nvim_win_set_cursor(0, { 3, 12 })
end

--- `:TradarRabbitmqPublish [connection]`: inside a publish buffer it
--- publishes that buffer; elsewhere it opens the connection's buffer.
function M.publish(name)
  if (name == nil or name == '') and vim.b.tradar_rabbitmq then return M.submit(vim.api.nvim_get_current_buf()) end
  with_connection(name, M.open_publish)
end

return M
