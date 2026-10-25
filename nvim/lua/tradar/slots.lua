-- Result slots: one kept result per *target* (a table, a collection, an
-- index, a Redis key), shown as tabs on the results window. Running another
-- query on the same target replaces that slot instead of pushing the others
-- away. Pure helpers only -- `results.lua` owns the state.
local M = {}

local REDIS_KEYED = {
  GET = true, GETEX = true, HGETALL = true, HGET = true, HKEYS = true, HVALS = true, HMGET = true,
  LRANGE = true, LLEN = true, LINDEX = true, SMEMBERS = true, SCARD = true, SISMEMBER = true,
  ZRANGE = true, ZREVRANGE = true, ZRANGEBYSCORE = true, ZCARD = true, ZSCORE = true,
  XRANGE = true, XREVRANGE = true, XLEN = true, TYPE = true, TTL = true, PTTL = true, STRLEN = true,
}

--- `query` without its leading blank space and `--`, `#` and `/* */` comments.
local function strip_leading(query)
  local q = query
  while true do
    local before = q
    q = q:gsub('^%s+', '')
    q = q:gsub('^%-%-[^\n]*', '')
    q = q:gsub('^//[^\n]*', '')
    q = q:gsub('^/%*.-%*/', '')
    if q == before then return q end
  end
end

local function unquote(name)
  return (name:gsub('^["`%[\']', ''):gsub('["`%]\']$', ''))
end

--- The table / collection / index / key a statement reads, or nil when it
--- has no single obvious one (`EXPLAIN`, `SHOW`, a join over a subquery, an
--- admin command...). Text patterns on purpose: the answer is needed before
--- the statement runs, for any driver, with no round trip.
function M.target_of(query)
  local q = strip_leading(query or '')
  if q == '' then return nil end

  -- Mongo shell: db.getCollection("users").find(...) / db.users.find(...)
  local coll = q:match('^db%.getCollection%(%s*["\']([^"\']+)["\']')
  if coll then return coll end
  coll = q:match('^db%.([%a_$][%w_$]*)%.[%a_]+%s*%(')
  if coll then return coll end

  -- Elasticsearch: GET /logs/_search, POST logs/_doc. A plain `GET key` is
  -- Redis: only a GET with a leading slash or an `_endpoint` is Elasticsearch.
  local verb, rest_of = q:match('^(%u+)%s+(%S+)')
  local slash = rest_of and rest_of:sub(1, 1) == '/' and '/' or ''
  local path = rest_of and rest_of:sub(#slash + 1):match('^[^?]*') or ''
  if verb and (verb == 'GET' or verb == 'POST' or verb == 'PUT' or verb == 'DELETE' or verb == 'HEAD') then
    local es = verb ~= 'GET' or slash == '/' or path:find('/_', 1, true) or path:sub(1, 1) == '_'
    if es then
      local index = path:match('^([^/]+)')
      if index and index:match('^[%w][%w_.,*%-]*$') then return index end
      return nil
    end
  end

  -- Redis: GET key / HGETALL key / LRANGE key 0 -1
  local cmd, rest = q:match('^(%a+)%s+(.+)$')
  if cmd and REDIS_KEYED[cmd:upper()] then
    if rest:sub(1, 1) == '"' then
      -- A key with spaces or quotes, as the Redis connector writes it: "a \"b\"".
      local out, i = {}, 2
      while i <= #rest do
        local c = rest:sub(i, i)
        if c == '\\' then
          out[#out + 1] = rest:sub(i + 1, i + 1)
          i = i + 2
        elseif c == '"' then
          return table.concat(out)
        else
          out[#out + 1] = c
          i = i + 1
        end
      end
      return table.concat(out)
    end
    return unquote(rest:match('^(%S+)'))
  end

  -- SQL: the first table of a SELECT (a join is shown under its first table).
  local lower = q:lower()
  if lower:match('^%(?%s*select%f[%W]') then
    local s, e = lower:find('%f[%w_]from%s+[%w_."`%[%]]+')
    if s then
      local name = q:sub(s, e):match('^%a+%s+(.+)$'):gsub('["`%[%]]', '')
      if name ~= '' then return name end
    end
  end
  return nil
end

local function squash(text, max)
  text = (text or ''):gsub('%s+', ' '):gsub('^ ', ''):gsub(' $', '')
  if vim.fn.strchars(text) > max then text = vim.fn.strcharpart(text, 0, max - 1) .. '…' end
  return text
end

--- What identifies a slot: the connection plus the target -- or, for a
--- statement without one, the statement itself (running it again replaces it).
function M.key(conn, query)
  local target = M.target_of(query)
  return (conn or '') .. '\n' .. (target and ('t:' .. target) or ('q:' .. squash(query, 500)))
end

--- Short name for a tab.
function M.label(query)
  return M.target_of(query) or squash(query, 24)
end

local function tab_text(i, e)
  local text = (' %d %s%s'):format(i, e.pinned and '📌' or '', squash(e.label or e.query, 18))
  if e.kind == 'affected' then
    text = text .. (' ✓%d'):format(e.rows or 0)
  elseif e.total then
    text = text .. (' (%d)'):format(e.total)
  end
  text = text .. ' '
  return (text:gsub('%%', '%%%%'))
end

--- The `winbar` of the results window: one clickable tab per slot, the
--- current one highlighted. Empty with fewer than two slots -- one result
--- needs no tabs. Clicking a tab calls `_G.__tradar_result_click`.
function M.bar(log, pos)
  if not log or #log < 2 then return '' end
  local parts = {}
  for i, e in ipairs(log) do
    parts[#parts + 1] = ('%%%d@v:lua.__tradar_result_click@%s%s%%X'):format(i, i == pos and '%#TabLineSel#' or '%#TabLine#', tab_text(i, e))
  end
  parts[#parts + 1] = '%#TabLineFill#'
  return table.concat(parts, '')
end

return M
