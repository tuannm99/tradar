local ctx = require('tradar.ctx')
local M = ctx.M
local render = ctx.render
local opts = ctx.opts
local state = ctx.state
local notify = ctx.notify
local show_help = ctx.show_help

ctx.attach_results = function(buf)
  if vim.b[buf].tradar_results_attached then return end
  vim.b[buf].tradar_results_attached = true
  local function map(mode, lhs, rhs, desc) vim.keymap.set(mode, lhs, rhs, { buffer = buf, desc = 'tradar: ' .. desc }) end

  map('n', 'gyc', function()
    local row, col = ctx.cell_at()
    if not row then return end
    local _, rows = ctx.tabular(state.result)
    ctx.yank(tostring(rows[row][col] or ''), 'cell')
  end, 'yank cell')
  map('n', 'gyr', function()
    local row = ctx.cell_at()
    if not row then return end
    local cols, rows = ctx.tabular(state.result)
    ctx.yank(render.tsv(cols, { rows[row] }, false), 'row')
  end, 'yank row (tab-separated)')
  map('x', 'gyr', function()
    local a, b = vim.fn.line('v') - 2, vim.fn.line('.') - 2
    if a > b then a, b = b, a end
    local cols, all = ctx.tabular(state.result)
    local rows = {}
    for i = math.max(a, 1), math.min(b, #(all or {})) do rows[#rows + 1] = all[i] end
    vim.api.nvim_feedkeys(vim.keycode('<Esc>'), 'nx', false)
    if #rows > 0 then ctx.yank(render.tsv(cols, rows, false), #rows .. ' rows') end
  end, 'yank selected rows (tab-separated)')
  map('n', 'gyC', function()
    local _, col = ctx.cell_at()
    if not col then return end
    local cols, rows = ctx.tabular(state.result)
    local values = {}
    for _, row in ipairs(rows) do values[#values + 1] = tostring(row[col] or '') end
    ctx.yank(table.concat(values, '\n'), 'column ' .. cols[col])
  end, 'yank column')
  map('n', 'gyj', function() M.yank_all('json') end, 'yank all as JSON')
  map('n', 'gyv', function() M.yank_all('csv') end, 'yank all as CSV')
  map('n', 'gym', function() M.yank_all('md') end, 'yank all as Markdown table')
  -- Not `gT`: that shadows the builtin previous-tab binding (real risk now
  -- that gRPC's request buffers routinely live in their own tabs). Same
  -- prefix every other tradar action already uses, kept out of the
  -- plain-`g`-prefixed family on purpose for that reason.
  map('n', (opts.prefix or '<leader>r') .. 'v', function() M.toggle_view() end, 'result: table <-> JSON')
  map('n', (opts.prefix or '<leader>r') .. 'z', function() M.zoom() end, 'zoom the results window to full screen / restore')
  map('n', (opts.prefix or '<leader>r') .. 'w', function() M.write() end, 'commit grid edits (same as :w)')
  map('n', (opts.prefix or '<leader>r') .. 'u', function() M.revert() end, 'revert grid edits')
  map('n', 'gK', function() M.row_view() end, 'show this row, one field per line')
  map('n', (opts.prefix or '<leader>r') .. 'R', function() M.refresh() end, 'run this result again (read-only statements)')
  map('n', (opts.prefix or '<leader>r') .. 'W', function() M.toggle_watch() end, 'watch: re-run this result every few seconds / stop')
  map('n', '[r', function() M.step_result(-1) end, 'previous kept result')
  map('n', ']r', function() M.step_result(1) end, 'next kept result')
  map('n', 'zw', function() M.toggle_column_width() end, 'show this column in full / cut it back')
  map('n', 'zW', function() M.toggle_column_width(true) end, 'show every column in full / cut them all back')
  map('n', 'gd', function() M.follow_fk() end, 'follow foreign key under cursor')
  -- The grid is edited with plain vim; these keys only differ when the
  -- result cannot be edited, where they say why instead of "E21".
  for _, key in ipairs({ 'i', 'a', 'o', 'O', 'dd', 'cc', 'cw', 'x', 'D', 'C', 's', 'S', 'p', 'P' }) do
    vim.keymap.set('n', key, function()
      if vim.bo[buf].modifiable then return key end
      local r = state.result
      local reason = (r and r.view == 'json') and 'read-only in the JSON view — ' .. (opts.prefix or '<leader>r') .. 'v for the table'
        or (r and r.edit_reason) or 'this result cannot be edited'
      notify(reason, vim.log.levels.WARN)
      return ''
    end, { buffer = buf, expr = true, remap = false, desc = 'tradar: edit the grid in place' })
  end
  vim.api.nvim_create_autocmd('BufWriteCmd', { buffer = buf, callback = function() M.write() end })
  -- `K` is free here (the SQL buffer's own `K` is schema hover, a
  -- different buffer) -- same "K = more about what's under the cursor"
  -- idiom, useful once a cell is wider than the grid's own column cap
  -- and its text got truncated (see `render.MAX_COLUMN_WIDTH`; the last column is allowed `MAX_LAST_COLUMN_WIDTH`).
  map('n', 'K', function()
    local row, col = ctx.cell_at()
    if not row then return end
    local _, rows = ctx.tabular(state.result)
    local value = tostring(rows[row][col] or '')
    show_help(vim.split(value, '\n', { plain = true }))
  end, 'preview the full value of this cell')
  -- A plain vim yank copies what is on screen, `…` included. Put the full
  -- text of any cut cell the yank covered into the register instead
  -- (`gyc` / `gyr` already do, from the raw rows; this is for `yy`, `y$`,
  -- visual `y`...). Block-wise yanks are left as they are.
  vim.api.nvim_create_autocmd('TextYankPost', {
    buffer = buf,
    callback = function()
      local ev = vim.v.event
      if ev.operator ~= 'y' or ev.regtype:byte() == 22 then return end
      local text = table.concat(ev.regcontents, '\n')
      if not text:find('…', 1, true) then return end
      local cols, rows, widths = ctx.grid_of(state.result)
      if not cols or ctx.dirty() then return end
      local a, b = vim.api.nvim_buf_get_mark(buf, '['), vim.api.nvim_buf_get_mark(buf, ']')
      local lines, changed = {}, false
      for l = a[1], b[1] do
        local line = vim.api.nvim_buf_get_lines(buf, l - 1, l, false)[1] or ''
        local s, e = 1, #line
        if ev.regtype == 'v' then
          if l == a[1] then s = a[2] + 1 end
          if l == b[1] then e = b[2] + #(vim.fn.strcharpart(line:sub(b[2] + 1), 0, 1)) end
        end
        local row = rows[l - 2]
        local fixed = row and render.expand_truncated(line, row, widths, s, e)
        lines[#lines + 1] = fixed or line:sub(s, e)
        if fixed and fixed ~= line:sub(s, e) then changed = true end
      end
      if not changed then return end
      local reg = ev.regname ~= '' and ev.regname or '"'
      vim.fn.setreg(reg, lines, ev.regtype)
      if vim.o.clipboard:find('unnamed', 1, true) then pcall(vim.fn.setreg, '+', lines, ev.regtype) end
    end,
  })
  map('n', 'q', function()
    local win = vim.fn.bufwinid(buf)
    if win ~= -1 then vim.api.nvim_win_close(win, true) end
  end, 'close results')
  map('n', 'g?', function()
    show_help({
      '# tradar: results buffer',
      '- `gyc` / `gyr` / `gyC` yank cell / row (visual: many rows) / column',
      '- `gyj` / `gyv` / `gym` yank all as JSON / CSV / Markdown table',
      ('- `%sv` table <-> JSON (SQL rows and documents)'):format(opts.prefix or '<leader>r'),
      ('- `%sm` load the next page of rows'):format(opts.prefix or '<leader>r'),
      ('- `%sz` zoom this window to the full screen / restore (also from the SQL buffer)'):format(opts.prefix or '<leader>r'),
      '- `gK` show the current row, one field per line',
      ('- `%sR` run this result again in place (only statements that cannot change data); `%sW` / `:TradarWatch [secs|off]` repeat it every few seconds'):format(opts.prefix or '<leader>r', opts.prefix or '<leader>r'),
      '- `[r` / `]r` previous / next result kept from this and earlier runs (`:TradarResults` picks one; `setup{ results_kept }`, default 10)',
      '- `zw` show the column under the cursor in full (not cut at 40) / cut it back; `zW` every column',
      '- `gd` follow foreign key under cursor',
      '- edit the grid like any buffer (`cw`, `dd`, `o` + a new row, `:s`, `:g`, `u`…); nothing runs until `:w`',
      '- `:w` lists the UPDATE/DELETE/INSERT statements in a preview; `:w` / `<CR>` there applies them, `q` cancels',
      ('- `%su` revert the grid to the query result'):format(opts.prefix or '<leader>r'),
      '- `K` preview the full value of this cell (newlines kept)',
      '- `q` close',
    })
  end, 'show keymap help')

  -- The next page is a key (`<leader>rm`), not a side effect of moving the
  -- cursor: auto-loading on scroll re-rendered the whole grid under the
  -- user's hands (and `gg` could land on a repaint). `setup{ auto_load_more
  -- = true }` brings the old behaviour back.
  map('n', (opts.prefix or '<leader>r') .. 'm', function() M.more() end, 'load the next page')
  vim.api.nvim_create_autocmd('CursorMoved', {
    buffer = buf,
    callback = function()
      local r = state.result
      if opts.auto_load_more and r and r.shown < r.total
          and vim.api.nvim_win_get_cursor(0)[1] >= vim.api.nvim_buf_line_count(buf) - 3 then
        M.more()
      end
    end,
  })
end
