-- Shared setup for nvim/tests/*_spec.lua, run by nvim/tests/run.sh: each spec
-- gets a fresh tradar-server ($TRADAR_SOCKET) whose only saved connection,
-- "local", is an empty SQLite file in $TRADAR_TEST_TMP.
local H = {}

H.tmp = assert(os.getenv('TRADAR_TEST_TMP'), 'run through nvim/tests/run.sh')
H.db_path = H.tmp .. '/t.db'

local out = assert(io.open(H.tmp .. '/results', 'w'))
local failures = 0

--- Records one assertion; `detail` is shown only on failure.
function H.check(name, ok, detail)
  ok = not not ok
  if not ok then failures = failures + 1 end
  local line = (ok and 'PASS ' or 'FAIL ') .. name
  if not ok and detail ~= nil then line = line .. ' -- ' .. tostring(detail):gsub('\n', '\\n') end
  out:write(line .. '\n')
  out:flush()
end

--- Ends the spec. A spec that errors before this never writes DONE, which
--- the runner counts as a failure.
function H.done()
  out:write('DONE\n')
  out:close()
  vim.cmd(failures > 0 and 'cquit 1' or 'qall!')
end

--- Runs SQL straight against the test database (seeding, or checking what a
--- commit did) -- not through the plugin. Returns the rows of the last
--- statement as lists of strings (NULL as 'NULL').
function H.sql(script)
  local py = [[
import sqlite3, sys, json
c = sqlite3.connect(sys.argv[1])
stmts = [s for s in sys.argv[2].split(';') if s.strip()]
rows = []
for s in stmts:
    rows = c.execute(s).fetchall()
c.commit()
print(json.dumps([['NULL' if v is None else str(v) for v in r] for r in rows]))
]]
  local res = vim.fn.system({ 'python3', '-I', '-c', py, H.db_path, script })
  assert(vim.v.shell_error == 0, res)
  return vim.json.decode(res)
end

--- Puts telescope.nvim (and plenary) on the runtimepath, from the copies
--- `deps.sh` cloned. Returns whether telescope loads.
function H.telescope()
  local deps = vim.fn.getcwd() .. '/.deps'
  vim.opt.rtp:append(deps .. '/plenary.nvim')
  vim.opt.rtp:append(deps .. '/telescope.nvim')
  vim.cmd('runtime! plugin/plenary.vim')
  local ok, err = pcall(require, 'telescope')
  if not ok then H.check('telescope.nvim loads (.deps/, see nvim/tests/deps.sh)', false, err) end
  return ok
end

function H.wait(ms, pred) return vim.wait(ms, pred, 20) end
function H.sleep(ms) vim.wait(ms, function() return false end, 20) end

--- Loads the plugin the way a user's config would, then `setup(opts)`.
--- Messages go to `H.notes` instead of the screen.
function H.setup(opts)
  vim.opt.rtp:prepend(vim.fn.getcwd() .. '/nvim')
  vim.opt.swapfile = false
  vim.cmd('filetype on')
  vim.cmd('source nvim/plugin/tradar.lua')
  H.notes = {}
  vim.notify = function(m, level) H.notes[#H.notes + 1] = { msg = tostring(m), level = level } end
  H.t = require('tradar')
  H.t.setup(vim.tbl_extend('force', { socket = os.getenv('TRADAR_SOCKET') }, opts or {}))
  return H.t
end

--- Notes whose text contains `needle`.
function H.notes_with(needle)
  return vim.tbl_filter(function(n) return n.msg:find(needle, 1, true) ~= nil end, H.notes)
end

--- Opens `<tmp>/<name>` as a SQL buffer holding `lines`; makes it current.
function H.sql_buffer(name, lines)
  vim.cmd('edit! ' .. vim.fn.fnameescape(H.tmp .. '/' .. name))
  vim.bo.filetype = 'sql'
  if lines then vim.api.nvim_buf_set_lines(0, 0, -1, false, lines) end
  H.sqlbuf = vim.api.nvim_get_current_buf()
  return H.sqlbuf
end

function H.connect()
  H.t.connect('local')
  return H.wait(5000, function() return H.t.is_connected('local') end)
end

--- Replaces the SQL buffer with `sql` and runs it from line 1.
function H.run(sql)
  local win = vim.fn.bufwinid(H.sqlbuf)
  if win ~= -1 then vim.api.nvim_set_current_win(win) end
  vim.api.nvim_buf_set_lines(H.sqlbuf, 0, -1, false, type(sql) == 'table' and sql or { sql })
  vim.api.nvim_win_set_cursor(0, { 1, 0 })
  H.t.run()
end

function H.rbuf()
  local b = require('tradar.ctx').state.results_buf
  return b and vim.api.nvim_buf_is_valid(b) and b or -1
end

function H.lines()
  local b = H.rbuf()
  return b ~= -1 and vim.api.nvim_buf_get_lines(b, 0, -1, false) or {}
end

function H.text(sep) return table.concat(H.lines(), sep or '\n') end

--- Runs `sql` and waits until the results text contains `needle`.
function H.run_until(sql, needle, ms)
  H.run(sql)
  return H.wait(ms or 5000, function() return H.text():find(needle, 1, true) ~= nil end)
end

--- `lhs` is mapped in normal mode in `buf` (comparing `<leader>` expanded).
local function keycode(s) return vim.api.nvim_replace_termcodes(s, true, true, true) end

function H.mapped(buf, lhs)
  local want = keycode(lhs)
  for _, m in ipairs(vim.api.nvim_buf_get_keymap(buf, 'n')) do
    if keycode(m.lhs) == want then return true end
  end
  return false
end

return H
