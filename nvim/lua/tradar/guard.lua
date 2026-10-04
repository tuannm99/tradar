-- Decides whether a statement deserves a "are you sure?" before it runs.
-- Pure text analysis (no server, no buffers) so it is trivially testable.
-- A heuristic on tokens, not a parser: it only has to catch the usual slips
-- (a forgotten WHERE, a DROP on the wrong connection), and a miss just means
-- no prompt -- the same trade the drivers' own `returns_rows` makes.
local M = {}

local WRITES = {
  insert = true, update = true, delete = true, merge = true, replace = true,
  create = true, alter = true, drop = true, truncate = true, rename = true,
  grant = true, revoke = true,
}

--- `text` with comments and string/identifier literals blanked out, so a
--- `WHERE` inside a string or a comment can't fool the checks below.
local function scrub(text)
  local out, i, n = {}, 1, #text
  while i <= n do
    local two = text:sub(i, i + 1)
    local c = text:sub(i, i)
    if two == '--' then
      local e = text:find('\n', i, true) or n + 1
      out[#out + 1] = ' '
      i = e
    elseif two == '/*' then
      local _, e = text:find('*/', i + 2, true)
      out[#out + 1] = ' '
      i = (e or n) + 1
    elseif c == "'" or c == '"' or c == '`' then
      local j = i + 1
      while j <= n do
        if text:sub(j, j) == c then
          if text:sub(j + 1, j + 1) == c then j = j + 2 else break end -- doubled quote = escaped
        else
          j = j + 1
        end
      end
      out[#out + 1] = ' '
      i = j + 1
    else
      out[#out + 1] = c
      i = i + 1
    end
  end
  return table.concat(out):lower()
end

local function words(clean)
  local list = {}
  for w in clean:gmatch('[%a_][%w_]*') do list[#list + 1] = w end
  return list
end

--- Whether `name` matches one of the `protected` patterns (plain,
--- case-insensitive substring: "prod" protects "prod-main" and "myprod").
function M.is_protected(name, protected)
  if not name then return false end
  local lower = name:lower()
  for _, p in ipairs(protected or {}) do
    if lower:find(p:lower(), 1, true) then return true end
  end
  return false
end

--- Whether `text` holds nothing but comments (`--`, `//`, `#`) and blank
--- lines. A file's `-- tradar: name` binding line is one of these, and the
--- line-oriented drivers (Mongo, Redis, Elasticsearch) split it out as a
--- statement of its own -- running it would be an error, not a no-op.
function M.is_comment_only(text)
  for line in text:gmatch('[^\n]+') do
    local t = vim.trim(line)
    if t ~= '' and t:sub(1, 2) ~= '--' and t:sub(1, 2) ~= '//' and t:sub(1, 1) ~= '#' then return false end
  end
  return true
end

--- Whether `text` changes the schema (so cached table/column info is stale).
function M.changes_schema(text)
  local first = words(scrub(text))[1]
  return first == 'create' or first == 'alter' or first == 'drop' or first == 'rename'
end

--- First line of `text` that is real code, not blank or a `--` comment --
--- what a prompt should quote, rather than the modeline above the statement.
function M.first_code_line(text)
  for line in text:gmatch('[^\n]+') do
    local t = vim.trim(line)
    if t ~= '' and t:sub(1, 2) ~= '--' then return t end
  end
  return vim.trim(text:match('[^\n]*') or '')
end

-- ── non-SQL languages ────────────────────────────────────────────────────

local MONGO_WRITES = {
  insertone = true, insertmany = true, updateone = true, updatemany = true, replaceone = true,
  deleteone = true, deletemany = true, drop = true, dropdatabase = true, createcollection = true,
  createindex = true, dropindex = true, dropindexes = true, renamecollection = true, bulkwrite = true,
  findoneandupdate = true, findoneanddelete = true, findoneandreplace = true,
  insert = true, update = true, remove = true, save = true,
}

local function assess_mongo(text, protected)
  local clean = scrub(text)
  -- The method called on the collection: db.users.deleteMany(...) -> deletemany
  local method = clean:match('%.%s*([%a_]+)%s*%(')
  local calls = {}
  for m in clean:gmatch('%.%s*([%a_]+)%s*%(') do calls[#calls + 1] = m end
  for _, m in ipairs(calls) do
    if m == 'drop' or m == 'dropdatabase' then
      return ('%s is destructive and cannot be undone'):format(m:upper())
    end
    -- deleteMany({}) / updateMany({}, ...) / remove({}) with an empty filter hit every document
    if (m == 'deletemany' or m == 'updatemany' or m == 'remove')
        and clean:find(m .. '%s*%(%s*{%s*}') then
      return ('%s with an empty filter changes every document'):format(m)
    end
  end
  for _, m in ipairs(calls) do
    if protected and MONGO_WRITES[m] then return ('%s on a protected connection'):format(m) end
  end
  return nil, method
end

local REDIS_DESTRUCTIVE = { flushall = true, flushdb = true, shutdown = true }
local REDIS_WRITES = {
  set = true, setex = true, psetex = true, setnx = true, mset = true, msetnx = true, getset = true,
  getdel = true, append = true, setrange = true, del = true, unlink = true, expire = true, pexpire = true,
  persist = true, rename = true, renamenx = true, incr = true, incrby = true, incrbyfloat = true,
  decr = true, decrby = true, hset = true, hmset = true, hdel = true, hsetnx = true, hincrby = true,
  lpush = true, rpush = true, lpop = true, rpop = true, lset = true, lrem = true, ltrim = true, linsert = true,
  sadd = true, srem = true, spop = true, smove = true, zadd = true, zrem = true, zincrby = true,
  zremrangebyrank = true, zremrangebyscore = true, xadd = true, xdel = true, xtrim = true,
  eval = true, evalsha = true, config = true, publish = true, move = true, copy = true,
}

local function assess_redis(text, protected)
  -- One command per line; the riskiest line decides.
  local worst
  for line in text:gmatch('[^\n]+') do
    local cmd = line:match('^%s*([%a_]+)')
    cmd = cmd and cmd:lower()
    if cmd then
      if REDIS_DESTRUCTIVE[cmd] then
        return ('%s is destructive and cannot be undone'):format(cmd:upper())
      end
      if protected and REDIS_WRITES[cmd] then worst = worst or ('%s on a protected connection'):format(cmd:upper()) end
    end
  end
  return worst
end

local ES_READ_ENDPOINTS = { '_search', '_count', '_msearch', '_mget', '_analyze', '_validate', '_explain', '_field_caps', '_mapping', '_settings', '_cat', '_cluster', '_nodes', '_resolve' }

local function assess_elasticsearch(text, protected)
  local worst
  for line in text:gmatch('[^\n]+') do
    local method, path = line:match('^%s*(%u+)%s+(%S+)')
    if method == 'GET' or method == 'POST' or method == 'PUT' or method == 'DELETE' or method == 'PATCH' or method == 'HEAD' then
      local lower = path:lower()
      if lower:find('_delete_by_query', 1, true) then
        return 'DELETE BY QUERY removes every matching document'
      end
      if method == 'DELETE' then
        return ('DELETE %s is destructive and cannot be undone'):format(path:sub(1, 40))
      end
      if protected and (method == 'PUT' or method == 'POST' or method == 'PATCH') then
        local read = false
        for _, e in ipairs(ES_READ_ENDPOINTS) do
          if lower:find(e, 1, true) then read = true break end
        end
        if not read then worst = worst or ('%s on a protected connection'):format(method) end
      end
    end
  end
  return worst
end

--- A reason string when `text` should be confirmed, else nil.
--- `protected`: the connection is a protected one -- any write asks.
--- `driver`: the connector id; Mongo, Redis and Elasticsearch have their own
--- languages, everything else is read as SQL.
function M.assess(text, protected, driver)
  if driver == 'mongo' then return (assess_mongo(text, protected)) end
  if driver == 'redis' then return assess_redis(text, protected) end
  if driver == 'elasticsearch' then return assess_elasticsearch(text, protected) end
  local ws = words(scrub(text))
  local first = ws[1]
  if not first then return nil end

  local set = {}
  for _, w in ipairs(ws) do set[w] = true end

  -- A CTE can hide a write behind `WITH`: look for a DML verb anywhere.
  local verb = first
  if first == 'with' then
    for _, w in ipairs(ws) do
      if w == 'update' or w == 'delete' or w == 'insert' then verb = w break end
    end
  end

  if (verb == 'update' or verb == 'delete') and not set['where'] then
    return ('%s without WHERE changes every row'):format(verb:upper())
  end
  if verb == 'drop' or verb == 'truncate' then
    return ('%s is destructive and cannot be undone'):format(verb:upper())
  end
  if protected and WRITES[verb] then
    return ('%s on a protected connection'):format(verb:upper())
  end
  return nil
end

return M
