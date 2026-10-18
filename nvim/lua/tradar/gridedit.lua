-- Edit-in-place for a SQL result grid: turns the *text* of the grid back
-- into row changes. Pure functions (no buffers, no server) -- see
-- docs/backlog/grid-edit-in-place-2026-10-22.md for why rows are matched by
-- their key columns rather than by line number or hidden ids.
local render = require('tradar.render')

local M = {}

local function key_text(cells, key_idx)
  local parts = {}
  for i, idx in ipairs(key_idx) do parts[i] = cells[idx] end
  return table.concat(parts, '\1')
end

--- Compares the grid's current data lines with the rows it was painted
--- from and returns what changed:
---
---   updates = { { row = i, cells = { [col_index] = new_text, ... } } }
---   deletes = { { row = i } }          -- originals that are gone
---   inserts = { { values = { [col_index] = text, ... } } }  -- unknown key
---   errors  = { 'line 3: ...' }        -- anything it will not guess about
---
--- `rows`/`widths` are the original (raw) rows and the column widths they
--- were painted at; `key_idx` the indices of the key columns. A line is
--- matched to its original by the key cells it shows: whatever vim did to
--- get there (dd, yyp, :g/x/d, :sort, cc ...) does not matter. A blank
--- line is ignored; a key that is not an original's is a new row; a key
--- seen twice is an error rather than a guess.
function M.diff(columns, rows, widths, key_idx, lines)
  local result = { updates = {}, deletes = {}, inserts = {}, errors = {} }
  local ncols = #columns

  local shown = {} -- per original row: its displayed cells
  local by_key = {}
  for i, row in ipairs(rows) do
    local cells = {}
    for c = 1, ncols do cells[c] = render.display_cell(row[c] or '', widths[c]) end
    shown[i] = cells
    local k = key_text(cells, key_idx)
    if by_key[k] then
      result.errors[#result.errors + 1] =
        'two rows show the same key (' .. k:gsub('\1', ', ') .. ') — widen the key column or select it untruncated'
      return result
    end
    by_key[k] = i
  end

  local seen = {}
  for n, line in ipairs(lines) do
    if not line:match('^%s*$') then
      local cells = render.split_line(line, ncols)
      if not cells then
        result.errors[#result.errors + 1] =
          ('line %d: cannot read it as %d columns (a value containing " │ "?)'):format(n, ncols)
      else
        local i = by_key[key_text(cells, key_idx)]
        if i then
          if seen[i] then
            result.errors[#result.errors + 1] =
              ('line %d: key already used by another line — change the key to add a row, or remove the line'):format(n)
          else
            seen[i] = true
            local changed
            for c = 1, ncols do
              if cells[c] ~= shown[i][c] then
                changed = changed or {}
                changed[c] = cells[c]
              end
            end
            if changed then table.insert(result.updates, { row = i, cells = changed }) end
          end
        else
          local values, any = {}, false
          for c = 1, ncols do
            if cells[c] ~= '' then
              values[c] = cells[c]
              any = true
            end
          end
          if any then table.insert(result.inserts, { values = values }) end
        end
      end
    end
  end
  for i = 1, #rows do
    if not seen[i] then table.insert(result.deletes, { row = i }) end
  end
  return result
end

return M
