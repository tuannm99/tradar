-- Socket tail over `tradar-server`'s push channel (`socket.*` methods,
-- `docs/backlog/socket-tail-server-2026-10-15.md`) -- raw TCP, netcat-style.
-- Same shape as `tradar.kafka` (its sibling module, same `rpc.on`
-- mechanism), with one real difference: a Socket "connection" has no
-- separate connect step and is two-way -- `:TradarSocketTail` both opens
-- the tail buffer and lets you type into it to send, where Kafka's tail is
-- read-only.
local rpc = require('tradar.rpc')

local M = {}

local state = { current = nil }
--- subscription id -> tail buffer, so `socket.tail.message` notifications
--- (routed by `rpc.on`, not by buffer) land in the right one.
local tails = {}

local function notify(msg, level) vim.notify('tradar-socket: ' .. msg, level or vim.log.levels.INFO) end

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
--- `with_connection` -- no request buffer to carry a connection of its own
--- here (unlike gRPC/Kafka, a socket connection picker has nothing to pick
--- beyond which saved connection to dial).
local function with_connection(name, cb)
  if name and name ~= '' then
    state.current = name
    return cb(name)
  end
  if state.current then return cb(state.current) end
  call('connections.list', nil, function(list)
    local usable = vim.tbl_filter(function(c) return c.driver == 'socket' end, list)
    if #usable == 0 then
      return notify('no saved connection with driver = "socket" -- add one with :TradarConnections', vim.log.levels.WARN)
    end
    vim.ui.select(usable, {
      prompt = 'tradar Socket connection',
      format_item = function(c) return ('%s  (%s)%s'):format(c.name, c.target, c.connected and '  ●' or '') end,
    }, function(c)
      if not c then return end
      state.current = c.name
      cb(c.name)
    end)
  end)
end

local function append(buf, lines) require('tradar.tail').append(buf, lines, { input = true }) end

--- One `socket.tail.message` as transcript text -- `›`/`‹` for sent/received,
--- same markers `tradar-connector-socket`'s own `format_entry` uses in the TUI.
local function format_message(msg)
  if msg.error then return { '[error] ' .. msg.error } end
  local marker = msg.direction == 'sent' and '›' or '‹'
  return vim.split(marker .. ' ' .. msg.text, '\n', { plain = true })
end

local function stop_tail(subscription)
  tails[subscription] = nil
  call('socket.tail.stop', { subscription = subscription }, function() end)
end

--- Opens a tail buffer for `conn` and starts the subscription immediately
--- (a Socket connection has no separate connect step, see the module doc
--- comment). The buffer stays modifiable -- unlike Kafka's read-only tail,
--- typing a line and pressing `<CR>` sends it (`M.send`).
local function open_tail(conn)
  local buf = vim.api.nvim_create_buf(false, true)
  vim.bo[buf].bufhidden = 'wipe'
  vim.bo[buf].swapfile = false
  pcall(vim.api.nvim_buf_set_name, buf, ('tradar-socket://%s [%d]'):format(conn, buf))
  vim.b[buf].tradar_socket = { connection = conn }

  call('socket.tail.start', { connection = conn }, function(result)
    local subscription = result.subscription
    vim.b[buf].tradar_socket_subscription = subscription
    tails[subscription] = buf
    local unsubscribe = rpc.on('socket.tail.message', function(params)
      local target = tails[params.subscription]
      if target then append(target, format_message(params)) end
    end)
    vim.api.nvim_create_autocmd('BufWipeout', {
      buffer = buf,
      once = true,
      callback = function()
        unsubscribe()
        stop_tail(subscription)
      end,
    })
  end, function(err)
    if vim.api.nvim_buf_is_valid(buf) then vim.api.nvim_buf_delete(buf, { force = true }) end
    notify(err, vim.log.levels.ERROR)
  end)

  --- The last line is the input line (like a REPL prompt): received/sent
  --- lines land above it. `<CR>` on it sends and clears it, in normal and
  --- insert mode; anywhere else it keeps its plain meaning, so the
  --- transcript above can still be navigated and yanked.
  local function submit()
    local line = vim.api.nvim_get_current_line()
    vim.api.nvim_set_current_line('')
    M.send(buf, line)
  end
  vim.keymap.set('n', '<CR>', function()
    if vim.api.nvim_win_get_cursor(0)[1] ~= vim.api.nvim_buf_line_count(buf) then return end
    submit()
  end, { buffer = buf, desc = 'tradar: send the input line' })
  vim.keymap.set('i', '<CR>', function()
    if vim.api.nvim_win_get_cursor(0)[1] ~= vim.api.nvim_buf_line_count(buf) then return '<CR>' end
    vim.schedule(submit)
    return ''
  end, { buffer = buf, expr = true, desc = 'tradar: send the input line' })
  vim.keymap.set('n', 'q', function()
    local win = vim.fn.bufwinid(buf)
    if win ~= -1 then vim.api.nvim_win_close(win, true) end
  end, { buffer = buf, desc = 'tradar: stop tail and close' })
  vim.keymap.set('n', 'g?', function()
    vim.lsp.util.open_floating_preview({
      '# tradar: Socket tail buffer',
      '- the last line is the input: `<CR>` on it sends and clears it',
      ('- keeps the last %d lines'):format(require('tradar.tail').MAX_LINES),
      '- `q` stop this tail and close',
      '- `›` sent, `‹` received',
    }, 'markdown', { border = 'rounded', focus_id = 'tradar_help', max_width = 90 })
  end, { buffer = buf, desc = 'tradar: show keymap help' })

  vim.cmd.tabnew()
  vim.api.nvim_win_set_buf(vim.api.nvim_get_current_win(), buf)
  vim.cmd.startinsert({ bang = true })
  notify(('connected to %s'):format(conn))
end

--- Sends `text` (the call site decides what counts as "a line to send" --
--- `<CR>` passes the current line) on `buf`'s own subscription.
function M.send(buf, text)
  local subscription = vim.b[buf].tradar_socket_subscription
  if not subscription then return notify('not connected yet', vim.log.levels.WARN) end
  if text == '' then return end
  call('socket.send', { subscription = subscription, text = text }, function() end)
end

--- `:TradarSocketTail [connection]` -- opens a two-way tail buffer, prompting
--- for the connection if omitted.
function M.tail(name) with_connection(name, open_tail) end

return M
