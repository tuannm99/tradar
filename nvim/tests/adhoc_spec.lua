-- Connections written in place -- a URL, a path or `driver target` in a fence
-- or a modeline -- instead of an alias from connections.toml.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table people(id integer primary key, name text);
insert into people values (1,'ann'),(2,'bob')]])
local t = H.setup()
local ctx = require('tradar.ctx')
local state = ctx.state

-- name mapping
local an = t.adhoc_name
H.check('a URL with the language of a fence', an('http://localhost:9201', 'elasticsearch') == '@elasticsearch:http://localhost:9201')
H.check('esq is Elasticsearch', an('http://h:9200', 'esq') == '@elasticsearch:http://h:9200')
H.check('the scheme gives the driver to ```sql', an('postgres://u:p@h:5432/db', 'sql') == '@postgres:postgres://u:p@h:5432/db' and an('mongodb://h/db', 'mongo') == '@mongo:mongodb://h/db')
H.check('a path is SQLite', an('/tmp/x.db', 'sql') == '@sqlite:/tmp/x.db' and an('./x.db', 'sql') == '@sqlite:./x.db')
H.check('host:port needs a language that names the driver', an('localhost:9042', 'cassandra') == '@cassandra:localhost:9042' and an('localhost:9042', 'sql') == 'localhost:9042')
H.check('`driver target` as a modeline writes it', an('elasticsearch http://localhost:9201') == '@elasticsearch:http://localhost:9201' and an('sqlite /tmp/a b.db') == '@sqlite:/tmp/a b.db')
H.check('an alias is left alone', an('local', 'sql') == 'local' and an('prod-pg', 'postgres') == 'prod-pg' and an(nil) == nil)
H.check('an already ad-hoc name is left alone', an('@sqlite:/a.db', 'sql') == '@sqlite:/a.db')
H.check('a URL with no driver to be had is left alone (clickhouse must be named)', an('http://h:8123', 'sql') == 'http://h:8123' and an('http://h:8123', 'clickhouse') == '@clickhouse:http://h:8123')
H.check('~ is expanded for a file', an('~/x.db', 'sqlite') == '@sqlite:' .. vim.fn.expand('~/x.db'))
H.check('the driver of an ad-hoc name', t.adhoc_driver('@elasticsearch:http://h') == 'elasticsearch' and t.adhoc_driver('@https://api.io') == nil and t.adhoc_driver('local') == nil)
H.check('...is known without asking the server', t.cached_driver('@postgres:postgres://h/db') == 'postgres')
H.check('a password is not shown', t.display_name('@postgres:postgres://ann:s3cret@h:5432/db') == '@postgres:postgres://ann:***@h:5432/db' and t.display_name('@sqlite:/a.db') == '@sqlite:/a.db' and t.display_name('local') == 'local')

-- a .tdb block with the database in its fence
local db = H.db_path
vim.cmd('edit! ' .. H.tmp .. '/direct.tdb')
vim.api.nvim_buf_set_lines(0, 0, -1, false, {
  '```sqlite ' .. db, 'select name from people order by id', '```', '',
  '```sql ' .. db, 'select count(*) as n from peo', '```', '',
  '```sql local', 'select id from people', '```',
})
vim.bo.filetype = 'tdb' -- after the text, as when a file is read: its fences are connected on attach
local name = '@sqlite:' .. db
H.check('the block\'s connection is the ad-hoc name', t.block_connection(0, 2) == name and t.block_connection(0, 6) == name, tostring(t.block_connection(0, 2)))
H.check('...and one with an alias keeps it', t.block_connection(0, 10) == 'local')
H.check('opening the file connected it', H.wait(5000, function() return state.connected[name] end))
H.check('no saved connection has that name', (function()
  local list
  require('tradar.rpc').request('connections.list', nil, function(e, r) list = r or e end)
  H.wait(2000, function() return list ~= nil end)
  for _, c in ipairs(list) do if c.name == name then return false end end
  return true
end)())
vim.api.nvim_win_set_cursor(0, { 2, 0 })
t.run()
H.check('rr runs it', H.wait(5000, function() return H.text():find('ann', 1, true) and H.text():find('bob', 1, true) end), H.text())
H.check('...and the result belongs to that connection', state.result and state.result.conn == name, state.result and state.result.conn)
vim.api.nvim_win_set_cursor(0, { 10, 0 })
t.run()
H.check('a block with an alias still runs', H.wait(5000, function() return H.text():find('2/2 rows', 1, true) ~= nil end), H.text())

