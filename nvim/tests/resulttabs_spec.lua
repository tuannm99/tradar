-- One result tab per target (table / collection / index / key): running
-- another query on the same target replaces that tab, other tabs stay,
-- each tab is its own buffer, `[r` `]r` `<leader>r1`.. and a click switch.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table p(id integer primary key, name text);
insert into p values (1,'a'),(2,'b'),(3,'c'),(4,'d'),(5,'e');
create table q(id integer primary key, v text);
insert into q values (1,'x'),(2,'y'),(3,'z');
create table r(id integer primary key);
insert into r values (1);
create table s(id integer primary key);
insert into s values (1)]])
local t = H.setup({ page_size = 2, results_kept = 4, results_cursors = 2 })
local state = require('tradar.ctx').state
local slots = require('tradar.slots')
H.sql_buffer('tabs.sql', { '' })
H.check('connects', H.connect())

-- the target of a statement
H.check('SQL: first table of a SELECT', slots.target_of('SELECT a, b FROM public.users u WHERE x = 1') == 'public.users')
H.check('SQL: quotes are dropped', slots.target_of('select * from "Orders"') == 'Orders')
H.check('SQL: a leading comment is skipped', slots.target_of('-- hi\nselect * from t') == 't')
H.check('SQL: EXPLAIN has no target', slots.target_of('explain select * from t') == nil)
H.check('SQL: SELECT without FROM has none', slots.target_of('select 1') == nil)
H.check('Mongo: db.coll.find', slots.target_of('db.users.find({})') == 'users')
H.check('Mongo: getCollection', slots.target_of('db.getCollection("my-coll").find({})') == 'my-coll')
H.check('Mongo: runCommand has none', slots.target_of('db.runCommand({ping: 1})') == nil)
H.check('ES: GET /index/_search', slots.target_of('GET /logs-2024/_search\n{}') == 'logs-2024')
H.check('ES: _cat has none', slots.target_of('GET /_cat/indices') == nil)
H.check('Redis: HGETALL key', slots.target_of('HGETALL user:1') == 'user:1')
H.check('same target, other filter: same slot', slots.key('c', 'select * from t where a=1') == slots.key('c', 'select b from t'))
H.check('other connection: other slot', slots.key('c', 'select * from t') ~= slots.key('d', 'select * from t'))

local function log() return state.result_log or {} end
local function cur() return log()[state.result_pos or 0] end
local function wait_on(label, needle)
  return H.wait(5000, function()
    local e = cur()
    return e and e.label == label and (not needle or (e.query or ''):find(needle, 1, true) ~= nil)
  end)
end
local function in_results() vim.api.nvim_set_current_win(vim.fn.bufwinid(H.rbuf())) end
local function step(d) in_results() t.step_result(d) end
local function bar() return vim.wo[vim.fn.bufwinid(H.rbuf())].winbar end

