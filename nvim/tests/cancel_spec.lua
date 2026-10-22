-- Cancelling a running query (SQLite: a real sqlite3_interrupt).
local H = dofile('nvim/tests/helpers.lua')
local t = H.setup()
H.sql_buffer('cancel.sql')
H.check('connects', H.connect())
local spin = '[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]'
H.run('with recursive c(x) as (select 1 union all select x+1 from c where x < 900000000) select count(*) from c')
H.sleep(600)
H.check('shows a spinner while running', (t.status() or ''):find(spin), tostring(t.status()))
t.cancel()
H.check('cancel stops it quickly', H.wait(3000, function() return not (t.status() or ''):find(spin) end), tostring(t.status()))
H.check('the next query runs right away', H.run_until('select 42 as answer', '42', 3000), H.text())
H.done()
