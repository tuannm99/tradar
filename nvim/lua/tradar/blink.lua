-- blink.cmp source: completion from tradar-server, so `alias.` offers that
-- table's columns, a table after JOIN ranks FK-related ones first, etc. --
-- exactly what the TUI offers, because it is the same code answering.
local source = {}

-- LSP CompletionItemKind numbers, which blink uses for icons/sorting.
local KIND = { table = 22, column = 5, keyword = 14 } -- Struct, Field, Keyword

function source.new(opts)
  return setmetatable({ opts = opts or {} }, { __index = source })
end

function source:enabled()
  return vim.b.tradar_attached == true and require('tradar').connection_for(0) ~= nil
end

function source:get_trigger_characters()
  return { '.' }
end

function source:get_completions(ctx, callback)
  local buf = ctx.bufnr or vim.api.nvim_get_current_buf()
  local row, col = ctx.cursor[1], ctx.cursor[2]
  local lines = vim.api.nvim_buf_get_lines(buf, 0, row, false)
  lines[#lines] = lines[#lines]:sub(1, col)

  local cancelled = false
  require('tradar').complete(buf, table.concat(lines, '\n'), function(items)
    if cancelled then return end
    local out = {}
    for i, item in ipairs(items) do
      out[i] = {
        label = item.text,
        kind = KIND[item.kind] or 1,
        -- Server order is the ranking (tables before columns before
        -- keywords, FK-related first); keep it as the tie-break.
        sortText = ('%05d'):format(i),
        labelDetails = { description = item.kind },
      }
    end
    callback({ items = out, is_incomplete_backward = false, is_incomplete_forward = false })
  end)
  return function() cancelled = true end
end

return source
