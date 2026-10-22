-- Buffer integration: modeline, schema, completion, keymaps, explain, guard.
local H = dofile('nvim/tests/helpers.lua')
H.sql('create table p(id integer primary key, name text); insert into p values (1, \'a\')')
local t = H.setup()
local sqlbuf = H.sql_buffer('api.sql', { '-- tradar: local', 'select * from p;', 'select id from p;' })
t.attach(sqlbuf)
H.check('attaching connects in the background', H.wait(5000, function() return t.is_connected('local') end))
H.check('connection_for reads the modeline', t.connection_for(sqlbuf) == 'local', tostring(t.connection_for(sqlbuf)))
H.check('statusline names the connection', tostring(t.status()):find('local', 1, true), tostring(t.status()))

local entries
t.schema_entries(function(e) entries = e end)
H.wait(3000, function() return entries end)
H.check('schema lists the table', entries and entries[1] and entries[1].name == 'p', vim.inspect(entries))
local cands
t.complete(sqlbuf, 'select na', function(c) cands = c end, 2)
H.wait(3000, function() return cands end)
H.check('completion returns candidates', cands and #cands > 0)

H.check('SQL buffer maps run/zoom/K/gd',
  H.mapped(sqlbuf, '<leader>rr') and H.mapped(sqlbuf, '<leader>rz') and H.mapped(sqlbuf, 'K') and H.mapped(sqlbuf, 'gd'))

t.run(2, 3)
H.wait(5000, function() return H.text():find('rows', 1, true) end)
H.check('a two-statement range runs', H.rbuf() ~= -1 and H.text():find('rows', 1, true), H.text())
for _, lhs in ipairs({ 'gyc', 'gyr', 'gyC', 'gyj', 'gK', 'K', '<leader>rv', '<leader>rm', '<leader>ru', 'g?' }) do
  H.check('results maps ' .. lhs, H.mapped(H.rbuf(), lhs))
end
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.rbuf()))
vim.api.nvim_win_set_cursor(0, { 3, 0 })
H.check('row view', pcall(t.row_view))
H.check('table/JSON toggle round trip', pcall(t.toggle_view) and pcall(t.toggle_view))
H.check('yank as JSON', pcall(t.yank_all, 'json'))

vim.api.nvim_set_current_win(vim.fn.bufwinid(sqlbuf))
vim.api.nvim_win_set_cursor(0, { 2, 0 })
H.check('explain', pcall(t.explain))
H.check('hover', pcall(t.hover))
H.check('cancel with nothing running', pcall(t.cancel))
H.check('schema panel', pcall(t.schema))
H.wait(2000, function() return vim.fn.bufnr('tradar://schema') ~= -1 end)
H.check('schema panel buffer', vim.fn.bufnr('tradar://schema') ~= -1)

-- guard: a destructive statement asks first, and nothing runs on "no"
local guarded = H.sql_buffer('guard.sql', { '-- tradar: local', 'delete from p' })
t.attach(guarded)
local asked = false
vim.ui.select = function(_, _, cb) asked = true; cb(nil) end
vim.fn.confirm = function() asked = true; return 2 end
vim.api.nvim_win_set_cursor(0, { 2, 0 })
t.run()
H.wait(1500, function() return asked end)
H.check('DELETE without WHERE asks first', asked)
H.sleep(300)
H.check('declining runs nothing', H.sql('select count(*) from p')[1][1] == '1')

H.done()
