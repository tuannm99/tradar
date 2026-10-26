-- Pure Lua: grid rendering and the grid-edit diff (no server calls).
local H = dofile('nvim/tests/helpers.lua')
H.setup()
local render, g = require('tradar.render'), require('tradar.gridedit')

-- column widths
local long = string.rep('x', 120)
H.check('a one-column result is not cut at 40', render.column_widths({ 'explain' }, { { long } })[1] == 120)
local w = render.column_widths({ 'name', 'path' }, { { long, long } })
H.check('middle columns are capped, the last is not', w[1] == render.MAX_COLUMN_WIDTH and w[2] == 120, vim.inspect(w))
H.check('the last column is still bounded',
  render.column_widths({ 'a' }, { { string.rep('y', 5000) } })[1] == render.MAX_LAST_COLUMN_WIDTH)
H.check('an uncut cell has no ellipsis', not render.row_lines({ 'explain' }, { 120 }, { { long } })[1]:find('…', 1, true))

-- grid edit diff
local cols = { 'id', 'name', 'bio' }
local rows = { { '1', 'ann', 'a\nb' }, { '2', 'bob', string.rep('x', 60) }, { '3', 'cy', 'NULL' }, { '4', 'dee', '' } }
local lines, _, widths = render.table(cols, rows)
local data = { unpack(lines, 3) }
local function diff(d) return g.diff(cols, rows, widths, { 1 }, d) end
local function changes(r) return #r.updates + #r.deletes + #r.inserts + #r.errors end
local copy = vim.deepcopy

local r = diff(data)
H.check('untouched grid: no changes (truncation, newline, NULL)', changes(r) == 0, vim.inspect(r))
local d = copy(data); d[1] = d[1]:gsub('ann', 'ANN', 1); r = diff(d)
H.check('cell edit is an update', #r.updates == 1 and r.updates[1].row == 1 and r.updates[1].cells[2] == 'ANN' and #r.deletes == 0, vim.inspect(r))
d = copy(data); table.remove(d, 2); r = diff(d)
H.check('dd is a delete', #r.deletes == 1 and r.deletes[1].row == 2 and #r.updates == 0, vim.inspect(r))
d = copy(data); table.remove(d, 2); d[2] = d[2]:gsub('cy', 'CY', 1); r = diff(d)
H.check('delete next to an edit is not mispaired', #r.deletes == 1 and r.deletes[1].row == 2 and #r.updates == 1 and r.updates[1].row == 3, vim.inspect(r))
d = copy(data); d[#d + 1] = '5 │ eve │ '; r = diff(d)
H.check('new key is an insert, empty cells omitted', #r.inserts == 1 and r.inserts[1].values[2] == 'eve' and r.inserts[1].values[3] == nil, vim.inspect(r))
d = copy(data); table.insert(d, 2, d[1]); r = diff(d)
H.check('a duplicated key is an error', #r.errors == 1, vim.inspect(r))
d = copy(data); d[1], d[3] = d[3], d[1]; r = diff(d)
H.check('moving lines changes nothing', changes(r) == 0, vim.inspect(r))
d = copy(data); d[1] = '1 │ ann'; r = diff(d)
H.check('mangled separators are an error', #r.errors == 1, vim.inspect(r))
d = copy(data); table.insert(d, 1, '   '); r = diff(d)
H.check('a blank line is ignored', changes(r) == 0, vim.inspect(r))
d = copy(data); d[1] = d[1]:gsub('^(%s*)1', '%110', 1); r = diff(d)
H.check('editing the key is delete + insert', #r.deletes == 1 and #r.inserts == 1, vim.inspect(r))

-- Mongo shell literals in the JSON view
local render = require('tradar.render')
local doc = { _id = 'ObjectId("507f1f77bcf86cd799439011")', at = 'ISODate("2024-05-01T10:00:00Z")', name = 'ObjectId("not hex")', tags = { 'ObjectId("507f1f77bcf86cd799439012")' } }
local shown = table.concat(render.documents({ doc }, { { '_id', 'at', 'name', 'tags' } }), '\n')
H.check('an ObjectId prints as a shell literal, not an escaped string', shown:find('"_id": ObjectId("507f1f77bcf86cd799439011")', 1, true) ~= nil, shown)
H.check('...and an ISODate', shown:find('"at": ISODate("2024-05-01T10:00:00Z")', 1, true) ~= nil, shown)
H.check('...inside arrays too', shown:find('  ObjectId("507f1f77bcf86cd799439012")', 1, true) ~= nil, shown)
H.check('a string that only looks like one stays a string', shown:find('"name": "ObjectId(\\"not hex\\")"', 1, true) ~= nil, shown)
H.check('exports stay valid JSON', render.pretty(doc, { '_id' }):find('"_id": "ObjectId(\\"507f1f77bcf86cd799439011\\")"', 1, true) ~= nil)

H.done()
