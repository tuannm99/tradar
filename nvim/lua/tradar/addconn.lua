-- `:TradarAddConnection`: a few questions instead of editing connections.toml
-- by hand -- which driver, a name, the target (prefilled with that driver's
-- shape) -- then it connects to prove the target works, and offers to fix,
-- keep or drop it when it does not. The file is only ever appended to (its
-- comments and the other connections are untouched), and "drop it" puts the
-- original text back byte for byte.
local ctx = require('tradar.ctx')
local M = ctx.M
local notify = ctx.notify
local call = ctx.call

local PERCENT = 'special characters in a user name or password must be percent-encoded (@ → %40, : → %3A, / → %2F, # → %23, space → %20)'

--- id, what it is, the shape of a `target`, and a note shown with the question.
local DRIVERS = {
  { 'postgres', 'PostgreSQL', 'postgres://user:password@localhost:5432/dbname', PERCENT },
  { 'mysql', 'MySQL / MariaDB', 'mysql://user:password@localhost:3306/dbname', PERCENT },
  { 'sqlite', 'SQLite (a file)', '', 'path to the database file' },
  { 'mongo', 'MongoDB', 'mongodb://localhost:27017/dbname', 'the /dbname part is optional' },
  { 'elasticsearch', 'Elasticsearch', 'http://localhost:9200', nil },
  { 'redis', 'Redis', 'redis://localhost:6379/0', nil },
  { 'cassandra', 'Cassandra', '127.0.0.1:9042', 'host:port, not a URI' },
  { 'clickhouse', 'ClickHouse', 'http://user:password@127.0.0.1:8123/dbname', 'the HTTP port (8123), not the native 9000; ' .. PERCENT },
  { 'kafka', 'Kafka', 'localhost:9092', 'bootstrap servers: host:port[,host:port…]' },
  { 'rabbitmq', 'RabbitMQ (Management API)', 'http://guest:guest@localhost:15672/%2f', '%2f is the default vhost "/"' },
  { 'http', 'HTTP (requests)', 'https://api.example.com', 'a base URL; empty if you always type full URLs' },
  { 'socket', 'TCP socket', '127.0.0.1:9000', 'host:port' },
  { 'grpc', 'gRPC', 'localhost:50051', 'host:port, or https://… for TLS' },
}
M.connection_drivers = DRIVERS

-- Drivers that connect through a method of their own.
local OWN_CONNECT = { kafka = true, rabbitmq = true, http = true, grpc = true }

--- A TOML basic string for `s`.
local function toml_string(s)
  local escapes = { ['\\'] = '\\\\', ['"'] = '\\"', ['\n'] = '\\n', ['\r'] = '\\r', ['\t'] = '\\t' }
  return '"' .. s:gsub('[%c\\"]', function(c) return escapes[c] or ('\\u%04X'):format(c:byte()) end) .. '"'
end

local function block(name, driver, target)
  return ('\n[[connections]]\nname = %s\ndriver = %s\ntarget = %s\n'):format(toml_string(name), toml_string(driver), toml_string(target))
end

local function read_file(path)
  local f = io.open(path, 'rb')
  if not f then return nil end
  local text = f:read('*a')
  f:close()
  return text
end

local function write_file(path, text)
  vim.fn.mkdir(vim.fn.fnamemodify(path, ':h'), 'p')
  local f, err = io.open(path, 'wb')
  if not f then return nil, err end
  f:write(text)
  f:close()
  -- An open buffer of that file notices the change.
  pcall(vim.cmd, 'silent! checktime')
  return true
end

--- Proves `name` connects. `cb(ok, err)`.
local function try_connect(name, driver, cb)
  if driver == 'socket' then return cb(true) end -- it dials when you tail
  local method = OWN_CONNECT[driver] and (driver .. '.connect') or 'connect'
  ctx.state.connected[name] = nil
  call(method, { connection = name }, function()
    if not OWN_CONNECT[driver] then ctx.state.connected[name] = true end
    cb(true)
  end, function(err) cb(false, err) end)
end

local function ask_target(entry, default, cb)
  vim.ui.input({
    prompt = ('target (%s)%s: '):format(entry[2], entry[4] and (' — ' .. entry[4]) or ''),
    default = default or entry[3],
    completion = entry[1] == 'sqlite' and 'file' or nil,
  }, cb)
end

--- Appends the connection, tests it, and settles what to keep.
local function finish(path, original, name, entry, target)
  local text = original
  if text ~= '' and text:sub(-1) ~= '\n' then text = text .. '\n' end
  local ok, err = write_file(path, text .. block(name, entry[1], target))
  if not ok then return notify('could not write ' .. path .. ': ' .. tostring(err), vim.log.levels.ERROR) end
  try_connect(name, entry[1], function(connected, why)
    if connected then
      return notify(('added `%s` (%s)%s — :TradarConnect %s binds this buffer to it'):format(
        name, entry[1], entry[1] == 'socket' and '' or ' and connected', name))
    end
    vim.ui.select({ 'Edit the target', 'Keep it anyway', 'Remove it' }, {
      prompt = ('`%s` did not connect: %s'):format(name, tostring(why):sub(1, 160)),
    }, function(choice)
      if choice == 'Edit the target' then
        return ask_target(entry, target, function(new)
          if new == nil then return finish(path, original, name, entry, target) end
          finish(path, original, name, entry, new)
        end)
      elseif choice == 'Keep it anyway' then
        notify(('kept `%s` (not connected) — fix it with :TradarConnections'):format(name), vim.log.levels.WARN)
      else
        write_file(path, original)
        notify(('removed `%s` again; %s is as it was'):format(name, vim.fn.fnamemodify(path, ':~')))
      end
    end)
  end)
end

--- Names already taken, from the file the server reads.
local function taken(cb)
  call('connections.list', nil, function(list)
    local names = {}
    for _, c in ipairs(list) do names[c.name] = true end
    cb(names)
  end)
end

function M.add_connection()
  call('connections.path', nil, function(res)
    local path = res.path
    if not path then return notify('the server has no connections file', vim.log.levels.WARN) end
    local buf = vim.fn.bufnr(path)
    if buf ~= -1 and vim.bo[buf].modified then
      return notify('connections.toml has unsaved changes in a buffer — save it first', vim.log.levels.WARN)
    end
    taken(function(names)
      vim.ui.select(DRIVERS, {
        prompt = 'Add a connection — which kind?',
        format_item = function(d) return ('%-14s %s'):format(d[1], d[2]) end,
      }, function(entry)
        if not entry then return end
        local function ask_name(default)
          vim.ui.input({ prompt = 'name: ', default = default }, function(name)
            if name == nil then return end
            if not (name:match('^[%w_.%- ]+$') and name:match('^%S') and name:match('%S$')) then
              notify('a name is letters, digits, spaces between words, _ - .', vim.log.levels.WARN)
              return ask_name(name)
            end
            if names[name] then
              notify(('there is already a connection called `%s`'):format(name), vim.log.levels.WARN)
              return ask_name(name)
            end
            ask_target(entry, nil, function(target)
              if target == nil then return end
              target = vim.trim(target)
              if target == '' and entry[1] ~= 'http' then
                return notify('a target is needed', vim.log.levels.WARN)
              end
              finish(path, read_file(path) or '', name, entry, target)
            end)
          end)
        end
        ask_name(entry[1] .. '-local')
      end)
    end)
  end)
end

return M
