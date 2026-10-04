-- Result pages <-> text. Pure functions: no buffers, no server.
local M = {}

local SEP = ' │ '
local SEP_WIDTH = 3

local function cell(value)
  return (tostring(value):gsub('[\r\n\t]', ' '))
end

--- Aligned text table plus, per column, the display-column span it
--- occupies (`spans[i] = {first, last}`, 1-based, inclusive) -- what lets a
--- cursor position map back to a cell for yanking.
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
    return table.concat(parts, SEP)
  end
  local rule = {}
  for i = 1, #columns do rule[i] = string.rep('─', widths[i]) end
  local out = { line(columns), table.concat(rule, '─┼─') }
  for _, row in ipairs(rows) do out[#out + 1] = line(row) end

  local spans, pos = {}, 1
  for i = 1, #columns do
    spans[i] = { pos, pos + widths[i] - 1 }
    pos = pos + widths[i] + SEP_WIDTH
  end
  return out, spans
end

--- One JSON document per line, so `/` search and `yy` work per document.
function M.documents(items)
  local out = {}
  for _, item in ipairs(items) do out[#out + 1] = vim.json.encode(item) end
  return out
end

--- Which column index a display column (virtcol) falls in. The separator
--- after a column counts as that column, so the cursor is never "between".
function M.column_at(spans, virtcol)
  for i, span in ipairs(spans) do
    if virtcol <= span[2] + SEP_WIDTH then return i end
  end
  return #spans > 0 and #spans or nil
end

local function csv_field(value)
  value = tostring(value)
  if value:find('[",\r\n]') then return '"' .. value:gsub('"', '""') .. '"' end
  return value
end

function M.csv(columns, rows)
  local out = { table.concat(vim.tbl_map(csv_field, columns), ',') }
  for _, row in ipairs(rows) do
    local fields = {}
    for i = 1, #columns do fields[i] = csv_field(row[i] or '') end
    out[#out + 1] = table.concat(fields, ',')
  end
  return table.concat(out, '\n') .. '\n'
end

function M.tsv(columns, rows, with_header)
  local out = {}
  if with_header then out[1] = table.concat(columns, '\t') end
  for _, row in ipairs(rows) do
    local fields = {}
    for i = 1, #columns do fields[i] = cell(row[i] or '') end
    out[#out + 1] = table.concat(fields, '\t')
  end
  return table.concat(out, '\n')
end

--- Array of objects, keys in column order. The literal cell text "NULL" --
--- every SQL driver's null sentinel -- becomes a real JSON null, the same
--- rule as the TUI's own JSON export.
function M.json(columns, rows)
  local objects = {}
  for _, row in ipairs(rows) do
    local fields = {}
    for i, name in ipairs(columns) do
      local value = row[i]
      local encoded = (value == nil or value == 'NULL') and 'null' or vim.json.encode(tostring(value))
      fields[i] = vim.json.encode(name) .. ':' .. encoded
    end
    objects[#objects + 1] = '{' .. table.concat(fields, ',') .. '}'
  end
  return '[' .. table.concat(objects, ',\n ') .. ']\n'
end

function M.markdown(columns, rows)
  local function esc(v) return (cell(v):gsub('|', '\\|')) end
  local out = { '| ' .. table.concat(vim.tbl_map(esc, columns), ' | ') .. ' |' }
  out[2] = '|' .. string.rep(' --- |', #columns)
  for _, row in ipairs(rows) do
    local fields = {}
    for i = 1, #columns do fields[i] = esc(row[i] or '') end
    out[#out + 1] = '| ' .. table.concat(fields, ' | ') .. ' |'
  end
  return table.concat(out, '\n') .. '\n'
end

--- One table and its columns, for a picker preview.
function M.entry(entry)
  local prefix = entry.schema and (entry.schema .. '.') or ''
  local lines = { prefix .. entry.name .. (entry.object_kind and ('  [' .. entry.object_kind .. ']') or '') }
  if entry.kind then lines[1] = lines[1] .. '  (' .. entry.kind .. ')' end
  lines[#lines + 1] = ''
  for _, col in ipairs(entry.columns or {}) do
    local marks = (col.primary_key and '  pk' or '') .. (col.indexed and '  idx' or '')
    local fk = col.foreign_key and ('  → ' .. col.foreign_key.table .. '.' .. col.foreign_key.column) or ''
    lines[#lines + 1] = ('%-24s %s%s%s'):format(col.name, col.type, marks, fk)
  end
  if #(entry.columns or {}) == 0 then lines[#lines + 1] = '(no column detail for this backend)' end
  return lines
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
