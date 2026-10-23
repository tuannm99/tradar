-- Editing rows in the results grid: :w -> preview -> commit in one transaction.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table n(id integer primary key, val text, note text);
insert into n values (1,'a','x'),(2,'b','y'),(3,'c','z'),(4,'d','w');
create table p(id integer primary key, name text, age int, city text, zip text, note text)]])
local t = H.setup()
H.sql_buffer('rowedit.sql')
H.check('connects', H.connect())
local function db() return table.concat(vim.tbl_map(function(r) return table.concat(r, ':') end, H.sql('select * from n order by id')), ',') end

H.check('select', H.run_until('select id, val, note from n order by id', '4/4'))
local b = H.rbuf()
H.wait(3000, function() return vim.bo[b].modifiable end)
H.check('grid is editable once its source is known', vim.bo[b].modifiable and not vim.bo[b].modified)

local win = vim.fn.bufwinid(b)
vim.api.nvim_set_current_win(win)
local L = H.lines() -- header, rule, 4 rows, '', footer
vim.api.nvim_buf_set_lines(b, 2, 3, false, { (L[3]:gsub('a', 'A'):gsub('x', 'X')) })
vim.api.nvim_win_set_cursor(win, { 4, 0 }); vim.cmd('normal! dd')
vim.api.nvim_buf_set_lines(b, 5, 5, false, { '9 │ new │ ' })
H.check('buffer is modified', vim.bo[b].modified)

vim.cmd('write')
local pv
H.wait(3000, function() pv = vim.fn.bufnr('tradar://commit'); return pv ~= -1 and #vim.api.nvim_buf_get_lines(pv, 0, -1, false) > 1 end)
local pl = pv ~= -1 and vim.api.nvim_buf_get_lines(pv, 0, -1, false) or {}
H.check('preview: one combined update, one delete, one insert',
  #pl == 4 and pl[1]:find('1 update(s), 1 delete(s), 1 insert(s)', 1, true) and pl[2]:find('"val" = ', 1, true) and pl[2]:find('"note" = ', 1, true), vim.inspect(pl))
H.check('database untouched before confirming', db() == '1:a:x,2:b:y,3:c:z,4:d:w', db())

vim.api.nvim_set_current_win(vim.fn.bufwinid(pv))
vim.api.nvim_feedkeys(vim.api.nvim_replace_termcodes('<CR>', true, false, true), 'x', false)
local want = '1:A:X,3:c:z,4:d:w,9:new:NULL'
H.wait(8000, function() return db() == want end)
H.check('committed', db() == want, db())
H.wait(3000, function() return H.text():find('9  ', 1, true) and not vim.bo[b].modified end)
H.check('grid reloaded and clean', H.text():find('4/4', 1, true) and not vim.bo[b].modified, H.text())

vim.api.nvim_buf_set_lines(b, 2, 3, false, { (H.lines()[3]:gsub('A', 'ZZ')) })
t.revert()
H.check('revert restores the grid', H.lines()[3]:find('A', 1, true) and not vim.bo[b].modified)

vim.api.nvim_buf_set_lines(b, 0, 1, false, { 'oops' }); pcall(vim.cmd, 'write')
H.check('editing the header opens no preview', vim.fn.bufwinid(vim.fn.bufnr('tradar://commit')) == -1)
t.revert()

H.check('a join runs', H.run_until('select n.id, n.val from n join n m on m.id = n.id', '4/4'))
H.sleep(500)
H.check('a join result is read-only', not vim.bo[H.rbuf()].modifiable)

-- INSERT columns follow the grid order, whatever order the table has
H.check('wide select', H.run_until('select * from p', 'rows'))
-- each target has its own buffer: wait for p's, not the join's footer still on screen
H.wait(3000, function() local r = require('tradar.ctx').state.result; return r and r.label == 'p' and r.query == 'select * from p' end)
b = H.rbuf()
H.wait(3000, function() return vim.bo[b].modifiable end)
vim.api.nvim_set_current_win(vim.fn.bufwinid(b))
vim.api.nvim_buf_set_lines(b, 2, 2, false, { '7 │ bob │ 30 │ hanoi │ 100 │ hi' })
vim.cmd('write')
H.wait(3000, function() pv = vim.fn.bufnr('tradar://commit'); return pv ~= -1 and vim.fn.bufwinid(pv) ~= -1 end)
local text = pv ~= -1 and table.concat(vim.api.nvim_buf_get_lines(pv, 0, -1, false), '\n') or 'no preview'
H.check('INSERT names the columns in grid order', text:find('("id", "name", "age", "city", "zip", "note")', 1, true), text)

H.done()
