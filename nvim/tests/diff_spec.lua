-- :TradarDiff -- what changed between two results.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table p(id integer primary key, name text, age integer);
insert into p values (1,'ann',30),(2,'bob',25),(3,'cid',40);
create table q(id integer primary key, name text);
insert into q values (1,'ann'),(9,'zed')]])
local t = H.setup({})
local rdiff = require('tradar.rdiff')
local state = require('tradar.ctx').state
H.sql_buffer('diff.sql', { '' })
H.check('connects', H.connect())

-- pure
local d = rdiff.compare({ 'id', 'name' }, { { '1', 'a' }, { '2', 'b' }, { '3', 'c' } }, { 'id', 'name' }, { { '1', 'a' }, { '2', 'B' }, { '4', 'd' } }, { 'id' })
H.check('one added, one removed, one changed, one same', #d.added == 1 and #d.removed == 1 and #d.changed == 1 and d.same == 1, vim.inspect(d))
H.check('the change names the column and both values', d.changed[1].cells[1].col == 'name' and d.changed[1].cells[1].from == 'b' and d.changed[1].cells[1].to == 'B')
H.check('keys are named', d.added[1].key == 'id=4' and d.removed[1].key == 'id=3')
local only = rdiff.compare({ 'id', 'x' }, { { '1', 'a' } }, { 'id', 'y' }, { { '1', 'b' } })
H.check('columns in only one result are listed, not compared', only.only_a[1] == 'x' and only.only_b[1] == 'y' and #only.changed == 0)
H.check('no common column is refused', rdiff.compare({ 'a' }, {}, { 'b' }, {}) == nil)
local dup = rdiff.compare({ 'k' }, { { '1' }, { '1' } }, { 'k' }, { { '1' } })
H.check('a repeated key is matched in order', #dup.removed == 1 and dup.same == 1)

local function run(sql, col)
  vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
  H.run(sql)
  return H.wait(5000, function() local r = state.result; return r and r.query == sql and H.text():find(col, 1, true) ~= nil end)
end
local function diff_text()
  local b = vim.fn.bufnr('tradar://diff')
  return b ~= -1 and table.concat(vim.api.nvim_buf_get_lines(b, 0, -1, false), '\n') or ''
end

H.check('first run', run('select id, name, age from p order by id', 'ann'))
H.notes = {}
t.diff_result()
H.check('with nothing to compare it says so', #H.notes_with('nothing to compare') == 1, vim.inspect(H.notes))

H.check('an update runs', H.run_until("update p set age = 31 where id = 1", 'row(s) affected'))
H.check('second run of the same target', run('select id, name, age from p order by id limit 3', 'ann'))
t.diff_result()
H.check('the previous run against this one: one cell changed', diff_text():find('0 added · 0 removed · 1 changed · 2 same', 1, true) ~= nil, diff_text())
H.check('...naming the row and the cell', diff_text():find('~ id=1', 1, true) ~= nil and diff_text():find('age: 30 → 31', 1, true) ~= nil, diff_text())

-- against another tab
H.check('another table', run('select id, name from q', 'zed'))
vim.cmd('only')
t.diff_result(1)
H.check('tab n against the one on screen: id 1 same, 2 and 3 removed, 9 added', diff_text():find('1 added · 2 removed · 0 changed · 1 same', 1, true) ~= nil, diff_text())
H.check('...columns only in one are listed', diff_text():find('only before: age', 1, true) ~= nil, diff_text())
H.check('...the removed rows are shown', diff_text():find('- id=2 | 2 | bob', 1, true) ~= nil, diff_text())
H.notes = {}
t.diff_result(7)
H.check('a missing tab is refused', #H.notes_with('is not another table result') == 1, vim.inspect(H.notes))
H.done()
