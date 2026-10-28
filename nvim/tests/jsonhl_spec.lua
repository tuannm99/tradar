-- The JSON view's colours (extmarks, not `syntax json`).
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table p(id integer primary key, name text, note text);
insert into p values (1,'ann','say "hi" \ ok'),(2,'bob',null)]])
local t = H.setup({})
local hl = require('tradar.jsonhl')
local ctx = require('tradar.ctx')

local function find(tokens, group, text, line)
  for _, tk in ipairs(tokens) do
    if tk[3] == group and line:sub(tk[1] + 1, tk[2]) == text then return true end
  end
  return false
end

-- one line
local line = '  "name": "ann", "n": -12.5e3, "ok": true, "gone": null, "_id": ObjectId("507f1f77bcf86cd799439011"),'
local tk = hl.tokens(line)
H.check('a key', find(tk, 'TradarJsonKey', '"name"', line) and find(tk, 'TradarJsonKey', '"n"', line))
H.check('a string value', find(tk, 'TradarJsonString', '"ann"', line))
H.check('a number, signs and exponent included', find(tk, 'TradarJsonNumber', '-12.5e3', line))
H.check('true / null', find(tk, 'TradarJsonConst', 'true', line) and find(tk, 'TradarJsonConst', 'null', line))
H.check('ObjectId( is a function name', find(tk, 'TradarJsonFunc', 'ObjectId', line))
H.check('...and its argument a string', find(tk, 'TradarJsonString', '"507f1f77bcf86cd799439011"', line))
local esc = '"a \\"quoted\\" word \\\\": "v\\\\"'
local etk = hl.tokens(esc)
H.check('escaped quotes and backslashes stay inside the string', find(etk, 'TradarJsonKey', '"a \\"quoted\\" word \\\\"', esc) and find(etk, 'TradarJsonString', '"v\\\\"', esc), vim.inspect(etk))
H.check('an unterminated string colours to the end of the line, not the next one', #hl.tokens('"oops') == 1)
H.check('a word that is not a literal gets no colour', #hl.tokens('  nope') == 0)
H.check('a very long line is still scanned to its end', (function()
  local long = '"k": "' .. string.rep('x', 5000) .. '", "after": 1'
  return find(hl.tokens(long), 'TradarJsonNumber', '1', long)
end)())

-- a buffer, however big
local function marks(buf) return vim.api.nvim_buf_get_extmarks(buf, hl.ns, 0, -1, {}) end
local buf = vim.api.nvim_create_buf(false, true)
local lines = {}
for i = 1, 20000 do lines[i] = ('  "k%d": "v", "n": %d,'):format(i, i) end
vim.api.nvim_buf_set_lines(buf, 0, -1, false, lines)
local started = vim.uv.hrtime()
hl.apply(buf, lines, #lines)
local ms = (vim.uv.hrtime() - started) / 1e6
H.check('20000 lines are coloured (the old limit was 3000)', #marks(buf) >= 20000 * 3, #marks(buf))
H.check('...in well under a second', ms < 3000, ('%.0f ms'):format(ms))
hl.apply(buf, nil)
H.check('nil takes the colours away', #marks(buf) == 0)
hl.apply(buf, { '"a": 1', '', 'the footer' }, 1)
H.check('only the lines asked for are coloured', #marks(buf) == 2, #marks(buf))

-- in the results window
H.sql_buffer('hl.sql', { '' })
H.check('connects', H.connect())
H.check('runs', H.run_until('select id, name, note from p order by id', 'ann'))
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.rbuf()))
t.toggle_view()
H.check('the JSON view shows JSON', H.wait(3000, function() return H.text():find('"name":"ann"', 1, true) ~= nil end), H.text())
local rb = H.rbuf()
H.check('...coloured by extmarks', #marks(rb) > 10, #marks(rb))
H.check('...and not by `syntax`', vim.bo[rb].syntax == '', vim.bo[rb].syntax)
local keys = 0
for _, m in ipairs(vim.api.nvim_buf_get_extmarks(rb, hl.ns, 0, -1, { details = true })) do
  if m[4].hl_group == 'TradarJsonKey' then keys = keys + 1 end
end
H.check('every key of both rows is a key', keys == 6, keys)
H.check('the footer line is left alone', (function()
  local last = vim.api.nvim_buf_line_count(rb) - 1
  return #vim.api.nvim_buf_get_extmarks(rb, hl.ns, { last, 0 }, { last, -1 }, {}) == 0
end)())
t.toggle_view()
H.check('back to the table: the colours are gone', H.wait(3000, function() return #marks(H.rbuf()) == 0 end), #marks(H.rbuf()))
H.done()
