-- `zw` / `zW`: show a column in full instead of cutting it at 40.
local H = dofile('nvim/tests/helpers.lua')
local long = string.rep('abcdefghij', 8) -- 80 characters
H.sql(([[create table d(id integer primary key, path text, note text);
insert into d values (1, '%s', 'x')]]):format(long))
local t = H.setup()
H.sql_buffer('columns.sql')
H.check('connects', H.connect())
H.check('select', H.run_until('select id, path, note from d', '1/1 rows'))
local b = H.rbuf()
H.wait(3000, function() return vim.bo[b].modifiable end)
local function row() return H.lines()[3] or '' end
H.check('a middle column is cut at 40', row():find('…', 1, true) and not row():find(long, 1, true), row())

local win = vim.fn.bufwinid(b)
vim.api.nvim_set_current_win(win)
vim.api.nvim_win_set_cursor(win, { 3, 0 })
vim.cmd('normal! 0') ; vim.fn.search('│', 'W', 3) ; vim.cmd('normal! 2l') -- into the `path` cell
H.check('zw is mapped', H.mapped(b, 'zw') and H.mapped(b, 'zW'))
t.toggle_column_width()
H.check('zw shows that column in full', row():find(long, 1, true) ~= nil, row())
H.check('the cursor stays on that column', require('tradar.render').column_at(H.t and require('tradar.ctx').state.result.spans, vim.fn.virtcol('.')) == 2)
H.check('the grid is still editable and clean', vim.bo[b].modifiable and not vim.bo[b].modified)
vim.cmd('write')
H.sleep(300)
H.check('no false edits after widening (:w opens no preview)', vim.fn.bufwinid(vim.fn.bufnr('tradar://commit')) == -1)
t.toggle_column_width()
H.check('zw again cuts it back', row():find('…', 1, true) ~= nil, row())

t.toggle_column_width(true)
H.check('zW shows every column in full', row():find(long, 1, true) ~= nil, row())
t.toggle_column_width(true)
H.check('zW again cuts them all back', row():find('…', 1, true) ~= nil, row())

t.toggle_column_width() -- widen `path` once more, then run something new
H.check('a new query starts cut again', H.run_until('select path, id as other from d', 'other') and H.lines()[3]:find('…', 1, true) ~= nil, H.text())
H.done()