-- run all: three statements, three tabs
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { "select id as first_col from p;", "update p set name = 'z' where id = 1;", 'select v as last_col from q;' })
t.run(nil, nil, true)
H.check('the last statement is shown', wait_on('q') and H.text():find('last_col', 1, true), H.text())
H.check('three tabs', #log() == 3, #log())
local bq = H.rbuf()
H.check('the buffer is named after the target', vim.api.nvim_buf_get_name(bq):find('tradar://results/q', 1, true) ~= nil, vim.api.nvim_buf_get_name(bq))
H.check('the footer says which tab', H.lines()[#H.lines()]:find('[3/3]', 1, true) ~= nil, H.text())
H.check('the tab bar lists them', bar():find(' 1 p', 1, true) and bar():find(' 3 q', 1, true) and bar():find('TabLineSel', 1, true), bar())

step(-1)
H.check('[r shows the UPDATE before it', H.text():find('row(s) affected', 1, true) and H.text():find('[2/3]', 1, true), H.text())
step(-1)
local bp = H.rbuf()
H.check('[r again shows the first SELECT, in its own buffer', H.text():find('first_col', 1, true) and bp ~= bq, H.text())
H.check('the other tab keeps its buffer', vim.api.nvim_buf_is_valid(bq) and table.concat(vim.api.nvim_buf_get_lines(bq, 0, -1, false), '\n'):find('last_col', 1, true) ~= nil)
step(-1)
H.check('[r wraps to the last tab', H.text():find('last_col', 1, true) ~= nil)
step(1)
H.check(']r wraps to the first', H.text():find('first_col', 1, true) ~= nil)

-- tabs keep their cursor
in_results()
vim.api.nvim_win_set_cursor(0, { 4, 0 })
t.goto_result(3)
H.check('<leader>r3 jumps to tab 3', H.text():find('last_col', 1, true) ~= nil)
t.goto_result(1)
H.check('coming back restores the cursor', vim.api.nvim_win_get_cursor(vim.fn.bufwinid(H.rbuf()))[1] == 4)
H.notes = {}
t.goto_result(9)
H.check('a missing tab says so', #H.notes_with('no result tab 9') == 1, vim.inspect(H.notes))

-- a click on the tab bar
_G.__tradar_result_click(3, 1, 'l')
H.check('clicking a tab shows it', H.text():find('last_col', 1, true) ~= nil)

-- an older tab with an open cursor still pages
t.goto_result(1)
H.check('tab 1 has more pages', H.lines()[#H.lines()]:find('loads the next page', 1, true) ~= nil, H.text())
in_results()
t.more()
H.check('and loads them', H.wait(3000, function() return H.lines()[#H.lines()]:find('4/5', 1, true) ~= nil end), H.text())

-- the same target again replaces its tab
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.run('select name as second_col from p')
H.check('re-running a target replaces its tab', wait_on('p', 'second_col') and #log() == 3 and state.result_pos == 1, #log())
H.check('...in the same buffer', H.rbuf() == bp)
H.check('...with the new rows', H.text():find('second_col', 1, true) ~= nil and H.text():find('first_col', 1, true) == nil)

-- results_cursors = 2: a third open result closes the least recently used
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.run('select id as r_col from r')
H.check('a fourth target is a fourth tab', wait_on('r') and #log() == 4, #log())
local open = 0
for _, e in ipairs(log()) do if e.id then open = open + 1 end end
H.check('only results_cursors stay open', open == 2, open)
local by = {}
for _, e in ipairs(log()) do by[e.label] = e end
H.check('the least recently used open one was closed', by.q.id == nil and by.p.id ~= nil and by.r.id ~= nil, vim.inspect({ by.p.id, by.q.id, by.r.id }))
step(-3)
H.check('a closed tab is still viewable, read-only', cur().label == 'p' or cur().label == 'q')

-- results_kept = 4: a fifth target drops the least recently used tab
local lru
for _, e in ipairs(log()) do
  if not lru or (e.used or 0) < (lru.used or 0) then lru = e end
end
local lru_label, lru_buf = lru.label, lru.buf
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.run('select id as s_col from s')
H.check('a fifth target keeps four tabs', wait_on('s') and #log() == 4, #log())
local labels = {}
for _, e in ipairs(log()) do labels[#labels + 1] = e.label end
H.check('...and drops the least recently used', not vim.tbl_contains(labels, lru_label) and vim.tbl_contains(labels, 's'), vim.inspect({ labels, lru_label }))
H.check('...its buffer goes too', lru_buf == nil or H.wait(2000, function() return not vim.api.nvim_buf_is_valid(lru_buf) end))

-- closing a tab
local before = #log()
in_results()
t.close_result()
H.check('<leader>rx closes the tab on screen', #log() == before - 1 and cur() ~= nil, #log())

-- picking
vim.ui.select = function(items, opts, cb)
  H.check('the picker lists every tab', #items == #log(), vim.inspect(items))
  H.check('...and marks the one shown', opts.format_item(state.result_pos):find('◀', 1, true) ~= nil)
  cb(1)
end
in_results()
vim.cmd('TradarResults')
H.check(':TradarResults jumps to the picked tab', state.result_pos == 1)

-- unsaved grid edits block navigation
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.run('select id, name from p order by id')
H.check('a single-table SELECT is editable', wait_on('p', 'order by') and H.wait(3000, function() return vim.bo[H.rbuf()].modifiable end))
in_results()
local lines = H.lines()
vim.api.nvim_buf_set_lines(H.rbuf(), 2, 3, false, { (lines[3]:gsub('1', '9')) })
H.notes = {}
local pos = state.result_pos
t.step_result(1)
H.check('stepping away with unsaved edits is refused', #H.notes_with('unsaved grid edits') == 1 and state.result_pos == pos, vim.inspect(H.notes))
t.revert()
H.done()
