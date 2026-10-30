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

--- `cb(items)` with `{text, kind}` for the text before the cursor. Used by
--- the blink.cmp source; async, so typing never waits on it. `row`
--- (1-based, defaults to the cursor's) picks which `.tdb` block's own
--- connection to use -- see `M.block_connection`; irrelevant otherwise.
function M.complete(buf, text, cb, row)
  row = row or vim.api.nvim_win_get_cursor(0)[1]
  local conn = M.block_connection(buf, row)
  if not conn or not state.connected[conn] then return cb({}) end
  rpc.request('complete', { connection = conn, text = text }, function(err, result)
    cb((not err and result) and result.items or {})
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
  if not conn or not state.connected[conn] then return {} end
  local text = M.text_before_cursor(0, row, col)
  if not text then return {} end
  local err, result = rpc.request_sync('complete', { connection = conn, text = text })
  if err or not result then return {} end
  local out = {}
  for _, item in ipairs(result.items) do out[#out + 1] = { word = item.text, menu = '[' .. item.kind .. ']' } end
  return out
end
