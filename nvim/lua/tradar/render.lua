-- Result pages <-> text. Pure functions: no buffers, no server.
local M = {}

--- `schema.name`, unless the driver already put the schema in the name
--- (Cassandra reports `demo.users` *and* keyspace `demo`) -- no `demo.demo.users`.
function M.qualified(entry)
  if entry.schema and entry.name:sub(1, #entry.schema + 1) ~= entry.schema .. '.' then
    return entry.schema .. '.' .. entry.name
  end
  return entry.name
end

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

--- Documents as a table: one column per field, nested objects flattened to
--- dotted names (`address.city`) like the TUI's grid, arrays kept as compact
--- JSON. Absent fields are empty, real nulls are `NULL`.
---
--- `order` (optional) is the server's own `field_order` for this page
--- (`tradar.protocol.StoredResult::field_order`, one array of top-level key
--- names per document, in insertion order) -- when given, a document's
--- own top-level fields become columns in that order; without it (no
--- server hint, an older server, a non-Mongo driver that never sends one),
--- top-level fields fall back to alphabetical, same as before this
--- existed. A column introduced by a *later* document that an earlier
--- one's `order` never mentioned is appended after, in the position that
--- later document's own order gives it -- `order[i]` only ever lists
--- document `i`'s own fields, not the union every document in the page
--- might end up contributing to `columns`.
---
--- Nested dotted paths are **not** covered by `order` (it is top-level
--- only) and always sort alphabetically, as a block *after* every
--- top-level column -- `pairs()` over a decoded nested object has no
--- order to begin with, and grouping "real" fields before drilled-down
--- ones reads better than interleaving them alphabetically ever did.
--- `_id` sorts first among top-level columns when there is no `order` to
--- say otherwise; an `order` that already puts `_id` first (true for
--- every real MongoDB document -- a server-generated `_id` is always
--- inserted first) keeps it there for free.
function M.flatten(items, order)
  local seen, top_columns, nested_columns, rows = {}, {}, {}, {}
  local function scalar(v)
    if v == nil or v == vim.NIL then return 'NULL' end
    local t = type(v)
    if t == 'string' then return v end
    if t == 'number' or t == 'boolean' then return tostring(v) end
    return vim.json.encode(v)
  end
  local function record(name, bucket)
    if not seen[name] then
      seen[name] = true
      bucket[#bucket + 1] = name
    end
  end
  --- A nested value's own sub-tree, one level or deeper below a top-level
  --- field -- always alphabetical, there is no `order` for this depth.
  local function walk_nested(prefix, value, out)
    if type(value) == 'table' and not vim.islist(value) and next(value) ~= nil then
      for k, child in pairs(value) do walk_nested(prefix .. '.' .. k, child, out) end
    else
      out[prefix] = scalar(value)
      record(prefix, nested_columns)
    end
  end
  local flat = {}
  for i, item in ipairs(items) do
    flat[i] = {}
    if type(item) == 'table' and not vim.islist(item) then
      -- `vim.NIL` (not plain Lua `nil`), when this document wasn't a JSON
      -- object server-side -- `rpc.lua`'s decode only nils out an
      -- *object's* null values, never an array element, since removing
      -- one would shift every index after it.
      local keys = order and order[i]
      if keys == vim.NIL then keys = nil end
      if not keys then
        keys = {}
        for k in pairs(item) do keys[#keys + 1] = k end
      end
      for _, k in ipairs(keys) do
        local value = item[k]
        if value ~= nil then
          if type(value) == 'table' and not vim.islist(value) and next(value) ~= nil then
            walk_nested(k, value, flat[i])
          else
            flat[i][k] = scalar(value)
            record(k, top_columns)
          end
        end
      end
    else
      flat[i].value = scalar(item)
      record('value', top_columns)
    end
  end
  if not order then
    table.sort(top_columns, function(a, b)
      if a == '_id' then return b ~= '_id' end
      if b == '_id' then return false end
      return a < b
    end)
  end
  table.sort(nested_columns)
  local columns = {}
  for _, c in ipairs(top_columns) do columns[#columns + 1] = c end
  for _, c in ipairs(nested_columns) do columns[#columns + 1] = c end
  for i, row in ipairs(flat) do
    local cells = {}
    for j, name in ipairs(columns) do cells[j] = row[name] or '' end
    rows[i] = cells
  end
  return columns, rows
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
  local lines = { M.qualified(entry) .. (entry.object_kind and ('  [' .. entry.object_kind .. ']') or '') }
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

--- Tree navigator: schema/keyspace/database groups, each holding its
--- tables, each collapsible to hide its columns -- same two collapsible
--- levels as the TUI navigator's own `flatten_outline`/`push_table`, minus
--- the extra Tables/Views/Functions/Procedures grouping under a schema
--- (not asked for here, and Neovim's panel is one connection at a time so
--- there's no `[kind]` folder-count pressure the way a cross-connection
--- tree would have).
---
--- Groups are bucketed by first-seen order, not sorted -- a schema list
--- reordered here would disagree with whatever order the driver's own
--- query already returned rows in (same reasoning as `flatten_outline`'s
--- own comment). `entries` with no `schema` at all (SQLite, Elasticsearch,
--- Redis) render with no folder, exactly like before.
---
--- `expanded` is the caller's toggle state, a plain set keyed by a node's
--- own `key` (below) -- schema folders default *open* (just names, cheap
--- to show), tables default *closed* (their columns are the actual bulk a
--- big schema would otherwise dump all at once).
---
--- Returns `lines` and `nodes` (parallel array: `{kind, key, insert}` --
--- `kind` is `"schema"`/`"table"`/`"column"`, `key` is what toggles a
--- foldable node's entry in `expanded`, `insert` is the text `<CR>` should
--- put into the SQL buffer, `nil` for a schema header since there's
--- nothing meaningful to insert for a grouping folder).
function M.schema_tree(entries, expanded)
  expanded = expanded or {}
  local groups, by_schema = {}, {}
  for _, entry in ipairs(entries) do
    local schema = entry.schema
    local group = by_schema[schema or '']
    if not group then
      group = { schema = schema, entries = {} }
      by_schema[schema or ''] = group
      groups[#groups + 1] = group
    end
    group.entries[#group.entries + 1] = entry
  end

  local lines, nodes = {}, {}
  for _, group in ipairs(groups) do
    local open = true
    if group.schema then
      local key = 'schema:' .. group.schema
      open = expanded[key] ~= false
      lines[#lines + 1] = (open and '▾ ' or '▸ ') .. group.schema
      nodes[#lines] = { kind = 'schema', key = key }
    end
    if open then
      local indent = group.schema and '  ' or ''
      for _, entry in ipairs(group.entries) do
        local qualified = M.qualified(entry)
        local has_columns = #(entry.columns or {}) > 0
        local table_open = has_columns and expanded[qualified] == true
        local marker = not has_columns and '  ' or (table_open and '▾ ' or '▸ ')
        local label = entry.name .. (entry.object_kind and ('  [' .. entry.object_kind .. ']') or '')
        lines[#lines + 1] = indent .. marker .. label
        nodes[#lines] = { kind = 'table', key = qualified, insert = qualified }
        if table_open then
          for _, col in ipairs(entry.columns) do
            local marks = (col.primary_key and ' pk' or '') .. (col.indexed and ' idx' or '')
            local fk = col.foreign_key and (' → ' .. col.foreign_key.table .. '.' .. col.foreign_key.column) or ''
            lines[#lines + 1] = indent .. '    ' .. col.name .. '  ' .. col.type .. marks .. fk
            nodes[#lines] = { kind = 'column', insert = col.name }
          end
        end
      end
    end
  end
  return lines, nodes
end

return M
