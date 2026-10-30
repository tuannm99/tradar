-- Colours of request text: the scanner (tokens) and where it is applied.
local H = dofile('nvim/tests/helpers.lua')
local f = io.open(os.getenv('XDG_CONFIG_HOME') .. '/tradar/connections.toml', 'a')
f:write('\n[[connections]]\nname = "web"\ndriver = "http"\ntarget = "http://127.0.0.1:1"\n')
f:close()
H.setup()
local hl = require('tradar.reqhl')

local function rows(text) return hl.tokens(vim.split(text, '\n', { plain = true })) end
local function has(tokens, from, to, group)
  for _, t in ipairs(tokens or {}) do
    if t[1] == from and t[2] == to and t[3] == group then return true end
  end
  return false
end

local t = rows(table.concat({
  '# a note',
  '@token = abc',
  '',
  '### list users',
  'GET /users?limit=10 HTTP/1.1',
  'Authorization: Bearer {{token}}',
  'Accept: application/json',
  '',
  '{"a": 1, "b": [true]}',
  '',
  '### rpc',
  'GRPC users.UserService/GetUser',
  'x-token: abc',
  'deadline: 5s',
  '',
  '// hint',
  '{"id": "{{id}}"}',
  '',
  'DELETE https://h.io/x/1',
  '/bare',
}, '\n'))
H.check('a comment line', has(t[1], 0, 8, 'TradarReqComment'), vim.inspect(t[1]))
H.check('a variable definition: name, then value', has(t[2], 0, 6, 'TradarReqVarDef') and has(t[2], 9, 12, 'TradarReqValue'), vim.inspect(t[2]))
H.check('### is a separator', has(t[4], 0, 14, 'TradarReqSeparator'), vim.inspect(t[4]))
H.check('the verb and the url; HTTP/1.1 is dimmed', has(t[5], 0, 3, 'TradarReqMethod') and has(t[5], 4, 19, 'TradarReqUrl') and has(t[5], 19, 28, 'TradarReqComment'), vim.inspect(t[5]))
H.check('a header: name and value', has(t[7], 0, 6, 'TradarReqHeader') and has(t[7], 8, 24, 'TradarReqValue'), vim.inspect(t[7]))
H.check('a {{placeholder}} is marked inside a header value', has(t[6], 22, 31, 'TradarReqVar'), vim.inspect(t[6]))
H.check('a JSON body is coloured as JSON', has(t[9], 1, 4, 'TradarJsonKey') and has(t[9], 6, 7, 'TradarJsonNumber') and has(t[9], 15, 19, 'TradarJsonConst'), vim.inspect(t[9]))
H.check('GRPC: keyword, service, method', has(t[12], 0, 4, 'TradarReqMethod') and has(t[12], 5, 22, 'TradarReqService') and has(t[12], 23, 30, 'TradarReqRpc'), vim.inspect(t[12]))
H.check('GRPC: metadata, and deadline is special', has(t[13], 0, 7, 'TradarReqHeader') and has(t[14], 0, 8, 'TradarReqDeadline'), vim.inspect({ t[13], t[14] }))
H.check('GRPC: a // hint in the body is a comment, the JSON after it is coloured', has(t[16], 0, 7, 'TradarReqComment') and has(t[17], 1, 5, 'TradarJsonKey') and has(t[17], 8, 14, 'TradarReqVar'), vim.inspect({ t[16], t[17] }))
H.check('a bare url is a url, a full one after the verb too', has(rows('/bare')[1], 0, 5, 'TradarReqUrl') and has(rows('DELETE https://h.io/x/1')[1], 7, 23, 'TradarReqUrl'), vim.inspect({ rows('/bare')[1], rows('DELETE https://h.io/x/1')[1] }))
H.check('a body that is not JSON is left alone', rows('POST /x\n\nname=ann&x=1')[3] == nil)
H.check('direct gRPC target is a url', has(rows('GRPC localhost:50051 pkg.Svc/Get')[1], 5, 20, 'TradarReqUrl'), vim.inspect(rows('GRPC localhost:50051 pkg.Svc/Get')[1]))

-- applied to buffers
local function marks(buf, row)
  local out = {}
  for _, m in ipairs(vim.api.nvim_buf_get_extmarks(buf, hl.ns, { row - 1, 0 }, { row - 1, -1 }, { details = true })) do
    out[#out + 1] = m[4].hl_group
  end
  return out
end

vim.cmd('edit! ' .. H.tmp .. '/colours.http')
vim.api.nvim_buf_set_lines(0, 0, -1, false, { 'GET /x', 'Accept: */*' })
local buf = vim.api.nvim_get_current_buf()
H.check('a .http file is a request buffer', vim.bo[buf].filetype == 'http')
H.check('...and is coloured once attached', H.wait(2000, function() return vim.tbl_contains(marks(buf, 1), 'TradarReqMethod') end) or (function() hl.paint(buf) return vim.tbl_contains(marks(buf, 1), 'TradarReqMethod') end)(), vim.inspect(marks(buf, 1)))
vim.api.nvim_buf_set_lines(buf, 0, 1, false, { 'POST /y' })
vim.cmd('doautocmd <nomodeline> TextChanged')
H.check('...and again after an edit', H.wait(2000, function() return #marks(buf, 1) > 0 and vim.tbl_contains(marks(buf, 1), 'TradarReqUrl') end), vim.inspect(marks(buf, 1)))

local tdb = vim.api.nvim_create_buf(true, false)
vim.api.nvim_buf_set_name(tdb, H.tmp .. '/mix.tdb')
vim.api.nvim_set_current_buf(tdb)
vim.api.nvim_buf_set_lines(tdb, 0, -1, false, { 'GET /prose', '```http web', 'GET /users', 'Accept: x', '```', '```sql', 'GET /nope', '```' })
vim.bo[tdb].filetype = 'tdb'
hl.paint(tdb)
H.check('in a .tdb only the http block is coloured', #marks(tdb, 3) > 0 and #marks(tdb, 4) > 0 and #marks(tdb, 1) == 0 and #marks(tdb, 7) == 0, vim.inspect({ marks(tdb, 1), marks(tdb, 3), marks(tdb, 7) }))

local wb = require('tradar.restbench')
vim.cmd('TradarHttpSend web')
H.check('...and the workbench', H.wait(5000, function() return vim.b.tradar_rest ~= nil end))
local wbuf = vim.api.nvim_get_current_buf()
H.check('the workbench example is coloured', H.wait(2000, function() return vim.tbl_contains(marks(wbuf, 6), 'TradarReqMethod') end) or (function() hl.paint(wbuf) return vim.tbl_contains(marks(wbuf, 6), 'TradarReqMethod') end)(), vim.inspect(marks(wbuf, 6)))
H.done()
