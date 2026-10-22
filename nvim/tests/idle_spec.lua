-- env: TRADAR_IDLE_TTL=3
-- The server closes an unused connection; the plugin reconnects quietly.
local H = dofile('nvim/tests/helpers.lua')
local t = H.setup()
H.sql_buffer('idle.sql')
H.check('connects', H.connect())
H.check('runs', H.run_until('select 1 as a', '1/1 rows'))
-- Only the plugin's 15 s `status` poll talks to the server now, and it must
-- not count as use.
H.check('the plugin notices the server closed it', vim.wait(25000, function() return not t.is_connected('local') end, 200))
H.check('the next run reconnects', H.run_until('select 2 as b', '1/1 rows', 8000), H.text())
H.check('no "not connected" error shown', #H.notes_with('not connected') == 0, vim.inspect(H.notes))
local log = io.open(os.getenv('TRADAR_LOG_FILE')):read('*a')
H.check('the server logged the idle disconnect', log:find('idle-disconnect [local]', 1, true), log)

-- Extension connections (HTTP here) are closed when idle too, and a request
-- sent afterwards -- an HTTP buffer left open -- reconnects by itself.
local port
local job = vim.fn.jobstart({ 'python3', '-I', '-c', [[
import http.server, socketserver
s = socketserver.TCPServer(('127.0.0.1', 0), http.server.SimpleHTTPRequestHandler)
print(s.server_address[1], flush=True)
s.serve_forever()
]] }, { cwd = H.tmp, stdout_buffered = false, on_stdout = function(_, data)
  port = port or tonumber(data[1])
end })
H.wait(5000, function() return port end)
local f = io.open(os.getenv('XDG_CONFIG_HOME') .. '/tradar/connections.toml', 'a')
f:write(('\n[[connections]]\nname = "web"\ndriver = "http"\ntarget = "http://127.0.0.1:%d"\n'):format(port))
f:close()
local rpc = require('tradar.rpc')
local function req(method, params)
  local done, err, res = false, nil, nil
  rpc.request(method, params, function(e, r) done, err, res = true, e, r end)
  H.wait(8000, function() return done end)
  return err, res
end
H.check('http connects', req('http.connect', { connection = 'web' }) == nil)
H.check('the idle HTTP connection is closed', H.wait(15000, function()
  return io.open(os.getenv('TRADAR_LOG_FILE')):read('*a'):find('idle-disconnect [web]', 1, true) ~= nil
end))
local err, res = req('http.send', { connection = 'web', method = 'GET', url = '/', headers = '', body = '' })
H.check('a send afterwards reconnects by itself', err == nil and res and res.status == 200, vim.inspect({ err, res }))
vim.fn.jobstop(job)
H.done()
