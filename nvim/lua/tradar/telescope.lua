-- Telescope pickers: tables (with a columns preview) and query history.
local M = {}

local function deps()
  local ok, pickers = pcall(require, 'telescope.pickers')
  if not ok then
    vim.notify('tradar: telescope.nvim is not installed', vim.log.levels.ERROR)
    return nil
  end
  return {
    pickers = pickers,
    finders = require('telescope.finders'),
    conf = require('telescope.config').values,
    actions = require('telescope.actions'),
    state = require('telescope.actions.state'),
    previewers = require('telescope.previewers'),
  }
end

--- `<CR>` inserts the name where you were typing; `<C-o>` opens the table
--- (the driver's own "show me this" statement, run immediately).
function M.tables(opts)
  local t = deps()
  if not t then return end
  local tradar = require('tradar')
  local render = require('tradar.render')
  opts = opts or {}
  tradar.schema_entries(function(entries, buf)
    t.pickers.new(opts, {
      prompt_title = 'tradar tables  (<CR> insert · <C-o> open)',
      finder = t.finders.new_table({
        results = entries,
        entry_maker = function(e)
          local label = render.qualified(e)
          return {
            value = e,
            ordinal = label,
            display = label .. (e.object_kind and ('  [' .. e.object_kind .. ']') or ''),
          }
        end,
      }),
      sorter = t.conf.generic_sorter(opts),
      previewer = t.previewers.new_buffer_previewer({
        title = 'columns',
        define_preview = function(self, entry)
          vim.api.nvim_buf_set_lines(self.state.bufnr, 0, -1, false, render.entry(entry.value))
        end,
      }),
      attach_mappings = function(prompt_bufnr, map)
        t.actions.select_default:replace(function()
          local entry = t.state.get_selected_entry()
          t.actions.close(prompt_bufnr)
          if entry then tradar.insert_text(buf, render.qualified(entry.value)) end
        end)
        map({ 'i', 'n' }, '<C-o>', function()
          local entry = t.state.get_selected_entry()
          t.actions.close(prompt_bufnr)
          if entry then tradar.open_table(buf, entry.value) end
        end)
        return true
      end,
    }):find()
  end)
end

--- `<CR>` pastes the query below the cursor; `<C-r>` runs it right away.
function M.history(opts)
  local t = deps()
  if not t then return end
  local tradar = require('tradar')
  opts = opts or {}
  local buf = vim.api.nvim_get_current_buf()
  t.pickers.new(opts, {
    prompt_title = 'tradar history  (<CR> paste · <C-r> run)',
    finder = t.finders.new_table({
      results = tradar.history(),
      entry_maker = function(h)
        local oneline = h.text:gsub('%s+', ' ')
        return { value = h, ordinal = h.conn .. ' ' .. h.text, display = ('%-12s %s'):format(h.conn, oneline) }
      end,
    }),
    sorter = t.conf.generic_sorter(opts),
    previewer = t.previewers.new_buffer_previewer({
      title = 'query',
      define_preview = function(self, entry)
        vim.api.nvim_buf_set_lines(self.state.bufnr, 0, -1, false, vim.split(entry.value.text, '\n', { plain = true }))
        vim.bo[self.state.bufnr].filetype = 'sql'
      end,
    }),
    attach_mappings = function(prompt_bufnr, map)
      t.actions.select_default:replace(function()
        local entry = t.state.get_selected_entry()
        t.actions.close(prompt_bufnr)
        if not entry then return end
        local win = vim.fn.bufwinid(buf)
        if win ~= -1 then vim.api.nvim_set_current_win(win) end
        local row = vim.api.nvim_win_get_cursor(0)[1]
        vim.api.nvim_buf_set_lines(buf, row, row, false, vim.split(entry.value.text, '\n', { plain = true }))
      end)
      map({ 'i', 'n' }, '<C-r>', function()
        local entry = t.state.get_selected_entry()
        t.actions.close(prompt_bufnr)
        if entry then
          local win = vim.fn.bufwinid(buf)
          if win ~= -1 then vim.api.nvim_set_current_win(win) end
          local lines = vim.split(entry.value.text, '\n', { plain = true })
          vim.api.nvim_buf_set_lines(buf, -1, -1, false, { '' })
          vim.api.nvim_buf_set_lines(buf, -1, -1, false, lines)
          local last = vim.api.nvim_buf_line_count(buf)
          vim.api.nvim_win_set_cursor(0, { last - #lines + 1, 0 })
          tradar.run()
        end
      end)
      return true
    end,
  }):find()
end

return M
