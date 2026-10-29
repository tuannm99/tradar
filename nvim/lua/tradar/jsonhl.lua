-- Colours for the JSON view of a result, set as extmarks by a small scanner
-- instead of Vim's `syntax json`. The regex engine is time-limited
-- ('redrawtime'): on a big or deeply nested result it gives up part-way and
-- colours come and go with the data -- so the view was left uncoloured past
-- 3000 lines. A scan per line costs the same however big the result is, and
-- knows `ObjectId("...")` / `ISODate("...")` (see `render.pretty`) instead of
-- flagging them as errors.
local M = {}

local ns = vim.api.nvim_create_namespace('tradar_json')

--- Default links; `highlight TradarJsonKey ...` in a colorscheme overrides.
function M.define()
  for name, link in pairs({
    TradarJsonKey = 'Label',
    TradarJsonString = 'String',
    TradarJsonNumber = 'Number',
    TradarJsonConst = 'Boolean',
    TradarJsonFunc = 'Function',
  }) do
    vim.api.nvim_set_hl(0, name, { link = link, default = true })
  end
end

--- Index of the quote closing the string that opens at `start`, or nil.
local function string_end(line, start)
  local i = start + 1
  while true do
    local q = line:find('["\\]', i)
    if not q then return nil end
    if line:sub(q, q) == '"' then return q end
    i = q + 2 -- skip the escaped character
  end
end

--- `{ { col_start, col_end, group }, ... }` (0-based, end exclusive) for one line.
function M.tokens(line)
  local out = {}
  local pos = 1
  local len = #line
  while pos <= len do
    local c = line:sub(pos, pos)
    if c == '"' then
      local stop = string_end(line, pos)
      if not stop then
        out[#out + 1] = { pos - 1, len, 'TradarJsonString' }
        break
      end
      local after = line:match('^%s*()', stop + 1)
      local is_key = line:sub(after, after) == ':'
      out[#out + 1] = { pos - 1, stop, is_key and 'TradarJsonKey' or 'TradarJsonString' }
      pos = stop + 1
    elseif c:match('[%-%d]') then
      local s, e = line:find('^%-?%d[%d%.eE%+%-]*', pos)
      if s then
        out[#out + 1] = { s - 1, e, 'TradarJsonNumber' }
        pos = e + 1
      else
        pos = pos + 1
      end
    elseif c:match('%a') then
      local word = line:match('^[%w_]+', pos)
      local group
      if word == 'true' or word == 'false' or word == 'null' then
        group = 'TradarJsonConst'
      elseif (word == 'ObjectId' or word == 'ISODate') and line:sub(pos + #word, pos + #word) == '(' then
        group = 'TradarJsonFunc'
      end
      if group then out[#out + 1] = { pos - 1, pos - 1 + #word, group } end
      pos = pos + #word
    else
      pos = pos + 1
    end
  end
  return out
end

local MAX_LINES = 100000

--- Colours `buf`'s lines `1..last` (all of `lines`, or nothing when `lines` is
--- nil -- the buffer is not JSON now), from line `from` (default 1). Replaces
--- the previous colours.
function M.apply(buf, lines, last, from)
  vim.api.nvim_buf_clear_namespace(buf, ns, 0, -1)
  if not lines then return end
  M.define()
  for i = from or 1, math.min(last or #lines, #lines, MAX_LINES) do
    for _, t in ipairs(M.tokens(lines[i])) do
      vim.api.nvim_buf_set_extmark(buf, ns, i - 1, t[1], { end_col = t[2], hl_group = t[3] })
    end
  end
end

M.ns = ns

return M