-- completion in it
local source = require('tradar.blink').new()
vim.api.nvim_win_set_cursor(0, { 6, 30 })
local items
H.check('the source is on there', source:enabled())
source:get_completions({ bufnr = 0, cursor = { 6, 30 } }, function(r) items = r end)
H.wait(5000, function() return items ~= nil end)
H.check('tables of that database are suggested', items and vim.tbl_contains(vim.tbl_map(function(i) return i.label end, items.items), 'people'), vim.inspect(items))

-- a modeline
vim.cmd('enew')
vim.cmd('edit! ' .. H.tmp .. '/modeline.sql')
vim.bo.filetype = 'sql'
vim.api.nvim_buf_set_lines(0, 0, -1, false, { '-- tradar: sqlite ' .. db, 'select name from people where id = 2;' })
H.check('a modeline with `driver target`', t.connection_for(0) == name, tostring(t.connection_for(0)))
vim.api.nvim_win_set_cursor(0, { 2, 0 })
t.run()
H.check('...runs', H.wait(5000, function() return H.text():find('bob', 1, true) ~= nil end), H.text())
vim.api.nvim_buf_set_lines(0, 0, -1, false, { '-- tradar: ' .. db, 'select name from people where id = 1;' })
H.check('and a bare path in one is SQLite', t.connection_for(0) == name, tostring(t.connection_for(0)))

-- an unknown alias still says so
vim.api.nvim_buf_set_lines(0, 0, -1, false, { '-- tradar: nothere', 'select 1;' })
vim.api.nvim_win_set_cursor(0, { 2, 0 })
H.notes = {}
t.run()
H.check('an alias that does not exist is refused as before', H.wait(5000, function() return #H.notes_with('no saved connection named `nothere`') > 0 end), vim.inspect(H.notes))

-- HTTP: the base URL in the fence
local server = [[
import http.server, sys, json
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        out = json.dumps({'path': self.path}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(out)))
        self.end_headers()
        self.wfile.write(out)
s = http.server.HTTPServer(('127.0.0.1', 0), H)
open(sys.argv[1], 'w').write(str(s.server_port))
s.serve_forever()
]]
vim.fn.writefile(vim.split(server, '\n'), H.tmp .. '/srv.py')
local portfile = H.tmp .. '/port'
local job = vim.fn.jobstart({ 'python3', '-I', H.tmp .. '/srv.py', portfile })
H.check('an HTTP server starts', H.wait(5000, function() return vim.fn.filereadable(portfile) == 1 and #vim.fn.readfile(portfile) > 0 end))
local base = 'http://127.0.0.1:' .. vim.fn.readfile(portfile)[1]
vim.cmd('enew')
vim.cmd('edit! ' .. H.tmp .. '/api.tdb')
vim.bo.filetype = 'tdb'
vim.api.nvim_buf_set_lines(0, 0, -1, false, { '```http ' .. base .. '/v1', 'GET /users', '```' })
vim.api.nvim_win_set_cursor(0, { 2, 0 })
t.run()
H.check('```http with a base URL runs a relative request against it', H.wait(5000, function() return state.result and state.result.status == '200 OK' end), vim.inspect(state.result and state.result.status))
H.check('...at the joined URL', table.concat(state.result.lines, '\n'):find('"path": "/v1/users"', 1, true) ~= nil, table.concat(state.result.lines, '\n'))
vim.fn.setreg('"', '')
require('tradar.restbench').copy(0)
H.check('...and copy as curl uses the same', H.wait(3000, function() return vim.fn.getreg('"'):find(base .. '/v1/users', 1, true) end), vim.fn.getreg('"'))
vim.fn.jobstop(job)
H.done()
