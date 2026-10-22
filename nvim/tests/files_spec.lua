-- :TradarSave (library) / :TradarSaveHere / :TradarOpen.
local H = dofile('nvim/tests/helpers.lua')
local lib, proj = H.tmp .. '/lib', H.tmp .. '/proj'
vim.fn.mkdir(proj, 'p')
H.setup({ queries_dir = lib })
local f = require('tradar.files')
local function rd(p)
  local fh = io.open(p)
  if not fh then return nil end
  local s = fh:read('*a'); fh:close(); return s
end

H.check('bare name goes to the library as .sql', f.resolve_library('daily') == lib .. '/daily.sql', f.resolve_library('daily'))
H.check('subfolder', f.resolve_library('reports/q1') == lib .. '/reports/q1.sql')
H.check('an extension is kept', f.resolve_library('x.tdb') == lib .. '/x.tdb')
H.check('an absolute path is taken as typed', f.resolve_library(H.tmp .. '/o/a.sql') == H.tmp .. '/o/a.sql')
H.check('.. inside a library name is refused', f.resolve_library('a/../../evil') == nil)
H.check('a bare folder is refused', f.resolve_library('dir/') == nil)

vim.cmd('enew'); vim.api.nvim_buf_set_lines(0, 0, -1, false, { 'select 1' })
vim.cmd('TradarSave reports/first')
H.check('unnamed buffer saved into the library, folder created', rd(lib .. '/reports/first.sql') == 'select 1\n')
H.check('the buffer becomes that file', vim.api.nvim_buf_get_name(0) == lib .. '/reports/first.sql')
H.check('filetype sql', vim.bo.filetype == 'sql', vim.bo.filetype)

vim.cmd('enew'); vim.api.nvim_buf_set_lines(0, 0, -1, false, { 'select 2' })
vim.cmd('TradarSave reports/first')
H.check('an existing file is not overwritten without !', rd(lib .. '/reports/first.sql') == 'select 1\n')
vim.cmd('TradarSave! reports/first')
H.check('! overwrites, even with the file open in another buffer', rd(lib .. '/reports/first.sql') == 'select 2\n')

vim.cmd('edit ' .. proj .. '/a.sql'); vim.api.nvim_buf_set_lines(0, 0, -1, false, { 'select 3' }); vim.cmd('write')
vim.b.tradar_connection = 'local'
vim.cmd('TradarSave copy')
H.check('a project file is copied to the library', (rd(lib .. '/copy.sql') or ''):find('select 3', 1, true))
H.check('its connection is pinned as a modeline', (rd(lib .. '/copy.sql') or ''):match('^%-%- tradar: local\n'), rd(lib .. '/copy.sql'))
H.check('the buffer keeps its own file', vim.api.nvim_buf_get_name(0) == proj .. '/a.sql')

vim.cmd('TradarSaveHere b')
H.check('a bare name lands next to the current file', rd(proj .. '/b.sql') ~= nil)
H.check('and the buffer is renamed to it', vim.api.nvim_buf_get_name(0) == proj .. '/b.sql')
vim.cmd('cd ' .. proj); vim.cmd('enew'); vim.api.nvim_buf_set_lines(0, 0, -1, false, { 'select 4' })
vim.cmd('TradarSaveHere sub/c')
H.check('an unnamed buffer lands relative to cwd', rd(proj .. '/sub/c.sql') == 'select 4\n')

vim.cmd('enew'); vim.api.nvim_buf_set_lines(0, 0, -1, false, { 'x' })
local asked
vim.ui.input = function(o, cb) asked = o; cb('prompted') end
vim.cmd('TradarSave')
H.check('no name: asks, then saves', asked and rd(lib .. '/prompted.sql') == 'x\n')

local names = vim.tbl_map(function(x) return x.rel end, f.library_files())
table.sort(names)
H.check('library lists nested files', vim.deep_equal(names, { 'copy.sql', 'prompted.sql', 'reports/first.sql' }), vim.inspect(names))
vim.ui.select = function(items, _, cb)
  for _, i in ipairs(items) do if i.rel == 'copy.sql' then return cb(i) end end
end
package.loaded['telescope.pickers'] = nil
vim.cmd('TradarOpen')
H.check(':TradarOpen without telescope opens the chosen file', vim.api.nvim_buf_get_name(0) == lib .. '/copy.sql', vim.api.nvim_buf_get_name(0))

H.done()
