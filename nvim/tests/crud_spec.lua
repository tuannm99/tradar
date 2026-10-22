-- Generating create/read/update/delete statements for a table.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table people(id integer primary key, name text, age integer);
create table orders(id integer primary key, person_id integer)]])
local t = H.setup()
H.sql_buffer('crud.sql', { 'select 1;', 'people', 'select 2;' })
H.check('connects', H.connect())
t.attach(H.sqlbuf)
local function buf_text() return table.concat(vim.api.nvim_buf_get_lines(H.sqlbuf, 0, -1, false), '\n') end
local function reset(lines)
  vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, lines)
  vim.api.nvim_set_current_win(vim.fn.bufwinid(H.sqlbuf))
end

-- Drives vim.ui.select with scripted answers (matched by label substring).
local asked
local function answers(list)
  local i = 0
  asked = {}
  vim.ui.select = function(items, opts, cb)
    i = i + 1
    local want = list[i]
    asked[#asked + 1] = opts and opts.prompt
    if want == nil then return cb(nil) end
    local fmt = opts.format_item or tostring
    for _, it in ipairs(items) do
      if fmt(it):find(want, 1, true) then return cb(it) end
    end
    error(('no item matching %q in %s'):format(want, vim.inspect(vim.tbl_map(fmt, items))))
  end
end
local function wait_text(needle) return H.wait(4000, function() return buf_text():find(needle, 1, true) ~= nil end) end

-- cursor on the table name: no table prompt
reset({ 'select 1;', 'people', 'select 2;' })
vim.api.nvim_win_set_cursor(0, { 2, 1 })
answers({ 'All columns' })
vim.cmd('TradarCrud create')
H.check(':TradarCrud create uses the table under the cursor', wait_text('INSERT INTO'), buf_text())
H.check('all columns in the INSERT', buf_text():find('"name"', 1, true) and buf_text():find('<age>', 1, true), buf_text())
H.check('it is inserted below the cursor line, between blank lines', buf_text():find('people\n\nINSERT INTO', 1, true) and buf_text():find(');\n\nselect 2;', 1, true), buf_text())
H.check('the SQL is not run', H.rbuf() == -1)
H.check('only the connection asked: no table prompt', #asked == 1, vim.inspect(asked))

-- pick columns for a read
reset({ 'people' })
vim.api.nvim_win_set_cursor(0, { 1, 1 })
answers({ 'Pick columns', 'name', 'age', 'Done with 2' })
vim.cmd('TradarCrud read')
H.check('picked columns narrow the SELECT', wait_text('SELECT'), buf_text())
H.check('only the picked columns, in table order', buf_text():find('SELECT "name", "age" FROM "people"', 1, true) ~= nil, buf_text())

-- update + delete
reset({ 'people' })
vim.api.nvim_win_set_cursor(0, { 1, 1 })
answers({ 'All columns' })
vim.cmd('TradarCrud update')
H.check('update has SET and a key condition', wait_text('UPDATE') and buf_text():find('SET', 1, true) and buf_text():find('"id" = <id>', 1, true), buf_text())
reset({ 'people' })
vim.api.nvim_win_set_cursor(0, { 1, 1 })
answers({})
vim.cmd('TradarCrud delete')
H.check('delete asks no column question', wait_text('DELETE FROM') and #asked == 0, vim.inspect(asked))
H.check('delete is keyed', buf_text():find('"id" = <id>', 1, true) ~= nil, buf_text())

-- no table under the cursor: pick the table, then the operation
reset({ 'nothing here' })
vim.api.nvim_win_set_cursor(0, { 1, 1 })
answers({ 'orders', 'Read', 'All columns' })
vim.cmd('TradarCrud')
H.check('without a table under the cursor it asks which', wait_text('FROM "orders"'), buf_text())
H.check('table, operation, columns: three questions', #asked == 3, vim.inspect(asked))

-- cancelling does nothing
reset({ 'people' })
vim.api.nvim_win_set_cursor(0, { 1, 1 })
answers({})
vim.cmd('TradarCrud')
H.sleep(300)
H.check('cancelling the operation inserts nothing', buf_text() == 'people', buf_text())

-- a bad operation name
vim.cmd('TradarCrud frobnicate')
H.check('an unknown operation is refused', #H.notes_with('usage: :TradarCrud') == 1, vim.inspect(H.notes))
H.check(':TradarCrud completes the four operations', vim.deep_equal(vim.fn.getcompletion('TradarCrud ', 'cmdline'), { 'create', 'read', 'update', 'delete' }))
H.check('<leader>rg is mapped', H.mapped(H.sqlbuf, '<leader>rg'))

-- schema panel `c`
reset({ 'select 1;' })
t.schema()
H.wait(3000, function() return vim.fn.bufnr('tradar://schema') ~= -1 end)
local sb = vim.fn.bufnr('tradar://schema')
vim.api.nvim_set_current_win(vim.fn.bufwinid(sb))
local lines = vim.api.nvim_buf_get_lines(sb, 0, -1, false)
local row
for i, l in ipairs(lines) do if l:find('people', 1, true) then row = i end end
H.check('schema panel lists the table', row ~= nil, vim.inspect(lines))
vim.api.nvim_win_set_cursor(0, { row, 0 })
answers({ 'Delete' })
vim.api.nvim_feedkeys('c', 'x', false)
H.check('`c` in the schema panel generates for that table', wait_text('DELETE FROM "people"'), buf_text())
H.done()
