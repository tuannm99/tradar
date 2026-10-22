-- Completing connection names, and `:Tradar <subcommand>`.
local H = dofile('nvim/tests/helpers.lua')
local f = io.open(os.getenv('XDG_CONFIG_HOME') .. '/tradar/connections.toml', 'a')
f:write('\n[[connections]]\nname = "k1"\ndriver = "kafka"\ntarget = "127.0.0.1:1"\n')
f:write('\n[[connections]]\nname = "web"\ndriver = "http"\ntarget = "http://127.0.0.1:1"\n')
f:close()
H.sql('create table p(id integer primary key); insert into p values (7)')
local t = H.setup({ queries_dir = H.tmp .. '/lib' })
local function comp(line) return vim.fn.getcompletion(line, 'cmdline') end

-- No server contact yet: names come from connections.toml itself.
H.check('server not contacted yet', not require('tradar.rpc').connected())
H.check(':TradarConnect completes query connections only', vim.deep_equal(comp('TradarConnect '), { 'local' }), vim.inspect(comp('TradarConnect ')))
H.check(':TradarKafkaTail completes kafka connections', vim.deep_equal(comp('TradarKafkaTail '), { 'k1' }), vim.inspect(comp('TradarKafkaTail ')))
H.check('completion filters by what is typed', vim.deep_equal(comp('TradarHttpSend w'), { 'web' }) and #comp('TradarHttpSend x') == 0)
H.check('only the first argument is a connection', #comp('TradarKafkaTail k1 ') == 0, vim.inspect(comp('TradarKafkaTail k1 ')))

-- `:Tradar <subcommand>`
local top = comp('Tradar ')
H.check(':Tradar lists subcommands', vim.tbl_contains(top, 'connect') and vim.tbl_contains(top, 'kafka') and vim.tbl_contains(top, 'run'), vim.inspect(top))
H.check(':Tradar kafka completes its second word', vim.deep_equal(comp('Tradar kafka '), { 'connect', 'tail' }), vim.inspect(comp('Tradar kafka ')))
H.check(':Tradar run offers `all`', vim.tbl_contains(comp('Tradar run '), 'all'), vim.inspect(comp('Tradar run ')))
H.check(':Tradar kafka tail completes the connection', vim.deep_equal(comp('Tradar kafka tail '), { 'k1' }), vim.inspect(comp('Tradar kafka tail ')))
H.check(':Tradar connect completes the connection', vim.deep_equal(comp('Tradar connect l'), { 'local' }))

H.sql_buffer('cmd.sql', { 'select id from p' })
vim.cmd('Tradar connect local')
H.check(':Tradar connect local connects', H.wait(5000, function() return t.is_connected('local') end))
H.check('with a server, names come from it', vim.deep_equal(comp('TradarConnect '), { 'local' }))
vim.cmd('1Tradar run')
H.check(':Tradar run (with a range) runs', H.wait(5000, function() return H.text():find('1/1 rows', 1, true) ~= nil end), H.text())
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
vim.cmd('Tradar save mine')
H.check(':Tradar save passes its argument on', vim.fn.filereadable(H.tmp .. '/lib/mine.sql') == 1)
vim.cmd('Tradar no such thing')
H.check('an unknown subcommand says so', #H.notes_with('unknown subcommand') == 1, vim.inspect(H.notes))
H.done()
