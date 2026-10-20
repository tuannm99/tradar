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

--- Past this many display columns, a cell is truncated with a trailing
--- `…` rather than stretching the column to fit -- one long value (a JSON
--- blob, a long text field) used to drag every other column far to the
--- right with it, matching nothing the TUI does (its own auto-width caps
--- the same way, `docs/backlog/column-resize-2026-09-24.md`). No manual
--- override here yet (TUI's `<`/`>`) -- `K` on the results buffer previews
--- a cell's untruncated value instead, see `init.lua`'s `attach_results`.
M.MAX_COLUMN_WIDTH = 40

local function cell(value)
  return (tostring(value):gsub('[\r\n\t]', ' '))
end

--- `text` padded to `width`, or truncated-with-`…` if it doesn't fit.
--- Character-count truncation (`strcharpart`), not display-width-exact --
--- a cell dense with double-width (CJK) characters can still overflow
--- `width` by a little; good enough without a wcwidth-aware clipper this
--- codebase doesn't otherwise need.
local function pad_or_truncate(text, width, right)
  local w = vim.fn.strdisplaywidth(text)
  if w <= width then
    if right then return string.rep(' ', width - w) .. text end
    return text .. string.rep(' ', width - w)
  end
  return vim.fn.strcharpart(text, 0, math.max(width - 1, 0)) .. '…'
end

--- One value as the grid shows it in a column of `width`: whitespace
--- flattened, truncated with `…` past the width, trimmed of padding. What
--- an unedited cell reads back as, so a diff can tell "untouched" from
--- "retyped" without being fooled by truncation.
function M.display_cell(value, width)
  return vim.trim(pad_or_truncate(cell(value), width))
end

--- Bytes `s..e` of a painted grid line `line`, with every cell that was cut
--- to `…` (value wider than its column) given back its full text -- what a
--- plain vim yank (`yy`, `y$`, visual `y`) should put in the register
--- instead of the truncated display. `row` is the raw row, `widths` the
--- painted column widths. Newlines/tabs in a value stay flattened to spaces,
--- as in the grid (`gyc` yanks the exact value). `nil` when the line does
--- not split into `#widths` cells.
function M.expand_truncated(line, row, widths, s, e)
  local parts = vim.split(line, SEP, { plain = true })
  if #parts ~= #widths then return nil end
  local out, pos, off = {}, s, 1
  for i, part in ipairs(parts) do
    local ell = off + #part - #'…' -- byte offset of a trailing `…`
    if part:sub(-#'…') == '…' and ell >= s and ell + #'…' - 1 <= e then
      local full = cell(row[i] or '')
      if vim.fn.strdisplaywidth(full) > widths[i] then
        out[#out + 1] = line:sub(pos, ell - 1)
        out[#out + 1] = vim.fn.strcharpart(full, math.max(widths[i] - 1, 0))
        pos = ell + #'…'
      end
    end
    off = off + #part + #SEP
  end
  out[#out + 1] = line:sub(pos, e)
  return table.concat(out)
end

--- Splits a grid line back into its `ncols` trimmed cell texts; `nil` when
--- the line does not have exactly that many (a value containing the column
--- separator, or a separator typed/deleted by hand).
function M.split_line(line, ncols)
  local parts = vim.split(line, SEP, { plain = true })
  if #parts ~= ncols then return nil end
  for i, part in ipairs(parts) do parts[i] = vim.trim(part) end
  return parts
end

--- A value that reads as a plain number: `12`, `-3.5`, `1e9`. Leading zeros
--- (`007`, a zip code, an id) stay text -- aligning those as numbers would
--- be a lie about what they are.
local function is_number(text)
  if text:match('^[-+]?0%d') then return false end
  return text:match('^[-+]?%d[%d.]*$') ~= nil
    or text:match('^[-+]?%d[%d.]*[eE][-+]?%d+$') ~= nil
    or text:match('^[-+]?%.%d+$') ~= nil
end

--- Which columns are right-aligned: a column whose every non-empty, non-NULL
--- cell is a number (and that has at least one). `prev` is the answer for
--- the rows painted so far, so an appended page can only *demote* a column
--- to text (`true` -> `false`), never promote one: the caller repaints when
--- that happens, the same as when a column outgrows its width.
function M.number_columns(columns, rows, prev)
  local aligns = {}
  for i = 1, #columns do
    local numeric, seen = prev == nil or prev[i] == true, prev ~= nil and prev[i] == true
    if numeric then
      for _, row in ipairs(rows) do
        local text = cell(row[i] or '')
        if text ~= '' and text ~= 'NULL' then
          if is_number(text) then
            seen = true
          else
            numeric = false
            break
          end
        end
      end
    end
    aligns[i] = numeric and seen
  end
  return aligns
end

--- Each column's display width (header vs. every cell in `rows`, whichever
--- is wider), clamped to `MAX_COLUMN_WIDTH`. `prev` (optional, the same
--- shape this returns) seeds the running max instead of starting from the
--- header alone -- a caller that already painted earlier pages can pass
--- their stored widths and scan only the *new* rows, so finding the
--- running max is O(new rows), not O(every row loaded so far). Widths
--- only ever grow (never shrink back down as old wide values scroll out
--- of view) whether computed this way or from scratch -- same as before
--- this function was split out of `M.table`.
function M.column_widths(columns, rows, prev)
  local widths = {}
  for i, name in ipairs(columns) do
    widths[i] = math.min(prev and prev[i] or vim.fn.strdisplaywidth(name), M.MAX_COLUMN_WIDTH)
  end
  for _, row in ipairs(rows) do
    for i = 1, #columns do
      widths[i] = math.min(math.max(widths[i], vim.fn.strdisplaywidth(cell(row[i] or ''))), M.MAX_COLUMN_WIDTH)
    end
  end
  return widths
end

--- The display-column span each column occupies (`{first, last}`,
--- 1-based, inclusive) at the given `widths` -- what lets a cursor
--- position map back to a cell for yanking (`column_at` below).
function M.spans_for(widths)
  local spans, pos = {}, 1
  for i, w in ipairs(widths) do
    spans[i] = { pos, pos + w - 1 }
    pos = pos + w + SEP_WIDTH
  end
  return spans
end

--- The header name row plus its `─┼─` rule, at `widths`.
function M.header_lines(columns, widths, aligns)
  local parts, rule = {}, {}
  for i, name in ipairs(columns) do
    parts[i] = pad_or_truncate(cell(name), widths[i], aligns and aligns[i])
    rule[i] = string.rep('─', widths[i])
  end
  return { table.concat(parts, SEP), table.concat(rule, '─┼─') }
end

--- One line per row in `rows`, at `widths` -- no header, so a caller
--- appending only the newest page's rows isn't handed lines it didn't ask
--- for.
function M.row_lines(columns, widths, rows, aligns)
  local out = {}
  for r, row in ipairs(rows) do
    local parts = {}
    for i = 1, #columns do
      parts[i] = pad_or_truncate(cell(row[i] or ''), widths[i], aligns and aligns[i])
    end
    out[r] = table.concat(parts, SEP)
  end
  return out
end

--- Aligned text table (header + every row in `rows`) plus the spans and
--- widths computed along the way -- the one-call convenience `init.lua`
--- uses for a full (re)paint; `column_widths`/`header_lines`/`row_lines`
--- above are what let it paint an *appended* page more cheaply instead
--- (see `attach_results`'s `paint_results`).
function M.table(columns, rows, prev_widths)
  local widths = M.column_widths(columns, rows, prev_widths)
  local aligns = M.number_columns(columns, rows)
  local out = M.header_lines(columns, widths, aligns)
  vim.list_extend(out, M.row_lines(columns, widths, rows, aligns))
  return out, M.spans_for(widths), widths, aligns
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

local EMPTY_DICT = getmetatable(vim.empty_dict())

local function is_array(t)
  if getmetatable(t) == EMPTY_DICT then return false end
  return next(t) == nil or vim.islist(t)
end

--- `value` as indented JSON lines. Object keys come out sorted, except that
--- the top-level object follows `order` (the server's own `field_order`
--- for it: Lua tables remember no insertion order, so without that hint
--- `_id` would land wherever the sort puts it). Nested objects have no such
--- hint and stay alphabetical.
function M.pretty(value, order, indent)
  indent = indent or ''
  if type(value) ~= 'table' then return vim.json.encode(value) end
  local inner = indent .. '  '
  if is_array(value) then
    if #value == 0 then return '[]' end
    local parts = {}
    for i, item in ipairs(value) do parts[i] = inner .. M.pretty(item, nil, inner) end
    return '[\n' .. table.concat(parts, ',\n') .. '\n' .. indent .. ']'
  end
  local keys, seen = {}, {}
  for _, k in ipairs(order or {}) do
    if value[k] ~= nil and not seen[k] then keys[#keys + 1], seen[k] = k, true end
  end
  local rest = {}
  for k in pairs(value) do
    if not seen[k] then rest[#rest + 1] = k end
  end
  table.sort(rest)
  vim.list_extend(keys, rest)
  if #keys == 0 then return '{}' end
  local parts = {}
  for i, k in ipairs(keys) do
    parts[i] = inner .. vim.json.encode(tostring(k)) .. ': ' .. M.pretty(value[k], nil, inner)
  end
  return '{\n' .. table.concat(parts, ',\n') .. '\n' .. indent .. '}'
end

--- Documents as pretty-printed JSON, a blank line between them. `orders`
--- (optional, one key list per item -- the server's `field_order`) keeps
--- each document's top-level fields in their stored order.
function M.documents(items, orders)
  local out = {}
  for i, item in ipairs(items) do
    if i > 1 then out[#out + 1] = '' end
    vim.list_extend(out, vim.split(M.pretty(item, orders and orders[i]), '\n', { plain = true }))
  end
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
        nodes[#lines] = { kind = 'table', key = qualified, insert = qualified, entry = entry }
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
