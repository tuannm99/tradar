-- The telescope pickers: tables (+ <C-o> open, <C-g> generate), history, and
-- the query library (:TradarOpen). Telescope is cloned by nvim/tests/deps.sh.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table people(id integer primary key, name text);
create table orders(id integer primary key, person_id integer)]])
local lib = H.tmp .. '/lib'
local t = H.setup({ queries_dir = lib })
if not H.telescope() then return H.done() end
local action_state = require('telescope.actions.state')

local function keys(s) vim.api.nvim_feedkeys(vim.api.nvim_replace_termcodes(s, true, false, true), 'x', false) end

--- Waits for a picker with at least `n` results and returns it.
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

H.sql_buffer('tele.sql', { 'select 1;', '' })
H.check('connects', H.connect())
t.attach(H.sqlbuf)
local sqlwin = vim.fn.bufwinid(H.sqlbuf)

-- ── tables ──────────────────────────────────────────────────────────────
vim.api.nvim_set_current_win(sqlwin)
vim.api.nvim_win_set_cursor(0, { 2, 0 })
vim.cmd('TradarTables')
local p = picker(2)
H.check('the tables picker lists both tables', p and #results(p) == 2, p and #results(p))
local title = p and p.prompt_title or ''
H.check('its title advertises <C-o> and <C-g>', title:find('<C-o>', 1, true) and title:find('<C-g>', 1, true), title)
keys('orders')
H.wait(2000, function() return #results(p) == 1 end)
H.check('typing filters the list', #results(p) == 1)
keys('<CR>')
H.check('<CR> inserts the table name into the SQL buffer', H.wait(3000, function() return buf_text():find('orders', 1, true) ~= nil end), buf_text())

vim.api.nvim_set_current_win(sqlwin)
vim.cmd('TradarTables')
p = picker(2)
keys('people')
H.wait(2000, function() return #results(p) == 1 end)
keys('<C-o>')
H.check('<C-o> puts a SELECT of the table in the buffer', H.wait(3000, function() return buf_text():lower():find('select * from "people"', 1, true) ~= nil end), buf_text())
H.sleep(300)
H.check('... and runs nothing', not H.text():find('rows', 1, true), H.text())
vim.api.nvim_set_current_win(sqlwin)
vim.api.nvim_win_set_cursor(0, { vim.fn.search('select', 'nw') > 0 and vim.fn.search('select', 'nw') or 1, 0 })
H.t.run()
H.check('<leader>rr then runs it: the rows are shown', H.wait(5000, function() return H.text():find('rows', 1, true) ~= nil end), H.text())

vim.api.nvim_set_current_win(sqlwin)
vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { 'select 1;', '' })
vim.api.nvim_win_set_cursor(0, { 1, 0 })
local asked = {}
local answers = { 'Create', 'All columns' }
vim.ui.select = function(items, opts, cb)
  asked[#asked + 1] = opts.prompt
  local want = table.remove(answers, 1)
  for _, it in ipairs(items) do
    if (opts.format_item or tostring)(it):find(want, 1, true) then return cb(it) end
  end
end
vim.cmd('TradarTables')
p = picker(2)
keys('people')
H.wait(2000, function() return #results(p) == 1 end)
keys('<C-g>')
H.check('<C-g> generates a statement for the picked table', H.wait(5000, function() return buf_text():find('INSERT INTO "people"', 1, true) ~= nil end), buf_text())
H.check('... after asking only operation and columns', #asked == 2, vim.inspect(asked))

-- ── history ─────────────────────────────────────────────────────────────
vim.api.nvim_set_current_win(sqlwin)
vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { '' })
-- (waits for a column name only this result has: an earlier result's footer would end the wait at once)
H.check('history gets a query', H.run_until('select name as hist_marker from people', 'hist_marker'))
vim.api.nvim_set_current_win(sqlwin)
vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, { 'first', 'last' })
vim.api.nvim_win_set_cursor(0, { 1, 0 })
vim.cmd('TradarHistory')
p = picker(1)
H.check('the history picker lists it', p and #results(p) >= 1)
keys('hist_marker')
H.wait(2000, function() return #results(p) == 1 end)
keys('<CR>')
H.check('<CR> pastes it below the cursor', H.wait(3000, function() return buf_text():find('first\nselect name as hist_marker from people\nlast', 1, true) ~= nil end), buf_text())

-- ── query library ───────────────────────────────────────────────────────
vim.fn.mkdir(lib .. '/reports', 'p')
for name, text in pairs({ ['daily.sql'] = 'select 1;', ['reports/weekly.sql'] = 'select 2;' }) do
  local f = io.open(lib .. '/' .. name, 'w') f:write(text .. '\n') f:close()
end
vim.cmd('enew')
vim.ui.select = nil -- telescope, not the fallback
package.loaded['telescope.pickers'] = nil
vim.cmd('TradarOpen')
p = picker(2)
local names = {}
for _, e in ipairs(p and results(p) or {}) do names[#names + 1] = e.display end
table.sort(names)
H.check(':TradarOpen lists the library, nested files included', vim.deep_equal(names, { 'daily.sql', 'reports/weekly.sql' }), vim.inspect(names))
H.check('it has a file previewer', p and p.previewer ~= nil)
keys('weekly')
H.wait(2000, function() return #results(p) == 1 end)
keys('<CR>')
H.check('<CR> opens the file', H.wait(3000, function() return vim.api.nvim_buf_get_name(0) == lib .. '/reports/weekly.sql' end), vim.api.nvim_buf_get_name(0))

vim.cmd('enew')
local before = #vim.api.nvim_list_wins() -- before the picker's own floating windows
vim.cmd('TradarOpen')
p = picker(2)
keys('daily')
H.wait(2000, function() return #results(p) == 1 end)
keys('<C-s>')
H.check('<C-s> opens the file in a split', H.wait(3000, function() return #vim.api.nvim_list_wins() == before + 1 and vim.api.nvim_buf_get_name(0) == lib .. '/daily.sql' end), vim.api.nvim_buf_get_name(0))

vim.cmd('enew')
vim.cmd('TradarOpen')
p = picker(2)
keys('daily')
H.wait(2000, function() return #results(p) == 1 end)
vim.fn.confirm = function() return 1 end
keys('<C-x>')
H.check('<C-x> deletes the file after confirming', H.wait(3000, function() return vim.fn.filereadable(lib .. '/daily.sql') == 0 end))
H.check('... and leaves the others', vim.fn.filereadable(lib .. '/reports/weekly.sql') == 1)

-- ── connections ─────────────────────────────────────────────────────────
local tele = require('tradar.telescope')
H.check('a password in a URL is hidden', tele.mask_target('postgres://ann:s3cret@db:5432/app') == 'postgres://ann:***@db:5432/app', tele.mask_target('postgres://ann:s3cret@db:5432/app'))
H.check('...and password= in a DSN', tele.mask_target('host=db user=ann password=s3cret dbname=app') == 'host=db user=ann password=*** dbname=app')
H.check('a target without one is untouched', tele.mask_target('/tmp/x.db') == '/tmp/x.db' and tele.mask_target('http://h:80/p') == 'http://h:80/p')

H.sql_buffer('conn.sql', { '' })
vim.cmd('TradarConnect')
p = picker(1)
H.check(':TradarConnect opens a telescope picker', p ~= nil and (p.prompt_title or ''):find('connections', 1, true) ~= nil, p and p.prompt_title)
H.check('...listing the saved connection', p and #results(p) == 1 and results(p)[1].value.name == 'local')
H.check('...marking it connected', p and results(p)[1].display:find('●', 1, true) ~= nil, p and results(p)[1].display)
keys('<CR>')
H.check('<CR> binds the buffer to it', H.wait(3000, function() return vim.b[H.sqlbuf].tradar_connection == 'local' end), tostring(vim.b[H.sqlbuf].tradar_connection))

-- <C-d> closes the connection on the server
vim.cmd('TradarConnect')
p = picker(1)
local asked_confirm
vim.fn.confirm = function(msg) asked_confirm = msg; return 1 end
H.notes = {}
keys('<C-d>')
H.check('<C-d> asks before closing', H.wait(3000, function() return asked_confirm ~= nil end) and asked_confirm:find('Disconnect `local`', 1, true) ~= nil, tostring(asked_confirm))
H.check('...then the connection is closed here', H.wait(3000, function() return not t.is_connected('local') end))
H.check('...and the server really closed it', (function()
  local list
  require('tradar.rpc').request('connections.list', nil, function(e, r) list = r end)
  H.wait(3000, function() return list ~= nil end)
  return list and list[1].connected == false
end)())
H.check('...and says so', #H.notes_with('disconnected `local`') == 1, vim.inspect(H.notes))
vim.cmd('TradarConnect')
p = picker(1)
H.notes = {}
keys('<C-d>')
H.check('on a connection that is not open it only says so', H.wait(3000, function() return #H.notes_with('is not connected') == 1 end), vim.inspect(H.notes))
keys('<Esc><Esc>')
H.check('it reconnects on the next use', H.connect())

local ctx = require('tradar.ctx')
ctx.opts.connection_picker = 'ui'
local via_ui
vim.ui.select = function(items, o, cb) via_ui = o.prompt; cb(nil) end
t.connect()
H.check("setup{ connection_picker = 'ui' } keeps the plain list", H.wait(3000, function() return via_ui == 'tradar connection' end), tostring(via_ui))
ctx.opts.connection_picker = nil

H.done()
