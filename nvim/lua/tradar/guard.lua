-- Decides whether a statement deserves a "are you sure?" before it runs.
-- Pure text analysis (no server, no buffers) so it is trivially testable.
-- A heuristic on tokens, not a parser: it only has to catch the usual slips
-- (a forgotten WHERE, a DROP on the wrong connection), and a miss just means
-- no prompt -- the same trade the drivers' own `returns_rows` makes.
local M = {}

local WRITES = {
  insert = true, update = true, delete = true, merge = true, replace = true,
  create = true, alter = true, drop = true, truncate = true, rename = true,
  grant = true, revoke = true,
}

--- `text` with comments and string/identifier literals blanked out, so a
--- `WHERE` inside a string or a comment can't fool the checks below.
local function scrub(text)
  local out, i, n = {}, 1, #text
  while i <= n do
    local two = text:sub(i, i + 1)
    local c = text:sub(i, i)
    if two == '--' then
      local e = text:find('\n', i, true) or n + 1
      out[#out + 1] = ' '
      i = e
    elseif two == '/*' then
      local _, e = text:find('*/', i + 2, true)
      out[#out + 1] = ' '
      i = (e or n) + 1
    elseif c == "'" or c == '"' or c == '`' then
      local j = i + 1
      while j <= n do
        if text:sub(j, j) == c then
          if text:sub(j + 1, j + 1) == c then j = j + 2 else break end -- doubled quote = escaped
        else
          j = j + 1
        end
      end
      out[#out + 1] = ' '
      i = j + 1
    else
      out[#out + 1] = c
      i = i + 1
    end
  end
  return table.concat(out):lower()
end

local function words(clean)
  local list = {}
  for w in clean:gmatch('[%a_][%w_]*') do list[#list + 1] = w end
  return list
end

--- Whether `name` matches one of the `protected` patterns (plain,
--- case-insensitive substring: "prod" protects "prod-main" and "myprod").
function M.is_protected(name, protected)
  if not name then return false end
  local lower = name:lower()
  for _, p in ipairs(protected or {}) do
    if lower:find(p:lower(), 1, true) then return true end
  end
  return false
end

--- Whether `text` changes the schema (so cached table/column info is stale).
function M.changes_schema(text)
  local first = words(scrub(text))[1]
  return first == 'create' or first == 'alter' or first == 'drop' or first == 'rename'
end

--- First line of `text` that is real code, not blank or a `--` comment --
--- what a prompt should quote, rather than the modeline above the statement.
function M.first_code_line(text)
  for line in text:gmatch('[^\n]+') do
    local t = vim.trim(line)
    if t ~= '' and t:sub(1, 2) ~= '--' then return t end
  end
  return vim.trim(text:match('[^\n]*') or '')
end

--- A reason string when `text` should be confirmed, else nil.
--- `protected`: the connection is a protected one -- any write asks.
function M.assess(text, protected)
  local ws = words(scrub(text))
  local first = ws[1]
  if not first then return nil end

  local set = {}
  for _, w in ipairs(ws) do set[w] = true end

  -- A CTE can hide a write behind `WITH`: look for a DML verb anywhere.
  local verb = first
  if first == 'with' then
    for _, w in ipairs(ws) do
      if w == 'update' or w == 'delete' or w == 'insert' then verb = w break end
    end
  end

  if (verb == 'update' or verb == 'delete') and not set['where'] then
    return ('%s without WHERE changes every row'):format(verb:upper())
  end
  if verb == 'drop' or verb == 'truncate' then
    return ('%s is destructive and cannot be undone'):format(verb:upper())
  end
  if protected and WRITES[verb] then
    return ('%s on a protected connection'):format(verb:upper())
  end
  return nil
end

return M
