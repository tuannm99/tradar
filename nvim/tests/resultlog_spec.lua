-- The results of a run-all (and earlier runs) are kept: `[r` / `]r` step
-- through them, `:TradarResults` picks one.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table p(id integer primary key, name text);
insert into p values (1,'a'),(2,'b'),(3,'c'),(4,'d'),(5,'e')]])
local t = H.setup({ page_size = 2, results_kept = 4 })
H.sql_buffer('log.sql', { '' })
H.check('connects', H.connect())
local function footer() return H.lines()[#H.lines()] or '' end
local function wait_footer(needle) return H.wait(5000, function() return footer():find(needle, 1, true) ~= nil end) end
local function in_results() vim.api.nvim_set_current_win(vim.fn.bufwinid(H.rbuf())) end
local function step(d) in_results() t.step_result(d) end

-- run all: three statements, each one's result is kept
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { "select id as first_col from p;", "update p set name = 'z' where id = 1;", 'select name as last_col from p;' })
t.run(nil, nil, true)
H.check('the last statement is shown', wait_footer('[3/3]') and H.text():find('last_col', 1, true), H.text())
H.check('the footer names the query', footer():find('select name as last_col', 1, true), footer())

step(-1)
H.check('[r shows the UPDATE before it', H.text():find('row(s) affected', 1, true) and H.text():find('[2/3]', 1, true), H.text())
step(-1)
H.check('[r again shows the first SELECT', H.text():find('first_col', 1, true) and footer():find('[1/3]', 1, true), H.text())
H.check('...with its rows', H.text():find('│', 1, true) == nil and H.lines()[3]:find('1', 1, true) ~= nil, H.text())
local b = H.rbuf()
H.check('an earlier result is read-only', not vim.bo[b].modifiable)
step(-1)
H.check('before the oldest, it says so', #H.notes_with('oldest kept result') == 1, vim.inspect(H.notes))
step(1); step(1)
H.check(']r goes forward to the newest', footer():find('[3/3]', 1, true) ~= nil and H.text():find('last_col', 1, true) ~= nil, H.text())
step(1)
H.check('past the newest, it says so', #H.notes_with('newest result') == 1)

-- the newest result is still live and pages; an older one is closed
H.check('the newest result still has more pages', footer():find('loads the next page', 1, true) ~= nil, footer())
step(-2)
H.check('an older result with unloaded rows says it was closed', footer():find('was closed when a newer one arrived', 1, true) ~= nil, footer())
H.notes = {}
in_results()
t.more()
H.check('loading more of it is refused, not hung', #H.notes_with('was closed when a newer one arrived') == 1, vim.inspect(H.notes))
local csv = H.tmp .. '/old.csv'
H.notes = {}
t.export('csv', csv)
H.wait(3000, function() return #H.notes_with('wrote') > 0 end)
local note = (H.notes_with('wrote')[1] or {}).msg or ''
H.check('exporting it warns the file is partial', note:find('NOT the whole result', 1, true) ~= nil, note)

-- picking
vim.ui.select = function(items, opts, cb)
  H.check('the picker lists newest first', items[1] == 3 and #items == 3, vim.inspect(items))
  H.check('...and marks the one shown', opts.format_item(1):find('◀', 1, true) ~= nil)
  cb(2)
end
in_results()
vim.cmd('TradarResults')
H.check(':TradarResults jumps to the picked result', footer():find('[2/3]', 1, true) ~= nil or H.text():find('[2/3]', 1, true) ~= nil, H.text())

-- unsaved grid edits block navigation
step(1)
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.check('a single-table SELECT is editable once it is the newest', H.run_until('select id, name from p order by id', '[4/4]'), footer())
H.wait(3000, function() return vim.bo[H.rbuf()].modifiable end)
in_results()
local lines = H.lines()
vim.api.nvim_buf_set_lines(H.rbuf(), 2, 3, false, { (lines[3]:gsub('z', 'Q')) })
H.notes = {}
t.step_result(-1)
H.check('stepping away with unsaved edits is refused', #H.notes_with('unsaved grid edits') == 1 and footer():find('[4/4]', 1, true) ~= nil, vim.inspect(H.notes))
t.revert()

-- results_kept caps the log
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.run_until('select 1 as k5', 'k5')
H.wait(2000, function() return footer():find('[4/4]', 1, true) ~= nil end)
H.check('only results_kept results are kept (oldest dropped)', footer():find('[4/4]', 1, true) ~= nil, footer())
H.done()
