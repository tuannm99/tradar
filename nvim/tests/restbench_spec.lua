-- The HTTP / gRPC request workbench: blocks run from one buffer, the responses
-- land in the result tabs (history, diff, headers, rerun), and gRPC blocks are
-- added from a picked method and run unary / as a stream (rpc stubbed: there
-- is no gRPC server here).
local H = dofile('nvim/tests/helpers.lua')

local server = [[
import http.server, json, sys
n = [0]
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def go(self):
        size = int(self.headers.get('Content-Length') or 0)
        body = self.rfile.read(size).decode()
        status, ctype = 200, 'application/json'
        if self.path.startswith('/text'):
            out, ctype = b'plain text', 'text/plain'
        elif self.path.startswith('/count'):
            n[0] += 1
            out = json.dumps({'n': n[0]}).encode()
        elif self.path.startswith('/missing'):
            status, out = 404, b'{"error":"gone"}'
        else:
            out = json.dumps({'method': self.command, 'path': self.path, 'auth': self.headers.get('Authorization'), 'body': body}).encode()
        self.send_response(status)
        self.send_header('Content-Type', ctype)
        self.send_header('X-Spec', 'yes')
        self.send_header('Content-Length', str(len(out)))
        self.end_headers()
        self.wfile.write(out)
    do_GET = do_POST = go
s = http.server.HTTPServer(('127.0.0.1', 0), H)
open(sys.argv[1], 'w').write(str(s.server_port))
s.serve_forever()
]]
local script = H.tmp .. '/server.py'
local portfile = H.tmp .. '/port'
vim.fn.writefile(vim.split(server, '\n'), script)
local job = vim.fn.jobstart({ 'python3', '-I', script, portfile })
H.check('the test HTTP server starts', H.wait(5000, function() return vim.fn.filereadable(portfile) == 1 and #vim.fn.readfile(portfile) > 0 end))
local port = vim.fn.readfile(portfile)[1]

local f = io.open(os.getenv('XDG_CONFIG_HOME') .. '/tradar/connections.toml', 'a')
f:write(('\n[[connections]]\nname = "web"\ndriver = "http"\ntarget = "http://127.0.0.1:%s"\n'):format(port))
f:write('\n[[connections]]\nname = "rpc"\ndriver = "grpc"\ntarget = "127.0.0.1:1"\n')
f:close()

local t = H.setup()
local ctx = require('tradar.ctx')
local state = ctx.state
local rb = require('tradar.restbench')

local function set_lines(buf, text) vim.api.nvim_buf_set_lines(buf, 0, -1, false, vim.split(text, '\n', { plain = true })) end
local function row_of(buf, pattern)
  for i, l in ipairs(vim.api.nvim_buf_get_lines(buf, 0, -1, false)) do
    if l:match(pattern) then return i end
  end
end
local function run_at(buf, pattern)
  vim.api.nvim_win_set_cursor(vim.fn.bufwinid(buf), { row_of(buf, pattern), 0 })
  rb.run(buf)
end
local function shown(label)
  return H.wait(5000, function() return state.result and state.result.label == label end)
end

vim.cmd('TradarHttpSend web')
H.check(':TradarHttpSend opens the workbench of the connection', H.wait(5000, function() return vim.b.tradar_rest ~= nil end))
local buf = vim.api.nvim_get_current_buf()
H.check('the workbench is bound to its connection', vim.b[buf].tradar_rest.connection == 'web' and vim.b[buf].tradar_rest.driver == 'http')
H.check('it starts with an example request', row_of(buf, '^GET /$') ~= nil)
H.check('<leader>rr, rx, ry are mapped in it', H.mapped(buf, '<leader>rr') and H.mapped(buf, '<leader>rx') and H.mapped(buf, '<leader>ry'))

set_lines(buf, table.concat({
  '@token = abc',
  '',
  '### echo',
  'POST /echo?x=1',
  'Authorization: Bearer {{token}}',
  'Content-Type: application/json',
  '',
  '{"b":2,"a":1}',
  '',
  '### count',
  'GET /count',
  '',
  '### text',
  'GET /text',
  '',
  '### gone',
  'GET /missing',
  '',
  '### wrong',
  'GRPC a.B/C',
  '',
  '### undefined',
  'GET /{{nope}}',
}, '\n'))

-- a JSON response
run_at(buf, '^POST /echo')
H.check('a response opens in a result tab named after method and path', shown('POST /echo'))
local r = state.result
H.check('it is a text result with the status and a tab badge', r.kind == 'text' and r.status == '200 OK' and r.badge == '200', vim.inspect(r.status))
local body = table.concat(r.lines, '\n')
H.check('the JSON body is re-indented in the server\'s order', body:find('{\n  "method": "POST",\n  "path": "/echo?x=1",', 1, true) ~= nil, body)
H.check('variables were expanded into the header, the body went through', body:find('Bearer abc', 1, true) ~= nil and body:find('\\"b\\":2', 1, true) ~= nil, body)
H.check('the body is coloured as JSON', r.json == true and #vim.api.nvim_buf_get_extmarks(state.results_buf, require('tradar.jsonhl').ns, 0, -1, {}) > 0)
local footer = H.lines()[#H.lines()]
H.check('the footer says status, time and size', footer:find('200 OK', 1, true) and footer:find('ms', 1, true) and footer:find('bytes', 1, true), footer)
H.check('the headers are hidden until asked for', not H.text():find('x-spec', 1, true))
require('tradar').toggle_head()
H.check('<leader>rH shows the status line and headers', H.text():find('POST 200 OK', 1, true) and H.text():find('x-spec: yes', 1, true), H.text())
require('tradar').toggle_head()
H.check('and hides them again', not H.text():find('x-spec', 1, true))

-- history, diff, rerun
run_at(buf, '^GET /count')
H.check('another path gets another tab', shown('GET /count') and #state.result_log == 2)
local first = state.result.lines[2]
run_at(buf, '^GET /count')
H.check('the same request again takes its tab and keeps the earlier run', H.wait(5000, function() return state.result.lines[2] ~= first end) and #state.result_log == 2 and #state.result.history == 1, vim.inspect(state.result.lines))
H.check('the tab bar shows the status', require('tradar.slots').bar(state.result_log, state.result_pos):find('200', 1, true) ~= nil)
require('tradar').diff_result()
H.check('<leader>rD diffs it against the previous run', H.wait(3000, function()
  local d = state.diff_buf and vim.api.nvim_buf_is_valid(state.diff_buf) and table.concat(vim.api.nvim_buf_get_lines(state.diff_buf, 0, -1, false), '\n') or ''
  return d:find('-  "n": 1', 1, true) and d:find('+  "n": 2', 1, true)
end), state.diff_buf and table.concat(vim.api.nvim_buf_get_lines(state.diff_buf, 0, -1, false), '\n') or 'no diff buffer')
local before = state.result
require('tradar').refresh()
H.check('<leader>rR sends it again in place, without a new history entry', H.wait(5000, function() return state.result ~= before and state.result.lines[2] ~= before.lines[2] end) and #state.result_log == 2 and #state.result.history == 1, vim.inspect(state.result and state.result.lines))

-- text and errors
run_at(buf, '^GET /text')
H.check('a non-JSON body is shown as it is', shown('GET /text') and state.result.json == false and state.result.lines[1] == 'plain text', vim.inspect(state.result.lines))
run_at(buf, '^GET /missing')
H.check('a 404 is a response, not an error', shown('GET /missing') and state.result.status == '404 Not Found' and state.result.badge == '404')
run_at(buf, '^GRPC a.B/C')
H.check('a GRPC block on an HTTP connection is refused', #H.notes_with('is an HTTP connection') == 1, vim.inspect(H.notes))
run_at(buf, '^GET /{{nope}}')
H.check('an undefined variable is named', #H.notes_with('undefined variable {{nope}}') == 1, vim.inspect(H.notes))

-- copy as curl
vim.api.nvim_win_set_cursor(vim.fn.bufwinid(buf), { row_of(buf, '^POST /echo'), 0 })
rb.copy(buf)
local curl = vim.fn.getreg('"')
H.check('<leader>ry copies the block as curl, with the base url', curl:find('-X POST', 1, true) and curl:find('http://127.0.0.1:' .. port .. '/echo?x=1', 1, true) and curl:find('Authorization: Bearer abc', 1, true), curl)

vim.fn.setreg('"', '')
vim.api.nvim_win_set_cursor(vim.fn.bufwinid(buf), { row_of(buf, '^POST /echo'), 0 })
vim.cmd('TradarCopyCurl')
H.check(':TradarCopyCurl does the same as <leader>ry', vim.fn.getreg('"'):find("-X POST \\\n  'http://127.0.0.1:" .. port .. "/echo?x=1'", 1, true) ~= nil, vim.fn.getreg('"'))

-- filtering a response
run_at(buf, '^POST /echo')
H.check('back on the echo response', shown('POST /echo'))
t.set_filter('.method')
H.check('a path filter shows just that value', table.concat(H.lines(), '\n'):find('^"POST"\n') ~= nil and H.lines()[#H.lines()]:find('filter «.method»: 1 lines', 1, true) ~= nil, H.text())
t.set_filter('.path')
H.check('and another', H.lines()[1] == '"/echo?x=1"', H.text())
H.notes = {}
t.set_filter('.nope')
H.check('a path with nothing in it is refused and the previous filter stays', #H.notes_with('nothing at .nope') == 1 and H.lines()[1] == '"/echo?x=1"', vim.inspect(H.notes))
t.set_filter('auth')
H.check('plain text keeps the matching lines', H.lines()[1]:find('"auth"', 1, true) ~= nil and not H.text():find('"method"', 1, true), H.text())
t.set_filter('off')
H.check('off shows everything again', H.text():find('"method"', 1, true) ~= nil and H.text():find('"auth"', 1, true) ~= nil)
run_at(buf, '^GET /text')
H.check('on a text body', shown('GET /text'))
H.notes = {}
t.set_filter('.a')
H.check('a path on a body that is not JSON says so', #H.notes_with('not JSON') == 1, vim.inspect(H.notes))

-- variables from setup{ request_vars }
require('tradar').rpc_opts().request_vars = { ['*'] = { who = 'everyone' }, web = { tok = 'abc', who = 'web' } }
local rows = vim.api.nvim_buf_line_count(buf)
vim.api.nvim_buf_set_lines(buf, rows, rows, false, { '', '### vars', 'GET /echo', 'X-Who: {{who}}', 'Authorization: Bearer {{tok}}' })
run_at(buf, '^GET /echo$')
H.check('request_vars: the connection\'s variables reach the request', H.wait(5000, function() return state.result and state.result.label == 'GET /echo' end)
  and table.concat(state.result.lines, '\n'):find('Bearer abc', 1, true) ~= nil, vim.inspect(state.result and state.result.lines))
vim.api.nvim_buf_set_lines(buf, rows + 1, rows + 1, false, { '@tok = from-file' })
run_at(buf, '^GET /echo$')
H.check('request_vars: a variable in the file overrides them', H.wait(5000, function() return state.result and table.concat(state.result.lines, '\n'):find('Bearer from-file', 1, true) end), vim.inspect(state.result and state.result.lines))

local real_select = vim.ui.select
local picked
vim.ui.select = function(items, o, cb)
  picked = { items = items, opts = o }
  for _, item in ipairs(items) do
    if item.title == 'Bearer token' then return cb(item) end
  end
  cb(nil)
end

-- the cheat sheet
ctx.opts.cheatsheet_picker = 'ui'
picked = nil
rb.cheatsheet(buf)
text = table.concat(vim.api.nvim_buf_get_lines(buf, 0, -1, false), '\n')
H.check('the cheat sheet offers the HTTP blocks', picked and #picked.items > 10, vim.inspect(picked and #picked.items))
H.check('a pick is added as a titled block', text:find('### Bearer token\n@token = \nGET /me\nAuthorization: Bearer {{token}}', 1, true) ~= nil or text:find('@token = \n### Bearer token', 1, true) ~= nil, text:sub(-200))
vim.api.nvim_win_set_cursor(0, { row_of(buf, '^GET /me$'), 0 })
rb.run(buf)
H.check('and runs like any other (the file\'s @token fills it in)', shown('GET /me') and table.concat(state.result.lines, '\n'):find('Bearer abc', 1, true) ~= nil, vim.inspect(state.result and state.result.lines))

-- the whole sheet in one buffer, for requests
vim.api.nvim_set_current_win(vim.fn.bufwinid(buf))
vim.cmd('TradarCheatsheetFile')
H.check('the request cheat sheet opens as one buffer of http blocks on the connection', H.wait(3000, function()
  return vim.api.nvim_buf_get_name(0):find('tradar://cheatsheet/web', 1, true) ~= nil
end), vim.api.nvim_buf_get_name(0))
local sheet_text = table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), '\n')
H.check('...with a block per entry', sheet_text:find('### Bearer token\n```http web\n@token = \nGET /me', 1, true) ~= nil and sheet_text:find('## Auth', 1, true) ~= nil, sheet_text:sub(1, 400))
vim.cmd('tabclose')
vim.api.nvim_set_current_win(vim.fn.bufwinid(buf))

-- completion
local function complete(line, col, rows_before)
  local b = vim.api.nvim_create_buf(false, true)
  vim.b[b].tradar_rest = vim.b[buf].tradar_rest
  vim.api.nvim_buf_set_lines(b, 0, -1, false, vim.list_extend(vim.deepcopy(rows_before or {}), { line }))
  local start, words = rb.complete_at(b, (rows_before and #rows_before or 0) + 1, col or #line)
  return start, words
end
local start, words = complete('PO')
H.check('completion: an HTTP verb at the start of the request line', start == 0 and vim.deep_equal(words, { 'POST' }), vim.inspect(words))
start, words = complete('Content-T', nil, { 'POST /x' })
H.check('completion: a header name', vim.tbl_contains(words or {}, 'Content-Type') and start == 0, vim.inspect(words))
start, words = complete('Content-Type: app', nil, { 'POST /x' })
H.check('completion: a header value', vim.tbl_contains(words or {}, 'application/json') and start == #'Content-Type: ', vim.inspect({ start, words }))
start, words = complete('', nil, { 'POST /x', 'A: b', '' })
H.check('completion: nothing in the body', words == nil)
start, words = complete('X: {{', nil, { '@tok = 1', 'GET /x' })
H.check('completion: variables inside {{ }}', vim.tbl_contains(words or {}, 'tok') and vim.tbl_contains(words, '$uuid') and start == #'X: {{', vim.inspect({ start, words }))

-- requests outside a workbench: .http files and .tdb blocks
local function open_file(name, lines)
  vim.cmd('edit! ' .. vim.fn.fnameescape(H.tmp .. '/' .. name))
  vim.api.nvim_buf_set_lines(0, 0, -1, false, lines)
  vim.bo.modified = false
  return vim.api.nvim_get_current_buf()
end
local function win_cursor(b, pattern)
  vim.api.nvim_set_current_win(vim.fn.bufwinid(b))
  vim.api.nvim_win_set_cursor(0, { row_of(b, pattern), 0 })
end

local base_url = 'http://127.0.0.1:' .. port
local fbuf = open_file('direct.http', { 'GET ' .. base_url .. '/echo?direct=1', 'X-Who: nobody' })
H.check('a .http file is a request buffer with the plugin\'s keys', vim.bo[fbuf].filetype == 'http' and H.mapped(fbuf, '<leader>rr'))
H.check('a request buffer is not connected as a database', vim.b[fbuf].tradar_attached == true and not H.t.is_connected('web'))
win_cursor(fbuf, '^GET http')
t.run()
H.check('a full URL runs with no saved connection at all', H.wait(5000, function() return state.result and state.result.label == 'GET /echo' and state.result.conn == '@' end), vim.inspect(state.result and state.result.conn))
H.check('...and the response is the real one', table.concat(state.result.lines, '\n'):find('"path": "/echo?direct=1"', 1, true) ~= nil)
vim.api.nvim_buf_set_lines(fbuf, 0, -1, false, { 'GET /echo?relative=1' })
H.notes = {}
win_cursor(fbuf, '^GET /echo')
t.run()
H.check('a relative URL with no connection says how to give one', #H.notes_with('no connection for this relative URL') == 1 and H.notes[1].msg:find('# tradar: <name>', 1, true) ~= nil, vim.inspect(H.notes))
vim.api.nvim_buf_set_lines(fbuf, 0, -1, false, { '# tradar: web', '', 'GET /echo?alias=1' })
win_cursor(fbuf, '^GET /echo')
t.run()
H.check('a `# tradar: name` line gives the buffer its connection, like any other file', H.wait(5000, function() return state.result and state.result.conn == 'web' and table.concat(state.result.lines, '\n'):find('alias=1', 1, true) end), vim.inspect(state.result and state.result.conn))
H.check('...and no database connection is tried for it', #H.notes_with('could not connect') == 0)
vim.fn.setreg('"', '')
t.set_filter('off')
vim.cmd('TradarCopyCurl')
H.check('copying as curl joins the relative URL to the alias\'s base', H.wait(5000, function() return vim.fn.getreg('"'):find(base_url .. '/echo?alias=1', 1, true) end), vim.fn.getreg('"'))

local tbuf = open_file('mixed.tdb', {
  '# notes',
  '@who = from-prose',
  '',
  '```http web',
  'GET /echo?tdb=1',
  'X-Who: {{who}}',
  '```',
  '',
  '```http',
  'GET ' .. base_url .. '/echo?noname=1',
  '```',
  '',
  '```sql local',
  'select 1 as one',
  '```',
})
H.check('a .tdb attaches as before', vim.bo[tbuf].filetype == 'tdb' or vim.api.nvim_buf_get_name(tbuf):match('%.tdb$') ~= nil)
win_cursor(tbuf, '^GET /echo%?tdb')
t.run()
H.check('an ```http block runs on the connection named in its fence', H.wait(5000, function() return state.result and state.result.conn == 'web' and table.concat(state.result.lines, '\n'):find('tdb=1', 1, true) end), vim.inspect(state.result and state.result.conn))
local echoed = H.text()
vim.api.nvim_win_set_cursor(0, { 4, 0 })
t.run()
H.check('...also from its fence line', H.wait(5000, function() return state.result and state.result.at ~= nil and #state.result_log >= 1 end))
win_cursor(tbuf, '^GET http')
t.run()
H.check('an ```http block with no connection runs a full URL straight', H.wait(5000, function() return state.result and state.result.conn == '@' and table.concat(state.result.lines, '\n'):find('noname=1', 1, true) end), vim.inspect(state.result and state.result.conn))
win_cursor(tbuf, '^select 1')
t.run()
H.check('a ```sql block of the same file still runs as a query', H.wait(5000, function() return H.text():find('1/1 rows', 1, true) ~= nil end), H.text())

-- the .tdb block: curl, completion, cheat sheet go to the block under the cursor
win_cursor(tbuf, '^GET /echo%?tdb')
vim.fn.setreg('"', '')
rb.copy(tbuf)
H.check('copying as curl in a .tdb block expands the file\'s variable and joins the base', H.wait(5000, function() return vim.fn.getreg('"'):find("-H 'X-Who: from-prose'", 1, true) and vim.fn.getreg('"'):find(base_url .. '/echo?tdb=1', 1, true) end), vim.fn.getreg('"'))
local cstart, cwords = rb.complete_at(tbuf, row_of(tbuf, '^X%-Who'), 3)
H.check('completion works inside a .tdb request block', cstart == 0 and vim.tbl_contains(cwords or {}, 'X-Request-ID') == false and vim.tbl_contains(cwords or {}, 'X-API-Key') == false and cwords ~= nil, vim.inspect({ cstart, cwords }))
cstart, cwords = rb.complete_at(tbuf, row_of(tbuf, '^select 1'), 3)
H.check('...and nothing is offered in a ```sql block', cstart == nil)
ctx.opts.cheatsheet_picker = 'ui'
local real_select2 = vim.ui.select
vim.ui.select = function(items, o, cb)
  for _, item in ipairs(items) do
    if item.title == 'DELETE' then return cb(item) end
  end
end
vim.api.nvim_set_current_win(vim.fn.bufwinid(tbuf))
vim.api.nvim_win_set_cursor(0, { row_of(tbuf, '^GET /echo%?tdb'), 0 })
rb.cheatsheet(tbuf)
local tl = vim.api.nvim_buf_get_lines(tbuf, 0, -1, false)
local close_at
for i, l in ipairs(tl) do
  if l == '```' and i > row_of(tbuf, '^GET /echo%?tdb') then close_at = i break end
end
H.check('a cheat sheet pick lands inside the block, before its closing fence', tl[close_at - 1] == 'DELETE /items/{{id}}' and tl[close_at - 2] == '### DELETE', vim.inspect(vim.list_slice(tl, 1, 12)))
vim.ui.select = real_select2

-- gRPC (rpc stubbed)
local rpc = require('tradar.rpc')
local real_request, real_on = rpc.request, rpc.on
local listeners, calls = {}, {}
rpc.on = function(name, fn) listeners[name] = fn return function() end end
rpc.request = function(method, params, cb)
  if not method:match('^grpc%.') then return real_request(method, params, cb) end
  calls[#calls + 1] = { method = method, params = params }
  if method == 'grpc.methods' then
    return cb(nil, {
      { name = 'Get', input = 'Req', output = 'Rep', client_streaming = false, server_streaming = false },
      { name = 'Watch', input = 'Req', output = 'Rep', client_streaming = false, server_streaming = true },
    })
  elseif method == 'grpc.skeleton' then
    return cb(nil, { skeleton = '{\n  "id": ""\n}' })
  elseif method == 'grpc.invoke' then
    return cb(nil, { response = { name = 'ann', id = params.request.id }, headers = { ['x-h'] = '1' } })
  elseif method == 'grpc.stream.start' then
    cb(nil, { subscription = 's1' })
    vim.schedule(function()
      listeners['grpc.stream.message']({ subscription = 's1', kind = 'message', message = { n = 1 } })
      listeners['grpc.stream.message']({ subscription = 's1', kind = 'end' })
    end)
    return
  end
  return cb(nil, {})
end

rb.insert_grpc('rpc', 'pkg.Svc', 'Get', { client_streaming = false, server_streaming = false })
H.check('a picked method adds its block, with a skeleton, to the connection\'s workbench', H.wait(5000, function() return vim.b.tradar_rest and vim.b.tradar_rest.connection == 'rpc' and vim.fn.search('^GRPC pkg.Svc/Get$', 'nw') > 0 end), vim.inspect(H.notes))
local gbuf = vim.api.nvim_get_current_buf()
local glines = table.concat(vim.api.nvim_buf_get_lines(gbuf, 0, -1, false), '\n')
H.check('the block has a title, the GRPC line and the JSON skeleton', glines:find('### Get\nGRPC pkg.Svc/Get\n\n{\n  "id": ""\n}', 1, true) ~= nil, glines)
rb.insert_grpc('rpc', 'pkg.Svc', 'Get', {})
H.check('picking it again jumps to the block instead of adding another', H.wait(2000, function() return #H.notes_with('is already there') == 1 end) and select(2, glines:gsub('GRPC pkg.Svc/Get', '')) == 1)

local row = row_of(gbuf, '^  "id"')
vim.api.nvim_buf_set_lines(gbuf, row - 1, row, false, { '  "id": "7"' })
vim.api.nvim_buf_set_lines(gbuf, row - 3, row - 3, false, { 'x-token: {{tok}}', 'deadline: 2s' })
H.notes = {}
run_at(gbuf, '^GRPC pkg.Svc/Get')
H.check('an undefined variable in the metadata stops the call', #H.notes_with('undefined variable {{tok}}') == 1)
local line = row_of(gbuf, '^x%-token')
vim.api.nvim_buf_set_lines(gbuf, line - 1, line, false, { '@tok = secret', 'x-token: {{tok}}' })
-- the variable line sits between the GRPC line and the metadata now: move it above
local gl = vim.api.nvim_buf_get_lines(gbuf, 0, -1, false)
for i, l in ipairs(gl) do if l == '@tok = secret' then table.remove(gl, i) break end end
table.insert(gl, 1, '@tok = secret')
vim.api.nvim_buf_set_lines(gbuf, 0, -1, false, gl)
run_at(gbuf, '^GRPC pkg.Svc/Get')
H.check('a unary call goes out with the body, metadata and deadline', H.wait(5000, function()
  for _, c in ipairs(calls) do
    if c.method == 'grpc.invoke' then return true end
  end
end))
local inv
for _, c in ipairs(calls) do if c.method == 'grpc.invoke' then inv = c.params end end
H.check('request fields', inv and inv.service == 'pkg.Svc' and inv.method == 'Get' and inv.request.id == '7' and inv.metadata['x-token'] == 'secret' and inv.timeout_ms == 2000, vim.inspect(inv))
H.check('the reply opens in a tab named after the method', shown('Get'))
H.check('with the headers kept for <leader>rH', state.result.head[1] == 'OK' and state.result.head[2] == 'x-h: 1', vim.inspect(state.result.head))
H.check('and the body as JSON', table.concat(state.result.lines, '\n'):find('"name": "ann"', 1, true) ~= nil and state.result.json == true)

rb.insert_grpc('rpc', 'pkg.Svc', 'Watch', { server_streaming = true })
H.check('a streaming method says so in its title', H.wait(5000, function() return vim.fn.search([[^### Watch \[server stream\]$]], 'nw') > 0 end))
run_at(gbuf, '^GRPC pkg.Svc/Watch')
H.check('a stream opens a transcript tab', shown('Watch'))
H.check('replies are appended as they arrive and the end is noted', H.wait(5000, function()
  local text = table.concat(state.result.lines, '\n')
  return text:find('‹ {', 1, true) and text:find('# end of stream', 1, true)
end), vim.inspect(state.result and state.result.lines))
H.check('the footer no longer says streaming', H.wait(2000, function() return not H.text():find('streaming', 1, true) end), H.text())

local dbuf = open_file('direct.tdb', {
  '```grpc',
  'GRPC localhost:50051 pkg.Svc/Get',
  '',
  '{"id": "9"}',
  '```',
})
win_cursor(dbuf, '^GRPC localhost')
t.run()
local direct
H.check('GRPC host:port service/Method calls a server with no saved connection', H.wait(5000, function()
  for _, c in ipairs(calls) do
    if c.method == 'grpc.invoke' and c.params.connection == '@localhost:50051' then direct = c.params return true end
  end
end), vim.inspect(calls[#calls]))
H.check('...with the body as written', direct and direct.request.id == '9' and direct.service == 'pkg.Svc')
vim.fn.setreg('"', '')
rb.copy(dbuf)
H.check('...and copies as grpcurl against that address', vim.fn.getreg('"'):find('localhost:50051', 1, true) ~= nil and vim.fn.getreg('"'):find('pkg.Svc/Get', 1, true) ~= nil, vim.fn.getreg('"'))

-- completion on a GRPC line (reflection stubbed)
local real_sync = rpc.request_sync
rpc.request_sync = function(method, params)
  if method == 'grpc.services' then return nil, { 'pkg.Svc', 'pkg.Other', 'zzz.Api' } end
  if method == 'grpc.methods' then return nil, { { name = 'Get' }, { name = 'Watch' } } end
  return real_sync(method, params)
end
local gstart, gwords = (function()
  local b = vim.api.nvim_create_buf(false, true)
  vim.b[b].tradar_rest = vim.b[gbuf].tradar_rest
  vim.api.nvim_buf_set_lines(b, 0, -1, false, { 'GRPC pk' })
  return rb.complete_at(b, 1, 7)
end)()
H.check('completion: GRPC offers the services that match', gstart == 5 and vim.deep_equal(gwords, { 'pkg.Svc', 'pkg.Other' }), vim.inspect({ gstart, gwords }))
gstart, gwords = (function()
  local b = vim.api.nvim_create_buf(false, true)
  vim.b[b].tradar_rest = vim.b[gbuf].tradar_rest
  vim.api.nvim_buf_set_lines(b, 0, -1, false, { 'GRPC pkg.Svc/W' })
  return rb.complete_at(b, 1, 14)
end)()
H.check('completion: after the slash, the service\'s methods', gstart == 5 and vim.deep_equal(gwords, { 'pkg.Svc/Watch' }), vim.inspect({ gstart, gwords }))
rpc.request_sync = real_sync
vim.ui.select = real_select

rpc.request, rpc.on = real_request, real_on
vim.fn.jobstop(job)
H.done()
