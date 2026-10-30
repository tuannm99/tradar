-- A self-signed HTTPS server: refused by default, reachable with insecure=1
-- (on the connection's target, or `@insecure = 1` for a request written
-- straight at a URL).
local H = dofile('nvim/tests/helpers.lua')

local cert, key = H.tmp .. '/c.pem', H.tmp .. '/k.pem'
vim.fn.system({
  'openssl', 'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-keyout', key, '-out', cert, '-days', '1',
  '-subj', '/CN=localhost', '-addext', 'subjectAltName=DNS:localhost,IP:127.0.0.1',
})
H.check('a self-signed certificate is made', vim.v.shell_error == 0 and vim.fn.filereadable(cert) == 1)

local server = [[
import http.server, ssl, sys, json
class H(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a): pass
    def do_GET(self):
        out = json.dumps({'secure': True}).encode()
        self.send_response(200)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(out)))
        self.end_headers()
        self.wfile.write(out)
s = http.server.HTTPServer(('127.0.0.1', 0), H)
ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
ctx.load_cert_chain(sys.argv[2], sys.argv[3])
s.socket = ctx.wrap_socket(s.socket, server_side=True)
open(sys.argv[1], 'w').write(str(s.server_port))
s.serve_forever()
]]
vim.fn.writefile(vim.split(server, '\n'), H.tmp .. '/tls.py')
local portfile = H.tmp .. '/port'
local job = vim.fn.jobstart({ 'python3', '-I', H.tmp .. '/tls.py', portfile, cert, key })
H.check('the HTTPS server starts', H.wait(5000, function() return vim.fn.filereadable(portfile) == 1 and #vim.fn.readfile(portfile) > 0 end))
local port = vim.fn.readfile(portfile)[1]

local f = io.open(os.getenv('XDG_CONFIG_HOME') .. '/tradar/connections.toml', 'a')
f:write(('\n[[connections]]\nname = "strict"\ndriver = "http"\ntarget = "https://127.0.0.1:%s"\n'):format(port))
f:write(('\n[[connections]]\nname = "loose"\ndriver = "http"\ntarget = "https://127.0.0.1:%s?insecure=1"\n'):format(port))
f:close()

local t = H.setup()
local state = require('tradar.ctx').state

local function request_file(name, lines)
  vim.cmd('edit! ' .. vim.fn.fnameescape(H.tmp .. '/' .. name))
  vim.api.nvim_buf_set_lines(0, 0, -1, false, lines)
  vim.bo.modified = false
  vim.api.nvim_win_set_cursor(0, { #lines, 0 })
end

request_file('strict.http', { '# tradar: strict', '', 'GET /' })
H.notes = {}
t.run()
H.check('a self-signed certificate is refused by default', H.wait(5000, function()
  for _, n in ipairs(H.notes) do
    if n.level == vim.log.levels.ERROR then return true end
  end
end), vim.inspect(H.notes))
H.check('...and nothing is shown', state.result == nil)

request_file('loose.http', { '# tradar: loose', '', 'GET /' })
H.notes = {}
t.run()
H.check('target ?insecure=1 reaches it', H.wait(5000, function() return state.result and state.result.status == '200 OK' end), vim.inspect(H.notes))
H.check('...the body is the server\'s', table.concat(state.result.lines, '\n'):find('"secure": true', 1, true) ~= nil)
H.check('...it says the certificate is not checked, in a warning', #H.notes_with('NOT verified') == 1, vim.inspect(H.notes))
H.check('...and in the result footer', H.lines()[#H.lines()]:find('TLS not verified', 1, true) ~= nil, H.lines()[#H.lines()])

state.result, state.result_log, state.result_pos = nil, {}, nil
request_file('direct.http', { 'GET https://127.0.0.1:' .. port .. '/' })
H.notes = {}
t.run()
H.check('a full URL straight at it is refused too', H.wait(5000, function()
  for _, n in ipairs(H.notes) do
    if n.level == vim.log.levels.ERROR then return true end
  end
end) and state.result == nil, vim.inspect(H.notes))
request_file('direct2.http', { '@insecure = 1', '', 'GET https://127.0.0.1:' .. port .. '/' })
t.run()
H.check('@insecure = 1 lets a request written at a URL through', H.wait(5000, function() return state.result and state.result.status == '200 OK' and state.result.conn == '@?insecure=1' end), vim.inspect(state.result and state.result.conn))
H.check('...and the footer says so', H.lines()[#H.lines()]:find('TLS not verified', 1, true) ~= nil, H.lines()[#H.lines()])

vim.fn.jobstop(job)
H.done()
