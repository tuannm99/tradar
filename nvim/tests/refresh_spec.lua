-- `<leader>rR` / :TradarRefresh and :TradarWatch.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table c(id integer primary key, v text); insert into c values (1,'one'),(2,'two')]])
local t = H.setup()
local guard = require('tradar.guard')
local ctx = require('tradar.ctx')

-- ── which statements may be repeated ────────────────────────────────────
local cases = {
  { 'select * from t', nil, true }, { 'SELECT 1; -- delete', nil, true }, { "select 'drop table x'", nil, true },
  { 'select * into backup from t', nil, false }, { 'with a as (select 1) select * from a', nil, true },
  { 'with a as (delete from t returning *) select * from a', nil, false },
  { 'explain select 1', nil, true }, { 'explain analyze delete from t', nil, false },
  { 'insert into t values (1) returning *', nil, false }, { 'update t set a=1', nil, false },
  { "select nextval('s')", nil, false }, { 'show tables', nil, true }, { 'call p()', nil, false }, { '', nil, false },
  { 'select * from t for update', nil, false },
  { 'db.users.find({a: 1}).limit(5)', 'mongo', true }, { 'db.users.deleteMany({})', 'mongo', false },
  { 'db.users.aggregate([{$out: "x"}])', 'mongo', false }, { 'db.runCommand({drop: "x"})', 'mongo', false },
  { 'show collections', 'mongo', true }, { 'db.users.find({}).sort({a: 1})', 'mongo', true },
  { 'GET mykey', 'redis', true }, { 'DEL mykey', 'redis', false }, { 'hgetall h', 'redis', true }, { 'FLUSHALL', 'redis', false },
  { 'GET /idx/_search\n{}', 'elasticsearch', true }, { 'POST idx/_search\n{}', 'elasticsearch', true },
  { 'DELETE idx/_doc/1', 'elasticsearch', false }, { 'PUT idx/_doc/1\n{}', 'elasticsearch', false }, { 'POST idx/_update/1', 'elasticsearch', false },
}
local bad = {}
for _, c in ipairs(cases) do
  if guard.is_read_only(c[1], c[2]) ~= c[3] then bad[#bad + 1] = vim.inspect(c[1]) end
end
H.check('is_read_only: ' .. #cases .. ' SQL / Mongo / Redis / Elasticsearch statements judged right', #bad == 0, table.concat(bad, ' | '))

-- ── refresh ─────────────────────────────────────────────────────────────
H.sql_buffer('refresh.sql', { '' })
H.check('connects', H.connect())
local function footer() return H.lines()[#H.lines()] or '' end
local function in_results() vim.api.nvim_set_current_win(vim.fn.bufwinid(H.rbuf())) end
H.check('select', H.run_until('select id, v from c order by id', '2/2'))
H.wait(3000, function() return vim.bo[H.rbuf()].modifiable end)
local kept = #ctx.state.result_log
local history = #t.history()

H.sql('insert into c values (3, \'three\')')
in_results()
vim.api.nvim_win_set_cursor(0, { 4, 2 })
t.refresh()
H.check('<leader>rR / :TradarRefresh shows the new row', H.wait(4000, function() return H.text():find('three', 1, true) ~= nil end), H.text())
H.check('...in the same slot (nothing added to the kept results)', #ctx.state.result_log == kept, #ctx.state.result_log)
H.check('...without a history entry of its own', #t.history() == history)
H.check('the cursor stays where it was', vim.api.nvim_win_get_cursor(0)[1] == 4 and vim.api.nvim_win_get_cursor(0)[2] == 2, vim.inspect(vim.api.nvim_win_get_cursor(0)))
H.check('and the refreshed grid is editable like the original', H.wait(3000, function() return vim.bo[H.rbuf()].modifiable end))

-- a statement that writes is not repeated
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.check('a write that returns rows runs', H.run_until("insert into c values (9, 'nine') returning id", '9'))
local rows_after = H.sql('select count(*) from c')[1][1]
H.notes = {}
in_results()
t.refresh()
H.check('refresh refuses to repeat a statement that may change data', H.wait(2000, function() return #H.notes_with('not repeated') == 1 end), vim.inspect(H.notes))
H.sleep(300)
H.check('...and it did not run again', H.sql('select count(*) from c')[1][1] == rows_after)

-- unsaved edits block it
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.check('back to a select', H.run_until('select id, v from c order by id', 'three'))
H.wait(3000, function() return vim.bo[H.rbuf()].modifiable end)
in_results()
vim.api.nvim_buf_set_lines(H.rbuf(), 2, 3, false, { (H.lines()[3]:gsub('one', 'ONE')) })
H.notes = {}
t.refresh()
H.check('refresh with unsaved grid edits is refused', #H.notes_with('unsaved grid edits') == 1, vim.inspect(H.notes))
t.revert()

-- a failing refresh keeps the grid
H.sql('create table gone(id integer primary key); insert into gone values (1)')
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.check('select from a table', H.run_until('select id from gone', '1/1'))
H.sql('drop table gone')
H.notes = {}
in_results()
t.refresh()
H.check('a failing refresh says so', H.wait(3000, function() return #H.notes_with('refresh failed') == 1 end), vim.inspect(H.notes))
H.check('...and the old rows stay on screen', H.text():find('1/1 rows', 1, true) ~= nil and H.text():find('no such table', 1, true) == nil, H.text())

-- ── watch ───────────────────────────────────────────────────────────────
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.check('select for the watch', H.run_until('select id, v from c order by id', 'rows'))
H.notes = {}
t.watch('abc')
H.check(':TradarWatch with nonsense is a usage message', #H.notes_with('usage: :TradarWatch') == 1)
t.watch('1')
H.check('watch starts and the footer says so', H.wait(2000, function() return footer():find('watching every 1s', 1, true) ~= nil end), footer())
H.sql("insert into c values (20, 'twenty')")
H.check('the grid follows the table', H.wait(4000, function() return H.text():find('twenty', 1, true) ~= nil end), H.text())
t.watch('off')
H.check('off removes the footer note', H.wait(1000, function() return footer():find('watching', 1, true) == nil end), footer())
H.sql("insert into c values (21, 'twentyone')")
H.sleep(2200)
H.check('after off it no longer follows', H.text():find('twentyone', 1, true) == nil)

-- another result replaces it: the watch gives up
t.watch('1')
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.notes = {}
H.run_until('select 1 as other_query', 'other_query')
H.check('the watch stops when another result takes the screen', H.wait(4000, function() return #H.notes_with('watch stopped: the results on screen changed') == 1 end), vim.inspect(H.notes))

-- a statement that writes cannot be watched
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.run_until("insert into c values (30, 'x') returning id", '30')
H.notes = {}
t.watch('1')
H.check('watching a write is refused', H.wait(2000, function() return #H.notes_with('not watched') == 1 end), vim.inspect(H.notes))
H.done()
