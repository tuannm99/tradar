-- A tab keeps the last runs of its target (`[R` / `]R`); a pinned tab is
-- never taken over by a later run of the same target.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table p(id integer primary key, name text);
insert into p values (1,'a'),(2,'b'),(3,'c');
create table o(id integer primary key);
insert into o values (1)]])
local t = H.setup({ results_history = 2 })
local state = require('tradar.ctx').state
H.sql_buffer('hist.sql', { '' })
H.check('connects', H.connect())
local function log() return state.result_log or {} end
local function footer() return H.lines()[#H.lines()] or '' end
local function in_results() vim.api.nvim_set_current_win(vim.fn.bufwinid(H.rbuf())) end
local function run(sql, col)
  vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
  H.run(sql)
  return H.wait(5000, function() local r = state.result; return r and r.query == sql and H.text():find(col, 1, true) ~= nil end)
end

H.check('first run', run('select id as c1 from p', 'c1'))
H.check('second run of the same target', run('select name as c2 from p', 'c2') and #log() == 1)
H.check('third run', run('select id as c3 from p', 'c3'))
H.check('one tab, two runs kept', #log() == 1 and #log()[1].history == 2, vim.inspect(#(log()[1].history or {})))
H.check('a fourth run drops the oldest', run('select name as c4 from p', 'c4') and #log()[1].history == 2)

in_results()
t.step_history(1)
H.check('[R shows the run before', H.text():find('c3', 1, true) and footer():find('run 1 back of 2', 1, true), footer())
H.check('...read-only', not vim.bo[H.rbuf()].modifiable)
t.step_history(1)
H.check('[R again shows the one before that', H.text():find('c2', 1, true) and footer():find('run 2 back of 2', 1, true), footer())
H.notes = {}
t.step_history(1)
H.check('past the oldest, it says so', #H.notes_with('oldest kept run') == 1, vim.inspect(H.notes))
t.step_history(-1); t.step_history(-1)
H.check(']R comes back to the newest', H.text():find('c4', 1, true) and footer():find('back of', 1, true) == nil, footer())
H.notes = {}
t.step_history(-1)
H.check('past the newest, it says so', #H.notes_with('newest run') == 1)

-- stepping off the tab and back shows the newest run
t.step_history(1)
H.check('another target, another tab', run('select id as k from o', 'k') and #log() == 2)
t.goto_result(1)
H.check('coming back to a tab shows its newest run', H.text():find('c4', 1, true) ~= nil and footer():find('back of', 1, true) == nil, footer())

-- pin
in_results()
t.pin_result()
H.check('the tab is pinned', log()[1].pinned == true)
H.check('the tab bar shows it', vim.wo[vim.fn.bufwinid(H.rbuf())].winbar:find('📌', 1, true) ~= nil)
H.check('a new run of a pinned target opens its own tab', run('select id as c5 from p', 'c5') and #log() == 3 and log()[1].pinned and not log()[3].pinned, #log())
H.check('the pinned tab still holds the old result', table.concat(vim.api.nvim_buf_get_lines(log()[1].buf, 0, -1, false), '\n'):find('c4', 1, true) ~= nil)
H.check('the next run replaces the unpinned one, not the pinned', run('select name as c6 from p', 'c6') and #log() == 3 and log()[1].query:find('c4', 1, true) ~= nil)
in_results()
t.pin_result() -- on the unpinned one: pins it
t.pin_result()
H.check('pinning toggles', not log()[state.result_pos].pinned)
H.done()
