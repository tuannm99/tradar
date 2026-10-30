-- :TradarCheatsheet -- the driver's ready-made queries, from the server.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table people(id integer primary key, name text);
insert into people values (1,'ann'),(2,'bob')]])
local t = H.setup({})
local ctx = require('tradar.ctx')
if not H.telescope() then return H.done() end
local action_state = require('telescope.actions.state')

local function keys(s) vim.api.nvim_feedkeys(vim.api.nvim_replace_termcodes(s, true, false, true), 'x', false) end
local function picker(n)
  local found
  H.wait(5000, function()
    local p = action_state.get_current_picker(vim.api.nvim_get_current_buf())
    if p and p.manager and p.manager:num_results() >= n then found = p return true end
  end)
  return found
end
local function results(p)
  local out = {}
  for e in p.manager:iter() do out[#out + 1] = e end
  return out
end
local function buf_text() return table.concat(vim.api.nvim_buf_get_lines(H.sqlbuf, 0, -1, false), '\n') end

H.sql_buffer('cheat.sql', { 'select 1;', '' })
H.check('connects', H.connect())
t.attach(H.sqlbuf)

-- what the server sends
local answer
require('tradar.rpc').request('cheatsheet', { connection = 'local' }, function(e, r) answer = r or e end)
H.wait(3000, function() return answer ~= nil end)
local entries = type(answer) == 'table' and answer.entries or {}
H.check('the server sends the SQLite cheat sheet', #entries >= 10 and entries[1].category and entries[1].title and entries[1].text, vim.inspect(answer))

-- the picker
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
vim.api.nvim_win_set_cursor(0, { 1, 0 })
vim.cmd('TradarCheatsheet')
local p = picker(10)
H.check('the picker lists every entry', p and #results(p) == #entries, p and #results(p))
H.check('...with a title that says what the keys do', (p.prompt_title or ''):find('<CR>', 1, true) and (p.prompt_title or ''):find('<C-r>', 1, true), p.prompt_title)
keys('integrity')
H.wait(2000, function() return #results(p) < #entries end)
H.check('typing filters, best match first', #results(p) < #entries and results(p)[1].value.title == 'Integrity check', vim.inspect(vim.tbl_map(function(e) return e.value.title end, results(p))))
keys('<CR>')
H.check('<CR> puts the query in the buffer', H.wait(3000, function() return buf_text():find('PRAGMA integrity_check;', 1, true) ~= nil end), buf_text())
H.sleep(300)
H.check('...without running it', H.rbuf() == -1)
H.check('the cursor is on the query', vim.api.nvim_get_current_line() == 'PRAGMA integrity_check;', vim.api.nvim_get_current_line())
H.t.run()
H.check('<leader>rr then runs it', H.wait(5000, function() return H.text():find('ok', 1, true) ~= nil end), H.text())

-- typing matches the query text too
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
vim.cmd('TradarCheatsheet')
p = picker(10)
keys('sqlite_master')
H.wait(2000, function() return #results(p) < #entries end)
local titles = vim.tbl_map(function(e) return e.value.title end, results(p))
H.check('typing matches the query text as well', vim.tbl_contains(titles, 'Tables and views'), vim.inspect(titles))
keys('<Esc><Esc>')

-- <C-r> inserts and runs; a placeholder is asked for
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { 'select 1;', '' })
vim.api.nvim_win_set_cursor(0, { 1, 0 })
local asked
vim.ui.input = function(o, cb) asked = o.prompt; cb('people') end
vim.cmd('TradarCheatsheet')
p = picker(10)
keys('First rows')
H.wait(2000, function() return #results(p) < #entries end)
H.check('the best match is picked', results(p)[1].value.title == 'First rows of a table', results(p)[1].value.title)
keys('<C-r>')
H.check('<C-r> runs it and asks for the ${table}', H.wait(5000, function() return asked ~= nil and asked:find('table', 1, true) ~= nil end), tostring(asked))
H.check('...the rows of that table are shown', H.wait(5000, function() return H.text():find('ann', 1, true) ~= nil end), H.text())
H.check('the buffer keeps ${table} for the next time', buf_text():find('${table}', 1, true) ~= nil, buf_text())

-- your own queries (cheatsheet.toml beside connections.toml)
local cfg = os.getenv('XDG_CONFIG_HOME') .. '/tradar/cheatsheet.toml'
H.check(':TradarCheatsheetEdit starts from a commented template', (function()
  vim.cmd('TradarCheatsheetEdit')
  H.wait(3000, function() return vim.api.nvim_buf_get_name(0):find('cheatsheet.toml', 1, true) ~= nil end)
  local text = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), '\n')
  return text:find('[[entry]]', 1, true) ~= nil and vim.bo.filetype == 'toml'
end)())
local f = io.open(cfg, 'w')
f:write('[[entry]]\ncategory = "Orders"\ntitle = "My people query"\ntext = "select name from people order by id"\n\n[[entry]]\ntitle = "broken"\n')
f:close()
vim.api.nvim_set_current_buf(H.sqlbuf)
vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { '' })
vim.api.nvim_win_set_cursor(0, { 1, 0 })
H.notes = {}
vim.cmd('TradarCheatsheet')
p = picker(10)
H.check('your entry is listed first, marked', p and results(p)[1].value.title == 'My people query' and results(p)[1].display:find('★', 1, true) ~= nil, p and results(p)[1].display)
H.check('a broken entry is reported, not fatal', #H.notes_with('entry 2 needs a `title` and a `text`') == 1, vim.inspect(H.notes))
keys('<CR>')
H.check('picking it puts your query in the buffer', H.wait(3000, function() return buf_text():find('select name from people order by id', 1, true) ~= nil end), buf_text())
os.remove(cfg)
vim.api.nvim_set_current_buf(H.sqlbuf)

-- no telescope: a plain list
ctx.opts.cheatsheet_picker = 'ui'
local listed
vim.ui.select = function(items, o, cb) listed = vim.tbl_map(o.format_item, items); cb(items[1]) end
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { '' })
vim.api.nvim_win_set_cursor(0, { 1, 0 })
vim.cmd('TradarCheatsheet')
H.check("setup{ cheatsheet_picker = 'ui' } lists category and title", H.wait(3000, function() return listed ~= nil end) and listed[1]:find(entries[1].category, 1, true) and listed[1]:find(entries[1].title, 1, true), vim.inspect(listed and listed[1]))
H.check('...and inserts the choice', H.wait(2000, function() return buf_text():find(entries[1].text:sub(1, 20), 1, true) ~= nil end), buf_text())
ctx.opts.cheatsheet_picker = nil

-- the whole sheet in one buffer
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { '' })
vim.api.nvim_win_set_cursor(0, { 1, 0 })
local tabs_before = #vim.api.nvim_list_tabpages()
vim.cmd('TradarCheatsheetFile')
H.check('the overview opens in a tab of its own', H.wait(5000, function() return #vim.api.nvim_list_tabpages() == tabs_before + 1 end))
local sheet = vim.api.nvim_get_current_buf()
local text = table.concat(vim.api.nvim_buf_get_lines(sheet, 0, -1, false), '\n')
H.check('...named after the connection, as a .tdb', vim.api.nvim_buf_get_name(sheet):find('tradar://cheatsheet/local', 1, true) and vim.bo[sheet].filetype == 'tdb', vim.api.nvim_buf_get_name(sheet))
H.check('every entry is there under its category', (function()
  for _, e in ipairs(entries) do
    if not text:find('### ' .. e.title, 1, true) or not text:find('## ' .. e.category, 1, true) then return false, e.title end
  end
  return true
end)(), text:sub(1, 300))
H.check('each entry is a fenced block on the connection', text:find('### Integrity check\n```sql local\nPRAGMA integrity_check;\n```', 1, true) ~= nil, text:sub(1, 500))
H.check('categories and entries fold', vim.fn.foldlevel(1) == 0 and (function()
  local win = vim.api.nvim_get_current_win()
  vim.api.nvim_win_set_cursor(win, { vim.fn.search('^## ', 'nw'), 0 })
  return vim.fn.foldlevel(vim.fn.line('.')) == 1
end)())
vim.cmd('normal! zM')
H.check('zM shows just the headings', vim.fn.foldclosed(vim.fn.search('^## ', 'nw')) ~= -1)
vim.cmd('normal! zR')

-- a block runs from the overview
vim.api.nvim_win_set_cursor(0, { vim.fn.search('^PRAGMA integrity_check;', 'nw'), 0 })
H.t.run()
H.check('rr on a block runs it', H.wait(5000, function() return H.text():find('ok', 1, true) ~= nil end), H.text())
local before = #vim.api.nvim_list_tabpages()
vim.cmd('TradarCheatsheetFile')
H.check('asking again goes back to the same buffer', H.wait(2000, function() return vim.api.nvim_get_current_buf() == sheet end) and #vim.api.nvim_list_tabpages() == before)

-- no connection
vim.cmd('enew')
H.notes = {}
vim.b.tradar_connection = nil
ctx.state.default = nil
vim.cmd('TradarCheatsheet')
H.check('without a connection it says so', #H.notes_with('no connection for this buffer') == 1, vim.inspect(H.notes))
H.done()
