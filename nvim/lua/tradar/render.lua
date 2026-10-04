-- Turns a result page into buffer lines. Display only; paging lives in init.
local M = {}

local function cell(value)
  return (tostring(value):gsub('[\r\n\t]', ' '))
end

--- Aligned text table, one line per row. Widths come from the rows shown
--- (a page), not the whole result, so a header never jumps while paging.
function M.table(columns, rows)
  local widths = {}
  for i, name in ipairs(columns) do widths[i] = vim.fn.strdisplaywidth(name) end
  for _, row in ipairs(rows) do
    for i = 1, #columns do
      widths[i] = math.max(widths[i], vim.fn.strdisplaywidth(cell(row[i] or '')))
    end
  end
  local function line(values)
    local parts = {}
    for i = 1, #columns do
      local text = cell(values[i] or '')
      parts[i] = text .. string.rep(' ', widths[i] - vim.fn.strdisplaywidth(text))
    end
    return table.concat(parts, ' │ ')
  end
  local sep = {}
  for i = 1, #columns do sep[i] = string.rep('─', widths[i]) end
  local out = { line(columns), table.concat(sep, '─┼─') }
  for _, row in ipairs(rows) do out[#out + 1] = line(row) end
  return out
end

--- One JSON document per line, so `/` search and `yy` work per document.
function M.documents(items)
  local out = {}
  for _, item in ipairs(items) do out[#out + 1] = vim.json.encode(item) end
  return out
end

--- Flat navigator: [schema/kind] table, then its columns indented.
function M.schema(entries)
  local lines, targets = {}, {}
  for _, entry in ipairs(entries) do
    local prefix = entry.schema and (entry.schema .. '.') or ''
    lines[#lines + 1] = prefix .. entry.name .. (entry.object_kind and ('  [' .. entry.object_kind .. ']') or '')
    targets[#lines] = prefix .. entry.name
    for _, col in ipairs(entry.columns or {}) do
      local marks = (col.primary_key and ' pk' or '') .. (col.indexed and ' idx' or '')
      local fk = col.foreign_key and (' → ' .. col.foreign_key.table .. '.' .. col.foreign_key.column) or ''
      lines[#lines + 1] = '  ' .. col.name .. '  ' .. col.type .. marks .. fk
      targets[#lines] = col.name
    end
  end
  return lines, targets
end

return M
