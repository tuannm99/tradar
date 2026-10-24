-- `${name}` / `${name=default}` parameters in a statement.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table p(id integer primary key, name text); insert into p values (1,'ann'),(2,'bob')]])
local t = H.setup()
local params = require('tradar.params')

-- ── pure ────────────────────────────────────────────────────────────────
local names = params.names({ "select ${a}, '${b=x}'", '-- ${commented}', 'where ${a} and ${c=1} and ${b}' })
H.check('names: distinct, in order of appearance, comments skipped', #names == 3 and names[1].name == 'a' and names[2].name == 'b' and names[3].name == 'c', vim.inspect(names))
H.check('names: the default comes from whichever placeholder gives one', names[2].default == 'x' and names[3].default == '1' and names[1].default == nil, vim.inspect(names))
H.check('apply: a bare placeholder is the value as typed', params.apply('where id = ${id}', { id = '5 or 1=1' }) == 'where id = 5 or 1=1')
H.check("apply: inside '...' SQL quotes are doubled", params.apply("name = '${n}'", { n = "O'Brien" }) == "name = 'O''Brien'")
H.check('apply: inside "..." for Mongo it is backslash-escaped', params.apply('{name: "${n}"}', { n = 'say "hi"\\' }, 'mongo') == '{name: "say \\"hi\\"\\\\"}')
H.check('apply: a comment line is left as it is', params.apply('-- ${x}\nselect ${x}', { x = '1' }) == '-- ${x}\nselect 1')
H.check('apply: the same placeholder twice', params.apply('${a}+${a}', { a = '2' }) == '2+2')
H.check('apply: a text with none is untouched', params.apply('select $1, {a: 1}', {}) == 'select $1, {a: 1}')

-- ── through the editor ──────────────────────────────────────────────────
H.sql_buffer('params.sql', { '' })
H.check('connects', H.connect())
local asked, answers = {}, {}
vim.ui.input = function(o, cb)
  asked[#asked + 1] = { prompt = o.prompt, default = o.default }
  local a = table.remove(answers, 1)
  cb(a)
end
local function footer() return H.lines()[#H.lines()] or '' end

answers = { '41', "O'Brien" }
H.run("select ${n} + 1 as r, '${s}' as s")
H.check('the statement runs with the values', H.wait(4000, function() return H.text():find("O'Brien", 1, true) ~= nil end), H.text())
H.check('...the number was added', H.text():find('42', 1, true) ~= nil, H.text())
H.check('one question per parameter', #asked == 2 and asked[1].prompt:find('n ', 1, true) and asked[2].prompt:find('s ', 1, true), vim.inspect(asked))

asked, answers = {}, { '7', 'x' }
H.run("select ${n} + 1 as r, '${s}' as s")
H.wait(3000, function() return #asked >= 2 end)
H.check('the previous values are the defaults', asked[1].default == '41' and asked[2].default == "O'Brien", vim.inspect(asked))
H.wait(4000, function() return H.text():find('8', 1, true) ~= nil end)

-- ${name=default}
asked, answers = {}, { '' }
H.sql_buffer('params2.sql', { '' })
vim.ui.input = function(o, cb) asked[#asked + 1] = o.default; cb(table.remove(answers, 1)) end
answers = { 'k' }
H.run("select '${fresh=hello}' as fresh_col")
H.wait(3000, function() return #asked >= 1 end)
H.check('a fresh parameter is offered its written default', asked[1] == 'hello', vim.inspect(asked))
H.check('the typed value is used', H.wait(4000, function() return H.text():find('fresh_col', 1, true) ~= nil end) and H.lines()[3]:find('k', 1, true) ~= nil, H.text())

-- cancel abandons the run
local before = #(require('tradar.ctx').state.result_log or {})
vim.ui.input = function(o, cb) cb(nil) end
H.run('select ${gone} as gone_col')
H.sleep(500)
H.check('cancelling a question runs nothing', #(require('tradar.ctx').state.result_log or {}) == before and not H.text():find('gone_col', 1, true))

-- run all: asked once for the batch
local calls = 0
vim.ui.input = function(o, cb) calls = calls + 1; cb('2') end
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { 'select ${v} as one;', 'select ${v} * 10 as two;' })
t.run(nil, nil, true)
H.check('run all asks once for a parameter used twice', H.wait(5000, function() return H.text():find('two', 1, true) ~= nil end) and calls == 1, calls)
H.check('...and both statements got the value', H.lines()[3]:find('20', 1, true) ~= nil, H.text())

-- earlier values complete with <Tab>
local st = require('tradar.ctx').state
local answers_for_w = { 'one', 'two', 'one' }
local seen_completion
vim.ui.input = function(o, cb) seen_completion = o.completion; cb(table.remove(answers_for_w, 1)) end
for _ = 1, 3 do
  vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { "select '${w}' as w_col" })
  vim.api.nvim_win_set_cursor(0, { 1, 0 })
  t.run()
  H.wait(3000, function() return not st.running end)
  H.sleep(150)
end
st.param_asking = 'w'
H.check('the question offers completion', seen_completion == 'customlist,v:lua.__tradar_param_complete', tostring(seen_completion))
local all = _G.__tradar_param_complete('')
H.check('earlier values, most recent first, no repeats', #all == 2 and all[1] == 'one' and all[2] == 'two', vim.inspect(all))
H.check('completion filters by what is typed', vim.deep_equal(_G.__tradar_param_complete('t'), { 'two' }))

-- the guard looks at what will be sent
local guarded
vim.ui.select = function(items, o, cb) guarded = o.prompt; cb('Cancel') end
vim.ui.input = function(o, cb) cb('') end
vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { 'delete from p ${where}' })
vim.api.nvim_win_set_cursor(0, { 1, 0 })
t.run()
H.check('a placeholder that expands to nothing is caught as DELETE without WHERE', H.wait(3000, function() return guarded ~= nil end), guarded)
H.sleep(300)
H.check('...and cancelling it deleted nothing', H.sql('select count(*) from p')[1][1] == '2')
H.done()
