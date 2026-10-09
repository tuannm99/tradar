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
  if vim.b.tradar_attached ~= true then return false end
  local tradar = require('tradar')
  local row = vim.api.nvim_win_get_cursor(0)[1]
  return tradar.block_connection(0, row) ~= nil
end

function source:get_trigger_characters()
  return { '.' }
end

function source:get_completions(ctx, callback)
  local tradar = require('tradar')
  local buf = ctx.bufnr or vim.api.nvim_get_current_buf()
  local row, col = ctx.cursor[1], ctx.cursor[2]
  local text = tradar.text_before_cursor(buf, row, col)
  if not text then return callback({ items = {}, is_incomplete_backward = false, is_incomplete_forward = false }) end

  local cancelled = false
  tradar.complete(buf, text, function(items)
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
