-- <leader>rA -- the filter / sort of a result, asked of the database.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table p(id integer primary key, name text, age integer);
insert into p values (1,'ann',31),(2,'bob',25),(3,'cid',40),(4,'dee',null),(5,'eve',9),(6,'fay',52),(7,'gus',33)]])
local t = H.setup({ page_size = 3 })
local sv = require('tradar.serverview')
local state = require('tradar.ctx').state
local cols = { 'id', 'name', 'age' }

-- the statement
local base = 'select id, name, age from p order by id;  '
local sql = sv.build(base, cols, 'age >= 30', { col = 3, desc = true }, 'sqlite')
H.check('wraps the query, filter and sort', sql == 'SELECT * FROM (\nselect id, name, age from p order by id\n) AS tradar_q\nWHERE "age" >= \'30\'\nORDER BY "age" DESC', sql)
H.check('= and != ', (sv.build(base, cols, "name = 'ann'", nil, 'sqlite')):find([[WHERE "name" = 'ann']], 1, true) and (sv.build(base, cols, 'name != ann', nil, 'sqlite')):find([["name" <> 'ann']], 1, true))
H.check('a quote in the value is doubled', (sv.build(base, cols, "name = O'Brien", nil, 'sqlite')):find([['O''Brien']], 1, true))
H.check('contains, escaped, with ESCAPE', (sv.build(base, cols, 'name ~ 50%_', nil, 'sqlite')):find([["name" LIKE '%50!%!_%' ESCAPE '!']], 1, true))
H.check('Postgres: ILIKE (as the grid is case-insensitive)', (sv.build(base, cols, 'name ~ an', nil, 'postgres')):find([["name" ILIKE '%an%']], 1, true))
H.check('MySQL: backticks, backslash doubled', (sv.build(base, cols, [[name = a\b]], nil, 'mysql')):find([[`name` = 'a\\b']], 1, true))
H.check('ClickHouse: backslash escape in LIKE', (sv.build(base, cols, 'name ~ a_b', nil, 'clickhouse')):find([[`name` LIKE '%a\\_b%']], 1, true))
H.check('a sort alone', sv.build(base, cols, nil, { col = 2 }, 'sqlite'):find('ORDER BY "name"\n?$') ~= nil and not sv.build(base, cols, nil, { col = 2 }, 'sqlite'):find('WHERE', 1, true))
H.check('a comment on the last line cannot swallow the wrapper', sv.build('select 1 -- hi', cols, 'id = 1', nil, 'sqlite'):find('-- hi\n)', 1, true) ~= nil)
local none, why = sv.build(base, cols, 'just text', nil, 'sqlite')
H.check('plain text is not a column filter', none == nil and why:find('column = value', 1, true) ~= nil, tostring(why))
local no2, why2 = sv.build(base, cols, 'nope = 1', nil, 'sqlite')
H.check('...nor an unknown column', no2 == nil, tostring(why2))
local no3, why3 = sv.build(base, cols, 'id = 1', nil, 'cassandra')
H.check('Cassandra and the others are refused', no3 == nil and why3:find('SQL results', 1, true) ~= nil, tostring(why3))

-- in the editor
H.sql_buffer('sv.sql', { '' })
H.check('connects', H.connect())
local function in_results() vim.api.nvim_set_current_win(vim.fn.bufwinid(H.rbuf())) end
local function footer() return H.lines()[#H.lines()] or '' end
local function log() return state.result_log or {} end
H.check('runs', H.run_until('select id, name, age from p order by id', '3/7'))
in_results()
H.notes = {}
t.apply_server_view()
H.check('with nothing on screen it says so', #H.notes_with('nothing to apply') == 1, vim.inspect(H.notes))

t.set_filter('age >= 30')
H.check('the grid filter sees the three loaded rows only', footer():find('filter «age >= 30»: 2 of 3 loaded', 1, true) ~= nil, footer())
H.check('...and the footer points at <leader>rA', footer():find('rA asks the server, for all 7 rows', 1, true) ~= nil, footer())
t.apply_server_view()
H.check('the server answers for every row: 4 match', H.wait(5000, function() return footer():find('server filter', 1, true) ~= nil end) and footer():find('3/4 rows', 1, true) ~= nil, footer())
H.check('...in the same tab', #log() == 1, #log())
H.check('...with the unfiltered run in its history', #(log()[1].history or {}) == 1)
H.check('the footer names the server filter', footer():find('server filter «age >= 30»', 1, true) ~= nil, footer())
H.check('the rows are the matching ones (gus 33 is on the second page)', (function() in_results(); t.more(); return H.wait(3000, function() return H.text():find('gus', 1, true) ~= nil end) end)(), H.text())
H.check('...and not the others', not H.text():find('bob', 1, true) and not H.text():find('eve', 1, true))
H.check('a server result is read-only', not vim.bo[H.rbuf()].modifiable)

-- sort on the server
vim.api.nvim_win_set_cursor(0, { 3, 0 })
t.set_filter('off')
vim.cmd(('normal! %d|'):format(state.result.spans[3][1]))
t.toggle_sort()
t.toggle_sort() -- descending
H.check('a descending sort on screen', state.result.sort and state.result.sort.desc == true)
t.apply_server_view()
H.check('asks the server: the oldest comes first', H.wait(5000, function() return footer():find('server sorted by age ↓', 1, true) ~= nil end) and (H.lines()[3] or ''):find('fay', 1, true) ~= nil, H.text())
H.check('...the filter is replaced, not stacked: all 7 rows', footer():find('/7 rows', 1, true) ~= nil and footer():find('server filter', 1, true) == nil, footer())

-- refresh keeps it
local before = state.result
t.refresh()
H.check('refresh repeats the wrapped statement, in the same tab', H.wait(5000, function() return state.result ~= before and not state.running end) and footer():find('server sorted by age ↓', 1, true) ~= nil and #log() == 1, footer())

-- take it off
in_results()
t.apply_server_view()
H.check('<leader>rA with none on screen takes it off', H.wait(5000, function() return footer():find('server', 1, true) == nil and footer():find('/7 rows', 1, true) ~= nil end), footer())
H.check('...back to the statement itself', state.result.query == 'select id, name, age from p order by id' and state.result.server_view == nil, tostring(state.result.query))
H.check('...and still one tab', #log() == 1)

-- not for a statement that writes
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
H.run("insert into p values (8,'hal',1) returning id, name")
H.wait(5000, function() return H.text():find('hal', 1, true) ~= nil end)
in_results()
t.set_filter('id = 8')
H.notes = {}
t.apply_server_view()
H.check('a statement that may change data is not wrapped', H.wait(3000, function() return #H.notes_with('may change data') == 1 end), vim.inspect(H.notes))
H.done()
