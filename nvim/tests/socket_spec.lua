-- Unix socket paths have a hard length limit; the plugin picks a short
-- default and explains an explicit path that does not fit.
local H = dofile('nvim/tests/helpers.lua')
H.setup()
local rpc = require('tradar.rpc')
local limit = rpc.max_socket_path()
H.check('the limit is a real OS limit', limit == 107 or limit == 103, limit)

local saved = vim.env.XDG_RUNTIME_DIR
vim.env.XDG_RUNTIME_DIR = '/run/user/1000'
H.check('a normal runtime dir is used as is', rpc.socket_path({}) == '/run/user/1000/tradar/server.sock', rpc.socket_path({}))

vim.env.XDG_RUNTIME_DIR = '/run/' .. string.rep('d', 120)
local fallback = rpc.socket_path({})
H.check('a runtime dir too long for a socket falls back to a short /tmp path', fallback:match('^/tmp/tradar%-[^/]+/server%.sock$') ~= nil and #fallback <= limit, fallback)
H.check('the fallback is never rejected', rpc.check_path(fallback) == nil)
vim.env.XDG_RUNTIME_DIR = saved

H.check('an explicit path is never rewritten', rpc.socket_path({ socket = '/x/y.sock' }) == '/x/y.sock')
local long = '/tmp/' .. string.rep('a', limit) .. '/s.sock'
local msg = rpc.check_path(long)
H.check('a path over the limit says so and what to do', msg and msg:find('at most ' .. limit, 1, true) and msg:find('setup{ socket', 1, true), msg)
H.check('a path at the limit is fine', rpc.check_path(string.rep('a', limit)) == nil)

-- ensure() reports it instead of waiting for a server that cannot start
local result
rpc.ensure({ socket = long, autostart = true }, function(err) result = err or 'ok' end)
H.check('ensure fails fast with that message', H.wait(1000, function() return result ~= nil end) and result:find('at most', 1, true) ~= nil, result)

-- :checkhealth flags it
require('tradar').setup({ socket = long })
local health = {}
local real = vim.health
vim.health = setmetatable({
  error = function(m) health[#health + 1] = 'error: ' .. m end,
  ok = function(m) health[#health + 1] = 'ok: ' .. m end,
  warn = function(m) health[#health + 1] = 'warn: ' .. m end,
  info = function(m) health[#health + 1] = 'info: ' .. m end,
  start = function() end,
}, { __index = real })
require('tradar.health').check()
vim.health = real
local flagged = false
for _, l in ipairs(health) do if l:find('^error: socket path is') then flagged = true end end
H.check(':checkhealth reports a too-long socket path as an error', flagged, vim.inspect(health))
-- the environment the plugin gives a server it starts
H.check('no idle_ttl and no server_env: nothing added', rpc.server_env({}) == nil)
local env = rpc.server_env({ idle_ttl = '10m', server_env = { NO_PROXY = '*', HTTPS_PROXY = '', N = 3 } })
H.check('idle_ttl and server_env are passed on, values as strings', env.TRADAR_IDLE_TTL == '10m' and env.NO_PROXY == '*' and env.HTTPS_PROXY == '' and env.N == '3', vim.inspect(env))
H.check('idle_ttl = false means never', rpc.server_env({ idle_ttl = false }).TRADAR_IDLE_TTL == 'off')
H.done()
