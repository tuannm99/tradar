-- Filtering and sorting the rows of a result *as loaded* -- no query runs.
-- Pure functions over `columns` (names) and `rows` (arrays of cell strings).
local M = {}

local OPS = { '>=', '<=', '!=', '=', '~', '>', '<' }

local function cell(row, i)
  local v = row[i]
  if v == nil or v == vim.NIL then return 'NULL' end
  return tostring(v)
end

local function unquote(v)
  local q = v:match('^(["\'])') -- a matching pair of quotes
  if q and v:sub(-1) == q and #v >= 2 then return v:sub(2, -2) end
  return v
end

local function column_index(columns, name)
  for i, c in ipairs(columns) do
    if c:lower() == name:lower() then return i end
  end
end

local function compare(a, b, op)
  if a == 'NULL' and op ~= '=' and op ~= '!=' then return false end
  local na, nb = tonumber(a), tonumber(b)
  if na and nb then a, b = na, nb else a, b = a:lower(), b:lower() end
  if op == '=' then return a == b end
  if op == '!=' then return a ~= b end
  if op == '>' then return a > b end
  if op == '<' then return a < b end
  if op == '>=' then return a >= b end
  return a <= b
end

--- `text` as a predicate over a row, or nil for an empty filter.
---   `status = failed`, `age >= 30`, `name != bob`, `city ~ han` (a column,
---   an operator, a value), or plain `text`: any cell containing it.
--- Case-insensitive; numbers compare as numbers.
function M.parse(text, columns)
  text = vim.trim(text or '')
  if text == '' then return nil end
  for _, op in ipairs(OPS) do
    local name, value = text:match('^([%w_.]+)%s*' .. vim.pesc(op) .. '%s*(.*)$')
    local idx = name and column_index(columns, name)
    if idx then
      value = unquote(value)
      if op == '~' then
        local needle = value:lower()
        return function(row) return cell(row, idx):lower():find(needle, 1, true) ~= nil end
      end
      return function(row) return compare(cell(row, idx), value, op) end
    end
  end
  local needle = text:lower()
  return function(row)
    for i = 1, #columns do
      if cell(row, i):lower():find(needle, 1, true) then return true end
    end
    return false
  end
end

--- The rows that pass `view.filter`, ordered by `view.sort = { col, desc }`
--- (stable; NULLs last; numbers before text compare as numbers).
function M.apply(columns, rows, view)
  local pred = M.parse(view.filter, columns)
  local out = {}
  for _, row in ipairs(rows) do
    if not pred or pred(row) then out[#out + 1] = row end
  end
  local sort = view.sort
  if sort and sort.col then
    local idx = {}
    for i, row in ipairs(out) do idx[row] = i end
    table.sort(out, function(x, y)
      local a, b = cell(x, sort.col), cell(y, sort.col)
      local an, bn = a == 'NULL', b == 'NULL'
      if an ~= bn then return bn end -- NULLs last either way
      if an then return idx[x] < idx[y] end
      local na, nb = tonumber(a), tonumber(b)
      local lt
      if na and nb then
        if na == nb then return idx[x] < idx[y] end
        lt = na < nb
      else
        a, b = a:lower(), b:lower()
        if a == b then return idx[x] < idx[y] end
        lt = a < b
      end
      if sort.desc then return not lt end
      return lt
    end)
  end
  return out
end

return M
