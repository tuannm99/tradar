-- Filtering and sorting the loaded rows of a result.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table p(id integer primary key, name text, age integer, city text);
insert into p values (1,'ann',31,'Hanoi'),(2,'bob',25,'Hue'),(3,'cid',40,'Hanoi'),(4,'dee',null,'Hue'),(5,'eve',9,'Dalat')]])
local t = H.setup({ page_size = 3 })
local gv = require('tradar.gridview')
local state = require('tradar.ctx').state
H.sql_buffer('view.sql', { '' })
H.check('connects', H.connect())

-- pure
local cols = { 'id', 'name', 'age', 'city' }
local rows = { { '1', 'ann', '31', 'Hanoi' }, { '2', 'bob', '25', 'Hue' }, { '3', 'cid', '40', 'Hanoi' }, { '4', 'dee', 'NULL', 'Hue' }, { '5', 'eve', '9', 'Dalat' } }
local function ids(list) local o = {} for _, r in ipairs(list) do o[#o + 1] = r[1] end return table.concat(o, ',') end
H.check('plain text matches any cell', ids(gv.apply(cols, rows, { filter = 'hanoi' })) == '1,3')
H.check('col = value', ids(gv.apply(cols, rows, { filter = "city = 'Hue'" })) == '2,4')
H.check('col != value', ids(gv.apply(cols, rows, { filter = 'city != hue' })) == '1,3,5')
H.check('col ~ text', ids(gv.apply(cols, rows, { filter = 'name ~ e' })) == '4,5')
H.check('numbers compare as numbers (9 < 25)', ids(gv.apply(cols, rows, { filter = 'age < 26' })) == '2,5')
H.check('>=', ids(gv.apply(cols, rows, { filter = 'age >= 31' })) == '1,3')
H.check('an unknown column falls back to plain text', ids(gv.apply(cols, rows, { filter = 'zz = 1' })) == '')
H.check('sort ascending, numbers as numbers, NULL last', ids(gv.apply(cols, rows, { sort = { col = 3 } })) == '5,2,1,3,4')
H.check('sort descending keeps NULL last', ids(gv.apply(cols, rows, { sort = { col = 3, desc = true } })) == '3,1,2,5,4')
H.check('sort is stable', ids(gv.apply(cols, rows, { sort = { col = 4 } })) == '5,1,3,2,4')
H.check('filter and sort together', ids(gv.apply(cols, rows, { filter = 'hanoi', sort = { col = 3, desc = true } })) == '3,1')

-- in the results buffer
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.check('runs', H.run_until('select id, name, age, city from p order by id', '3/5'))
local function in_results() vim.api.nvim_set_current_win(vim.fn.bufwinid(H.rbuf())) end
local function footer() return H.lines()[#H.lines()] or '' end
H.wait(3000, function() return vim.bo[H.rbuf()].modifiable end)
H.check('editable before', vim.bo[H.rbuf()].modifiable)
in_results()
t.set_filter('city = Hanoi')
H.check('the filter shows only matching loaded rows', H.text():find('ann', 1, true) and not H.text():find('bob', 1, true), H.text())
H.check('the footer says so', footer():find('filter «city = Hanoi»: 2 of 3 loaded', 1, true) ~= nil, footer())
H.check('read-only while filtered', not vim.bo[H.rbuf()].modifiable)
local cols_, shown = require('tradar.ctx').tabular(state.result)
H.check('yank/cell operations see the shown rows', #shown == 2 and shown[1][2] == 'ann' and shown[2][2] == 'cid')
t.set_filter('off')
H.check('off clears it', H.text():find('bob', 1, true) and vim.bo[H.rbuf()].modifiable)

-- sort by the column under the cursor
vim.api.nvim_win_set_cursor(0, { 3, 0 })
vim.cmd(('normal! %d|'):format(state.result.spans[3][1]))
t.toggle_sort()
H.check('sorted ascending by age', footer():find('sorted by age ↑', 1, true) ~= nil and H.lines()[3]:find('bob', 1, true) ~= nil, H.text())
t.toggle_sort()
H.check('...then descending', footer():find('sorted by age ↓', 1, true) ~= nil and H.lines()[3]:find('cid', 1, true) ~= nil, footer())
t.toggle_sort()
H.check('...then off', footer():find('sorted by', 1, true) == nil and H.lines()[3]:find('ann', 1, true) ~= nil)

-- more rows land under the filter
t.set_filter('city = Hue')
in_results()
t.more()
H.check('a loaded page is filtered too', H.wait(3000, function() return footer():find('5/5', 1, true) ~= nil end) and footer():find('2 of 5 loaded', 1, true) ~= nil, footer())

-- a refresh keeps the filter
t.refresh()
H.check('refresh keeps the filter', H.wait(3000, function() return footer():find('filter «city = Hue»', 1, true) ~= nil end), footer())
H.done()
