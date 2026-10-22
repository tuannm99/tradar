-- :TradarAddConnection -- the add-a-connection questions.
local H = dofile('nvim/tests/helpers.lua')
local t = H.setup()
local conf = os.getenv('XDG_CONFIG_HOME') .. '/tradar/connections.toml'
local function file() local f = io.open(conf, 'rb'); local s = f:read('*a'); f:close(); return s end
local original = file()

-- Scripted answers. A `select` answer is a substring of the item shown.
local selects, inputs, asked = {}, {}, {}
vim.ui.select = function(items, o, cb)
  asked[#asked + 1] = { kind = 'select', prompt = o.prompt }
  local want = table.remove(selects, 1)
  if want == nil then return cb(nil) end
  for _, it in ipairs(items) do
    if (o.format_item or tostring)(it):find(want, 1, true) then return cb(it) end
  end
  error(('nothing matching %q in %s'):format(want, vim.inspect(vim.tbl_map(o.format_item or tostring, items))))
end
vim.ui.input = function(o, cb)
  asked[#asked + 1] = { kind = 'input', prompt = o.prompt, default = o.default }
  cb(table.remove(inputs, 1))
end
local function run(sel, inp)
  selects, inputs, asked = sel, inp, {}
  H.notes = {}
  vim.cmd('TradarAddConnection')
end
local function listed()
  local out
  require('tradar.rpc').request('connections.list', nil, function(e, r) out = r end)
  H.wait(3000, function() return out ~= nil end)
  local by = {}
  for _, c in ipairs(out or {}) do by[c.name] = c end
  return by
end

-- ── happy path ──────────────────────────────────────────────────────────
local db = H.tmp .. '/wiz.db'
run({ 'sqlite' }, { 'wiz', db })
H.check('a connection is added and connected', H.wait(6000, function() return #H.notes_with('added `wiz`') == 1 end), vim.inspect(H.notes))
local text = file()
H.check('the existing file is untouched, the new table appended', text:sub(1, #original) == original and text:sub(#original + 1):find('[[connections]]', 1, true) ~= nil)
H.check('...with name, driver and target', text:find('name = "wiz"', 1, true) and text:find('driver = "sqlite"', 1, true) and text:find('target = "' .. db .. '"', 1, true), text)
local c = listed().wiz
H.check('the server sees it, connected', c and c.connected == true and c.target == db, vim.inspect(c))
H.check('the questions were: kind, name, target', #asked == 3 and asked[1].kind == 'select' and asked[2].default == 'sqlite-local', vim.inspect(asked))

-- ── name checks ─────────────────────────────────────────────────────────
run({ 'sqlite' }, { 'local', 'bad/name', 'ok_name', H.tmp .. '/ok.db' })
H.check('a taken name is asked again', H.wait(6000, function() return #H.notes_with('added `ok_name`') == 1 end), vim.inspect(H.notes))
H.check('...with the reason', #H.notes_with('already a connection called `local`') == 1, vim.inspect(H.notes))
H.check('a name with a slash is refused too', #H.notes_with('letters, digits') == 1, vim.inspect(H.notes))
run({ 'sqlite', 'Keep it anyway' }, { 'my local db', '/nonexistent-x/y.db' })
H.check('spaces between words are fine (the file allows them)', H.wait(6000, function() return #H.notes_with('kept `my local db`') == 1 end), vim.inspect(H.notes))

-- ── cancelling writes nothing ───────────────────────────────────────────
local before = file()
run({}, {})
H.sleep(200)
H.check('cancelling at the first question changes nothing', file() == before)
run({ 'sqlite' }, { nil })
H.sleep(200)
H.check('cancelling the name changes nothing', file() == before)
run({ 'sqlite' }, { 'nope' })
H.sleep(200)
H.check('cancelling the target changes nothing', file() == before)
run({ 'sqlite' }, { 'nope2', '   ' })
H.sleep(200)
H.check('an empty target is refused, nothing written', file() == before and #H.notes_with('target is needed') == 1, vim.inspect(H.notes))

-- ── a target that does not connect ──────────────────────────────────────
local bad = '/nonexistent-dir-for-tradar/x.db'
run({ 'sqlite', 'Remove it' }, { 'broken', bad })
H.check('a failing target asks what to do', H.wait(6000, function() return #asked >= 4 end), vim.inspect(asked))
H.check('...naming the error', asked[4] and asked[4].prompt:find('did not connect', 1, true), vim.inspect(asked))
H.check('Remove it puts the file back byte for byte', H.wait(3000, function() return file() == before end), file())
H.check('...and the server no longer lists it', listed().broken == nil)

local good = H.tmp .. '/fixed.db'
run({ 'sqlite', 'Edit the target' }, { 'fixme', bad, good })
H.check('Edit the target asks again, prefilled', H.wait(6000, function() return #H.notes_with('added `fixme`') == 1 end), vim.inspect(H.notes))
H.check('...with the target that failed', asked[5] and asked[5].default == bad, vim.inspect(asked))
local fixed = file()
H.check('only the fixed connection is in the file', select(2, fixed:gsub('name = "fixme"', '')) == 1 and fixed:find(good, 1, true) and not fixed:find(bad, 1, true), fixed)

run({ 'sqlite', 'Keep it anyway' }, { 'keeper', bad })
H.check('Keep it anyway keeps a connection that does not connect', H.wait(6000, function() return #H.notes_with('kept `keeper`') == 1 end), vim.inspect(H.notes))
H.check('...in the file', file():find('name = "keeper"', 1, true) ~= nil)

-- ── TOML escaping round-trips ───────────────────────────────────────────
local tricky = 'C:\\data\\"quoted"\\x.db'
run({ 'sqlite', 'Keep it anyway' }, { 'tricky', tricky })
H.wait(6000, function() return #H.notes_with('kept `tricky`') == 1 end)
H.check('backslashes and quotes survive the round trip through TOML', (listed().tricky or {}).target == tricky, vim.inspect(listed().tricky))

-- ── the connect picker offers it ────────────────────────────────────────
local offered
vim.ui.select = function(items, o, cb)
  offered = vim.tbl_map(o.format_item or tostring, items)
  cb(nil)
end
H.sql_buffer('wiz.sql', { '' })
t.connect()
H.check(':TradarConnect lists "+ add a connection…"', H.wait(3000, function() return offered ~= nil end) and vim.tbl_contains(offered, '+ add a connection…'), vim.inspect(offered))

-- ── first run: no connections at all starts the wizard ──────────────────
local f = io.open(conf, 'wb'); f:write('# nothing yet\n'); f:close()
selects, inputs, asked = {}, {}, {}
vim.ui.select = function(items, o, cb)
  asked[#asked + 1] = o.prompt
  cb(nil)
end
H.notes = {}
t.connect()
H.check('with no connections, :TradarConnect starts on adding one', H.wait(3000, function() return #asked > 0 and asked[1]:find('Add a connection', 1, true) ~= nil end), vim.inspect(asked))
H.done()
