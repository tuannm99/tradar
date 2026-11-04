local ctx = require('tradar.ctx')
local M = ctx.M
local rpc = ctx.rpc
local state = ctx.state
local resolve_buf = ctx.resolve_buf

-- ── completion ───────────────────────────────────────────────────────────

--- The text to send `complete`/`split` for "everything before the cursor
--- at (1-based) `row`, (0-based) `col`" -- the whole buffer up to there,
--- except for `.tdb`, where it is only the enclosing block's own lines up
--- to there: a block's dialect can differ from the one before/after it,
--- so including that other text would feed the wrong grammar into the
--- server's completion/split logic. `nil` for a `.tdb` row outside any
--- block -- nothing sensible to complete there.
function M.text_before_cursor(buf, row, col)
  buf = resolve_buf(buf)
  local from = 0
  if M.is_tdb(buf) then
    local block = M.block_at(buf, row)
    if not block then return nil end
    from = block.start - 1
  end
  local lines = vim.api.nvim_buf_get_lines(buf, from, row, false)
  lines[#lines] = lines[#lines]:sub(1, col)
  return table.concat(lines, '\n')
end

--- A short warning when completion cannot work because of the connection
--- (it does not open, or the server errors): without it a database that is
--- down looks like "nothing to suggest". Once per connection -- it is typed over
--- on every keystroke -- and again only after the connection has worked.
function M.completion_failed(conn, err)
  state.completion_warned = state.completion_warned or {}
  if state.completion_warned[conn] then return end
  state.completion_warned[conn] = true
  vim.schedule(function()
    ctx.notify(('completion: `%s` is not available -- %s'):format(conn, tostring(err):gsub('%s+', ' ')), vim.log.levels.WARN)
  end)
end

--- The words of `driver`'s query language, from the server (cached): all that
--- can be suggested with no schema -- the database is down, not connected yet,
--- or no connection is named at all. `cb(items)` with `{text, kind='keyword'}`
--- starting with `partial` (nothing for an empty one), case-insensitively.
function M.static_keywords(driver, partial, cb)
  state.keywords = state.keywords or {}
  local function answer(words)
    local out, p = {}, (partial or ''):lower()
    for _, w in ipairs(words) do
      if p ~= '' and w:lower():sub(1, #p) == p and w:lower() ~= p then out[#out + 1] = { text = w, kind = 'keyword' } end
    end
    cb(out)
  end
  if not driver then return cb({}) end
  if state.keywords[driver] then return answer(state.keywords[driver]) end
  rpc.request('keywords', { driver = driver }, function(err, result)
    if err or type(result) ~= 'table' then return cb({}) end
    state.keywords[driver] = result
    answer(result)
  end)
end

--- The fallback driver for a block with no usable connection: its fence's
--- language, else the failing connection's own.
local function fallback_driver(buf, row, conn)
  local block = M.block_at(buf, row)
  return (conn and M.cached_driver(conn)) or (block and M.lang_driver(block.lang)) or nil
end

local function word_before(text) return text:match('[%w_$]*$') or '' end

--- `cb(items)` with `{text, kind}` for the text before the cursor. Used by
--- the blink.cmp source; async, so typing never waits on it. `row`
--- (1-based, defaults to the cursor's) picks which `.tdb` block's own
--- connection to use -- see `M.block_connection`; irrelevant otherwise.
function M.complete(buf, text, cb, row)
  row = row or vim.api.nvim_win_get_cursor(0)[1]
  local conn = M.block_connection(buf, row)
  if not conn then return M.static_keywords(fallback_driver(buf, row, nil), word_before(text), cb) end
  local function go()
    rpc.request('complete', { connection = conn, text = text }, function(err, result)
      if err then
        M.completion_failed(conn, err)
      elseif state.completion_warned then
        state.completion_warned[conn] = nil -- it works again: a later failure is news
      end
      if err then return M.static_keywords(fallback_driver(buf, row, conn), word_before(text), cb) end
      cb(result and result.items or {})
    end)
  end
  if state.connected[conn] then return go() end
  -- A `.tdb` block can name a connection nothing else has opened yet: open it,
  -- then answer. One that will not open completes nothing -- and says why, once.
  ctx.ensure_connected(conn, go, function(err)
    M.completion_failed(conn, err)
    -- No schema, but the language's own words still help.
    M.static_keywords(fallback_driver(buf, row, conn), word_before(text), cb)
  end)
end

--- Built-in `omnifunc` fallback for setups without blink.cmp.
function M.omnifunc(findstart, base)
  if require('tradar.restbench').applies(0) then return require('tradar.restbench').omni(findstart, base) end
  local row, col = unpack(vim.api.nvim_win_get_cursor(0))
  if findstart == 1 then
    local line = vim.api.nvim_get_current_line():sub(1, col)
    return col - #line:match('[%w_$]*$')
  end
  local conn = M.block_connection(0, row)
  local function words()
    local text = M.text_before_cursor(0, row, col)
    local out = {}
    local driver = fallback_driver(0, row, conn)
    local cached = driver and state.keywords and state.keywords[driver]
    if not cached and driver then
      local err, r = rpc.request_sync('keywords', { driver = driver })
      cached = not err and r or nil
      if cached then
        state.keywords = state.keywords or {}
        state.keywords[driver] = cached
      end
    end
    local p = word_before(text or ''):lower()
    for _, w in ipairs(cached or {}) do
      if p == '' or (w:lower():sub(1, #p) == p and w:lower() ~= p) then out[#out + 1] = { word = w, menu = '[keyword]' } end
    end
    return out
  end
  if not conn then return words() end
  if not state.connected[conn] then
    -- Not open yet (a `.tdb` block's own connection): open it for the next try.
    ctx.ensure_connected(conn, function() end, function(err) M.completion_failed(conn, err) end)
    return words()
  end
  local text = M.text_before_cursor(0, row, col)
  if not text then return {} end
  local err, result = rpc.request_sync('complete', { connection = conn, text = text })
  if err then
    M.completion_failed(conn, err)
  elseif state.completion_warned then
    state.completion_warned[conn] = nil
  end
  if err or not result then return words() end
  local out = {}
  for _, item in ipairs(result.items) do out[#out + 1] = { word = item.text, menu = '[' .. item.kind .. ']' } end
  return out
end
