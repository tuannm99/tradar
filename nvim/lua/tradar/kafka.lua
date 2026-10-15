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
--- subscription id -> transcript buffer, so `kafka.tail.message` notifications
--- (routed by `rpc.on`, not by buffer) land in the right one.
local tails = {}

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
      return notify('no saved connection with driver = "kafka" -- add one to connections.toml', vim.log.levels.WARN)
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

local function append(buf, lines)
  if not vim.api.nvim_buf_is_valid(buf) then return end
  vim.bo[buf].modifiable = true
  local total = vim.api.nvim_buf_line_count(buf)
  -- The buffer opens with one empty placeholder line; the first real
  -- message replaces it instead of leaving a blank line above the tail.
  local at = (total == 1 and vim.api.nvim_buf_get_lines(buf, 0, 1, false)[1] == '') and 0 or total
  vim.api.nvim_buf_set_lines(buf, at, total, false, lines)
  vim.bo[buf].modifiable = false
  local win = vim.fn.bufwinid(buf)
  if win ~= -1 then vim.api.nvim_win_set_cursor(win, { vim.api.nvim_buf_line_count(buf), 0 }) end
end

--- One `kafka.tail.message` line as transcript text -- `key: value`, an
--- empty key omitted, same shape the TUI's own Kafka Topics mode renders.
local function format_message(msg)
  if msg.error then return { '[error] ' .. msg.error } end
  local prefix = ('[p%d@%d]'):format(msg.partition, msg.offset)
  local body = (msg.key and msg.key ~= vim.NIL and msg.key ~= '') and (msg.key .. ': ' .. msg.value) or msg.value
  return vim.split(prefix .. ' ' .. body, '\n', { plain = true })
end

local function stop_tail(buf, subscription)
  tails[subscription] = nil
  call('kafka.tail.stop', { subscription = subscription }, function() end)
  vim.b[buf].tradar_kafka_subscription = nil
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

  call('kafka.tail.start', { connection = conn, topic = topic, from = from }, function(result)
    local subscription = result.subscription
    vim.b[buf].tradar_kafka_subscription = subscription
    tails[subscription] = buf
    local unsubscribe = rpc.on('kafka.tail.message', function(params)
      local target = tails[params.subscription]
      if target then append(target, format_message(params)) end
    end)
    vim.api.nvim_create_autocmd('BufWipeout', {
      buffer = buf,
      once = true,
      callback = function()
        unsubscribe()
        stop_tail(buf, subscription)
      end,
    })
  end)

  vim.keymap.set('n', 'q', function()
    local win = vim.fn.bufwinid(buf)
    if win ~= -1 then vim.api.nvim_win_close(win, true) end
  end, { buffer = buf, desc = 'tradar: stop tail and close' })
  vim.keymap.set('n', 'g?', function()
    vim.lsp.util.open_floating_preview({
      '# tradar: Kafka tail buffer',
      '- `q` stop this tail and close',
      '- read-only transcript, newest message at the bottom',
    }, 'markdown', { border = 'rounded', focus_id = 'tradar_help', max_width = 90 })
  end, { buffer = buf, desc = 'tradar: show keymap help' })

  vim.cmd.tabnew()
  vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), buf)
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
