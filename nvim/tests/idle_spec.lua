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
H.done()
