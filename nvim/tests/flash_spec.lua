-- The statement being run is briefly highlighted.
local H = dofile('nvim/tests/helpers.lua')
H.sql('create table p(id integer primary key); insert into p values (1)')
local t = H.setup({ flash_ms = 400 })
H.sql_buffer('flash.sql', { 'select 1 as a;', '', 'select id,', '  2 as b', '  from p;', '', 'select 3 as c;' })
H.check('connects', H.connect())
local ns = vim.api.nvim_get_namespaces().tradar_flash
H.check('the flash namespace exists', ns ~= nil)
local function marks() return vim.api.nvim_buf_get_extmarks(H.sqlbuf, ns, 0, -1, { details = true }) end

-- cursor inside the middle (multi-line) statement
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
vim.api.nvim_win_set_cursor(0, { 4, 3 })
t.run()
H.check('a mark appears over the statement', H.wait(3000, function() return #marks() == 1 end), vim.inspect(marks()))
local m = marks()[1]
local d = m and m[4] or {}
H.check('it covers exactly that statement (rows 2..4, 0-based)', m and m[2] == 2 and d.end_row == 4, vim.inspect(m))
H.check('it ends at the statement itself (the `;` is not part of it)', d.end_col == #'  from p', vim.inspect(d))
H.check('it uses the TradarRunning group', d.hl_group == 'TradarRunning', vim.inspect(d))
H.check('and goes away on its own', H.wait(2000, function() return #marks() == 0 end))
H.check('the query still ran', H.wait(3000, function() return H.text():find('rows', 1, true) ~= nil end), H.text())

-- run all marks each statement in turn
local seen = {}
local timer = vim.uv and vim.uv.new_timer() or vim.loop.new_timer()
timer:start(0, 10, vim.schedule_wrap(function()
  for _, mk in ipairs(marks()) do seen[mk[2]] = true end
end))
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
t.run(nil, nil, true)
H.wait(6000, function() return seen[0] and seen[2] and seen[6] end)
timer:stop()
H.check('run all marks every statement as it starts', seen[0] and seen[2] and seen[6], vim.inspect(seen))

-- off
H.wait(2000, function() return #marks() == 0 end) -- the last run-all mark
require('tradar').setup({ socket = os.getenv('TRADAR_SOCKET'), flash_ms = 0 })
local before = 0
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
vim.api.nvim_win_set_cursor(0, { 1, 0 })
t.run()
H.sleep(300)
H.check('flash_ms = 0 turns it off', #marks() == 0)
H.done()
