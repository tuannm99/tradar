-- Re-indents a JSON text without parsing it into Lua values, so the order of
-- the keys and the exact digits of every number survive (an HTTP response is
-- shown as the server wrote it; `render.pretty` sorts keys and goes through
-- doubles).
local M = {}

--- Index of the quote closing the string that opens at `start`.
local function string_end(text, start)
  local i = start + 1
  while true do
    local q = text:find('["\\]', i)
    if not q then return nil end
    if text:sub(q, q) == '"' then return q end
    i = q + 2
  end
end

--- `text` as indented JSON, or nil when it is not valid JSON.
function M.pretty(text)
  if type(text) ~= 'string' or not text:find('%S') then return nil end
  if not pcall(vim.json.decode, text) then return nil end
  local out, n = {}, 0
  local depth, i, len = 0, 1, #text
  local function put(s)
    n = n + 1
    out[n] = s
  end
  local function newline() put('\n' .. ('  '):rep(depth)) end
  while i <= len do
    local c = text:sub(i, i)
    if c == '"' then
      local stop = string_end(text, i)
      put(text:sub(i, stop))
      i = stop + 1
    elseif c == '{' or c == '[' then
      local close = c == '{' and '}' or ']'
      local nxt = text:match('^%s*()', i + 1)
      if text:sub(nxt, nxt) == close then
        put(c .. close)
        i = nxt + 1
      else
        put(c)
        depth = depth + 1
        newline()
        i = i + 1
      end
    elseif c == '}' or c == ']' then
      depth = depth - 1
      newline()
      put(c)
      i = i + 1
    elseif c == ',' then
      put(',')
      newline()
      i = i + 1
    elseif c == ':' then
      put(': ')
      i = i + 1
    elseif c:match('%s') then
      i = i + 1
    else
      local s, e = text:find('^[^%s,:%]%}%[%{"]+', i)
      put(text:sub(s, e))
      i = e + 1
    end
  end
  return table.concat(out)
end

return M
