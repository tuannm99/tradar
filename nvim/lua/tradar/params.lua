-- Query parameters: `${name}` (or `${name=default}`) in a statement is
-- asked for when it runs, and the value is remembered for next time.
--
--   select * from orders where id = ${id} and status = '${status=open}'
--
-- This is text substitution, not a bound parameter: a number goes in as
-- typed, and a value inside quotes has that quote escaped (so `O'Brien`
-- is safe in `'${name}'`) -- but `${id}` in `where id = ${id}` is whatever
-- you type, exactly as if you had typed it in the statement.
local ctx = require('tradar.ctx')
local M = {}

local PATTERN = '%${([%a_][%w_]*)(=?)([^}]*)}'

--- Lines that are only a comment are left alone: `-- where id = ${id}`
--- is commented-out code, not something to ask about.
local function is_comment(line)
  local t = vim.trim(line)
  return t:sub(1, 2) == '--' or t:sub(1, 2) == '//' or t:sub(1, 1) == '#'
end

--- The distinct parameter names in `texts`, in order of appearance, each
--- with the default written in the first placeholder that gave one.
function M.names(texts)
  local out, seen = {}, {}
  for _, text in ipairs(texts) do
    for _, line in ipairs(vim.split(text, '\n', { plain = true })) do
      if not is_comment(line) then
        for name, eq, default in line:gmatch(PATTERN) do
          if not seen[name] then
            seen[name] = #out + 1
            out[#out + 1] = { name = name, default = eq == '=' and default or nil }
          elseif eq == '=' and out[seen[name]].default == nil then
            out[seen[name]].default = default
          end
        end
      end
    end
  end
  return out
end

local NON_SQL = { mongo = true, elasticsearch = true, redis = true }

--- Escapes `value` for sitting between `quote` characters.
local function escape(value, quote, driver)
  if NON_SQL[driver or ''] then
    return (value:gsub('\\', '\\\\'):gsub(quote, '\\' .. quote))
  end
  return (value:gsub(quote, quote .. quote)) -- SQL: double the quote
end

--- `text` with every placeholder replaced from `values` (name -> string).
function M.apply(text, values, driver)
  local lines = vim.split(text, '\n', { plain = true })
  for i, line in ipairs(lines) do
    if not is_comment(line) then
      local out, pos = {}, 1
      while true do
        local s, e, name = line:find(PATTERN, pos)
        if not s then break end
        local value = values[name] or ''
        local before, after = line:sub(s - 1, s - 1), line:sub(e + 1, e + 1)
        if before == after and (before == "'" or before == '"') and s > 1 then
          value = escape(value, before, driver)
        end
        out[#out + 1] = line:sub(pos, s - 1)
        out[#out + 1] = value
        pos = e + 1
      end
      out[#out + 1] = line:sub(pos)
      lines[i] = table.concat(out)
    end
  end
  return table.concat(lines, '\n')
end

--- Asks for each parameter used by `texts` (once, even across several
--- statements), then calls `cb(expanded_texts)`. Nothing to ask: `cb(texts)`
--- at once. Cancelling any question abandons the run (`cb` is not called).
function M.expand(conn, texts, cb)
  local names = M.names(texts)
  if #names == 0 then return cb(texts) end
  ctx.state.params = ctx.state.params or {}
  local remembered, values = ctx.state.params, {}
  local function ask(i)
    local p = names[i]
    if not p then
      local driver = ctx.M.cached_driver(conn)
      local out = {}
      for k, text in ipairs(texts) do out[k] = M.apply(text, values, driver) end
      return cb(out)
    end
    vim.ui.input({
      prompt = ('%s (%d/%d): '):format(p.name, i, #names),
      default = remembered[p.name] or p.default or '',
    }, function(value)
      if value == nil then return end
      values[p.name] = value
      remembered[p.name] = value
      ask(i + 1)
    end)
  end
  ask(1)
end

return M
