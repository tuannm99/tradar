-- What changed between two results of the same shape: rows added, removed,
-- and cells changed, matched by key. Pure -- `results.lua` shows it.
local M = {}

local function text(v)
  if v == nil or v == vim.NIL then return 'NULL' end
  return tostring(v)
end

local function common_columns(a, b)
  local by_b = {}
  for i, name in ipairs(b) do by_b[name:lower()] = i end
  local common, only_a = {}, {}
  local seen = {}
  for i, name in ipairs(a) do
    local j = by_b[name:lower()]
    if j then
      common[#common + 1] = { name = name, a = i, b = j }
      seen[name:lower()] = true
    else
      only_a[#only_a + 1] = name
    end
  end
  local only_b = {}
  for _, name in ipairs(b) do
    if not seen[name:lower()] then only_b[#only_b + 1] = name end
  end
  return common, only_a, only_b
end

--- `a` = the earlier result, `b` = the later one (each `columns, rows`).
--- `keys` names the columns that identify a row (default: the first common
--- column). Returns { keys, added = { {key, cells} }, removed = {...},
--- changed = { {key, cells = { {col, from, to} } } }, same, only_a, only_b }.
function M.compare(cols_a, rows_a, cols_b, rows_b, keys)
  local common, only_a, only_b = common_columns(cols_a, cols_b)
  if #common == 0 then return nil, 'the two results have no column in common' end
  local key_cols = {}
  for _, k in ipairs(keys or {}) do
    for _, c in ipairs(common) do
      if c.name:lower() == k:lower() then key_cols[#key_cols + 1] = c break end
    end
  end
  if #key_cols == 0 then key_cols = { common[1] } end

  local function key_of(row, side)
    local parts = {}
    for _, c in ipairs(key_cols) do parts[#parts + 1] = text(row[c[side]]) end
    return table.concat(parts, '\1')
  end
  local function key_label(row, side)
    local parts = {}
    for _, c in ipairs(key_cols) do parts[#parts + 1] = c.name .. '=' .. text(row[c[side]]) end
    return table.concat(parts, ', ')
  end
  local function index(rows, side)
    local by, order, dup = {}, {}, {}
    for _, row in ipairs(rows) do
      local k = key_of(row, side)
      dup[k] = (dup[k] or 0) + 1
      if dup[k] > 1 then k = k .. '\2' .. dup[k] end -- same key twice: matched in order
      by[k] = row
      order[#order + 1] = k
    end
    return by, order
  end
  local by_a, order_a = index(rows_a, 'a')
  local by_b, order_b = index(rows_b, 'b')

  local out = { keys = {}, added = {}, removed = {}, changed = {}, same = 0, only_a = only_a, only_b = only_b }
  for _, c in ipairs(key_cols) do out.keys[#out.keys + 1] = c.name end
  local function cells(row, side)
    local o = {}
    for _, c in ipairs(common) do o[#o + 1] = text(row[c[side]]) end
    return table.concat(o, ' | ')
  end
  for _, k in ipairs(order_b) do
    local rb, ra = by_b[k], by_a[k]
    if not ra then
      out.added[#out.added + 1] = { key = key_label(rb, 'b'), cells = cells(rb, 'b') }
    else
      local diffs = {}
      for _, c in ipairs(common) do
        local x, y = text(ra[c.a]), text(rb[c.b])
        if x ~= y then diffs[#diffs + 1] = { col = c.name, from = x, to = y } end
      end
      if #diffs > 0 then
        out.changed[#out.changed + 1] = { key = key_label(rb, 'b'), cells = diffs }
      else
        out.same = out.same + 1
      end
    end
  end
  for _, k in ipairs(order_a) do
    if by_b[k] == nil then
      local row = by_a[k]
      out.removed[#out.removed + 1] = { key = key_label(row, 'a'), cells = cells(row, 'a') }
    end
  end
  return out
end

local function one_line(s)
  s = s:gsub('%s+', ' ')
  if vim.fn.strchars(s) > 80 then s = vim.fn.strcharpart(s, 0, 79) .. '…' end
  return s
end

--- The comparison as the lines of a report.
function M.lines(d, title_a, title_b)
  local lines = {
    ('tradar diff   before: %s   after: %s'):format(title_a, title_b),
    ('matched by %s · %d added · %d removed · %d changed · %d same'):format(
      table.concat(d.keys, ', '), #d.added, #d.removed, #d.changed, d.same),
  }
  if #d.only_a > 0 then lines[#lines + 1] = 'only before: ' .. table.concat(d.only_a, ', ') end
  if #d.only_b > 0 then lines[#lines + 1] = 'only after: ' .. table.concat(d.only_b, ', ') end
  lines[#lines + 1] = ''
  for _, c in ipairs(d.changed) do
    lines[#lines + 1] = '~ ' .. c.key
    for _, x in ipairs(c.cells) do
      lines[#lines + 1] = ('    %s: %s → %s'):format(x.col, one_line(x.from), one_line(x.to))
    end
  end
  for _, r in ipairs(d.added) do lines[#lines + 1] = ('+ %s | %s'):format(r.key, one_line(r.cells)) end
  for _, r in ipairs(d.removed) do lines[#lines + 1] = ('- %s | %s'):format(r.key, one_line(r.cells)) end
  if #d.added + #d.removed + #d.changed == 0 then lines[#lines + 1] = 'no difference' end
  return lines
end

return M
