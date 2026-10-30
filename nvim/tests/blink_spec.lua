-- The blink.cmp source (tradar.blink), driven directly -- no blink.cmp needed:
-- completion in a .tdb block whichever connection its fence names, and in
-- HTTP / gRPC requests.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table people(id integer primary key, name text);
insert into people values (1,'ann')]])
-- a second database nothing opens until a fence names it
local other = H.tmp .. '/other.db'
vim.fn.system({ 'python3', '-I', '-c', "import sqlite3,sys; c=sqlite3.connect(sys.argv[1]); c.execute('create table gadgets(id integer primary key)'); c.commit()", other })
local f = io.open(os.getenv('XDG_CONFIG_HOME') .. '/tradar/connections.toml', 'a')
f:write(('\n[[connections]]\nname = "other"\ndriver = "sqlite"\ntarget = "%s"\n'):format(other))
f:write('\n[[connections]]\nname = "web"\ndriver = "http"\ntarget = "http://127.0.0.1:1"\n')
f:write('\n[[connections]]\nname = "rpc"\ndriver = "grpc"\ntarget = "127.0.0.1:1"\n')
f:close()
H.setup()
local ctx = require('tradar.ctx')
local source = require('tradar.blink').new()

--- Asks the source about the end of line `row` of the current buffer.
local function ask(row, col)
  local line = vim.api.nvim_buf_get_lines(0, row - 1, row, false)[1]
  vim.api.nvim_win_set_cursor(0, { row, col or #line })
  if not source:enabled() then return nil, false end
  local out
  source:get_completions({ bufnr = 0, cursor = { row, col or #line } }, function(r) out = r end)
  H.wait(5000, function() return out ~= nil end)
  return out, true
end
local function labels(r) return vim.tbl_map(function(i) return i.label end, r and r.items or {}) end

-- ── .tdb: SQL blocks ─────────────────────────────────────────────────────
vim.cmd('edit! ' .. H.tmp .. '/c.tdb')
vim.bo.filetype = 'tdb'
vim.api.nvim_buf_set_lines(0, 0, -1, false, { '# notes', '', '```sql local', 'select * from peo', '```', '', '```sql other', 'select * from gad', '```' })
H.check('a block whose fence names a connection nobody opened', not ctx.state.connected.other)
local r, enabled = ask(4)
H.check('the source is on in a .tdb block that names a connection', enabled)
H.check('a table is suggested', vim.tbl_contains(labels(r), 'people'), vim.inspect(labels(r)))
r = ask(8)
H.check('...and one from the other block\'s connection, opened by the completion itself', vim.tbl_contains(labels(r), 'gadgets'), vim.inspect(labels(r)))

-- opening a .tdb opens the connections its fences name
vim.cmd('enew')
vim.cmd('edit! ' .. H.tmp .. '/pre.tdb')
ctx.state.connected.other = nil
require('tradar').disconnect('other', function() end)
H.wait(2000, function() return not ctx.state.connected.other end)
vim.bo.filetype = ''
vim.api.nvim_buf_set_lines(0, 0, -1, false, { '```sql other', 'select 1', '```' })
vim.bo.filetype = 'tdb'
H.check('attaching a .tdb opens the connections of its fences', H.wait(5000, function() return ctx.state.connected.other end))

-- ── HTTP requests ────────────────────────────────────────────────────────
vim.cmd('enew')
vim.cmd('edit! ' .. H.tmp .. '/req.tdb')
vim.bo.filetype = 'tdb'
vim.api.nvim_buf_set_lines(0, 0, -1, false, {
  '@who = ann', '',
  '```http web', 'PO', '```', '',
  '```http web', 'GET /x', 'Content-T', '```', '',
  '```http web', 'GET /x', 'Content-Type: app', '```', '',
  '```http web', 'GET /x', 'X: {{w', '```', '',
})
r, enabled = ask(4)
H.check('the source is on in an ```http block', enabled)
H.check('an HTTP verb is suggested', vim.deep_equal(labels(r), { 'POST' }), vim.inspect(labels(r)))
H.check('...replacing what was typed (range from the start of the word)', r.items[1].textEdit.range.start.character == 0 and r.items[1].textEdit.range['end'].character == 2 and r.items[1].textEdit.newText == 'POST', vim.inspect(r.items[1]))
r = ask(9)
H.check('a header name', vim.tbl_contains(labels(r), 'Content-Type'), vim.inspect(labels(r)))
r = ask(14)
H.check('a header value replaces from after the colon', vim.tbl_contains(labels(r), 'application/json') and r.items[1].textEdit.range.start.character == #'Content-Type: ', vim.inspect(r))
r = ask(19)
H.check('a {{variable}} (the file\'s own)', vim.tbl_contains(labels(r), 'who') and r.items[1].textEdit.range.start.character == #'X: {{', vim.inspect(r))
local sql_r, sql_on = (function()
  vim.api.nvim_buf_set_lines(0, 20, 20, false, { '```sql local', 'select * from peo', '```' })
  return ask(22)
end)()
H.check('...and a ```sql block in the same file is still completed as SQL', sql_on and vim.tbl_contains(labels(sql_r), 'people'), vim.inspect(labels(sql_r)))

-- a .http file
vim.cmd('enew')
vim.cmd('edit! ' .. H.tmp .. '/api.http')
vim.api.nvim_buf_set_lines(0, 0, -1, false, { '# tradar: web', '', 'GET /x', 'Acc' })
r, enabled = ask(4)
H.check('a .http file is completed too', enabled and vim.tbl_contains(labels(r), 'Accept'), vim.inspect(labels(r)))

-- the request workbench
vim.cmd('TradarHttpSend web')
H.check('a workbench opens', H.wait(5000, function() return vim.b.tradar_rest ~= nil end))
vim.api.nvim_buf_set_lines(0, 0, -1, false, { 'GET /x', 'Acc' })
r, enabled = ask(2)
H.check('...and is completed', enabled and vim.tbl_contains(labels(r), 'Accept'), vim.inspect(labels(r)))

-- ── gRPC: services come from the server, without blocking the typing ─────
local rpc = require('tradar.rpc')
local real_request = rpc.request
rpc.request = function(method, params, cb)
  if method == 'grpc.services' then return vim.defer_fn(function() cb(nil, { 'pkg.Svc', 'pkg.Other', 'zzz.Api' }) end, 30) end
  if method == 'grpc.methods' then return vim.defer_fn(function() cb(nil, { { name = 'Get' }, { name = 'Watch' } }) end, 30) end
  return real_request(method, params, cb)
end
vim.cmd('enew')
vim.cmd('edit! ' .. H.tmp .. '/rpc.tdb')
vim.bo.filetype = 'tdb'
vim.api.nvim_buf_set_lines(0, 0, -1, false, { '```grpc rpc', 'GRPC pk', '```' })
r = ask(2)
H.check('a service list not fetched yet: nothing now, and the source asks to be asked again', #labels(r) == 0 and r.is_incomplete_forward == true, vim.inspect(r))
H.sleep(150)
r = ask(2)
H.check('then the services that match', vim.deep_equal(labels(r), { 'pkg.Svc', 'pkg.Other' }) and r.is_incomplete_forward == false, vim.inspect(r))
vim.api.nvim_buf_set_lines(0, 1, 2, false, { 'GRPC pkg.Svc/W' })
ask(2)
H.sleep(150)
r = ask(2)
H.check('after the slash, the service\'s methods', vim.deep_equal(labels(r), { 'pkg.Svc/Watch' }), vim.inspect(r))
rpc.request = real_request
H.done()
