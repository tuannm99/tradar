-- The request workbench's text format, shared by HTTP and gRPC. Pure
-- helpers (no RPC, no buffers); `restbench.lua` owns the buffer and the runs.
--
--     @token = abc                 -- a variable, used as {{token}}
--
--     ### list users               -- starts a request block (title optional)
--     GET /users?limit=10          -- METHOD url   (a bare url is a GET)
--     Authorization: Bearer {{token}}
--
--     ### create
--     POST /users
--     Content-Type: application/json
--
--     {"name": "ann"}
--
--     ### get user
--     GRPC users.UserService/GetUser     -- GRPC service/Method
--     x-token: {{token}}                 -- metadata
--     deadline: 5s                       -- the call's deadline, not a header
--
--     {"id": "42"}
local M = {}

local HTTP_METHODS = { GET = true, POST = true, PUT = true, PATCH = true, DELETE = true, HEAD = true, OPTIONS = true }

local function is_separator(line) return line:match('^###') ~= nil end
local function is_comment(line) return line:match('^%s*#') ~= nil or line:match('^%s*//') ~= nil end
local function is_blank(line) return line:match('^%s*$') ~= nil end

--- `{ first, last, title }` for each block: `###` starts one (the line is its
--- first), and the text before the first `###` is a block of its own.
function M.blocks(lines)
  local out = {}
  local cur = { first = 1 }
  for i, line in ipairs(lines) do
    if is_separator(line) then
      local title = vim.trim(line:sub(4))
      if i > 1 then
        cur.last = i - 1
        out[#out + 1] = cur
        cur = { first = i }
      end
      cur.title = title ~= '' and title or nil
    end
  end
  cur.last = #lines
  out[#out + 1] = cur
  return out
end

--- The block that contains line `row`.
function M.block_at(lines, row)
  for _, b in ipairs(M.blocks(lines)) do
    if row >= b.first and row <= b.last then return b end
  end
end

--- Whether a block has a request line (not just variables or comments).
function M.has_request(lines, block)
  for i = block.first, block.last do
    local line = lines[i]
    if not (is_separator(line) or is_blank(line) or is_comment(line) or line:match('^@')) then return true end
  end
  return false
end

--- `@name = value` lines anywhere in the file: `name -> value`. A value may
--- use the variables defined above it.
function M.variables(lines)
  local vars = {}
  for _, line in ipairs(lines) do
    local name, value = line:match('^@([%w_%.%-]+)%s*=%s*(.-)%s*$')
    if name then vars[name] = M.expand(value, vars) or value end
  end
  return vars
end

local seeded = false
local function uuid()
  if not seeded then
    math.randomseed(os.time() + (vim.uv or vim.loop).hrtime() % 100000)
    seeded = true
  end
  local t = {}
  for i = 1, 16 do t[i] = math.random(0, 255) end
  t[7] = bit.bor(bit.band(t[7], 0x0f), 0x40)
  t[9] = bit.bor(bit.band(t[9], 0x3f), 0x80)
  local hex = {}
  for i = 1, 16 do hex[i] = ('%02x'):format(t[i]) end
  local s = table.concat(hex)
  return ('%s-%s-%s-%s-%s'):format(s:sub(1, 8), s:sub(9, 12), s:sub(13, 16), s:sub(17, 20), s:sub(21, 32))
end

local BUILTIN = {
  ['$uuid'] = uuid,
  ['$timestamp'] = function() return tostring(os.time()) end,
  ['$isoTimestamp'] = function() return os.date('!%Y-%m-%dT%H:%M:%SZ') end,
}

--- `text` with every `{{name}}` replaced. Names: a variable (`@name = …`),
--- `$uuid`, `$timestamp`, `$isoTimestamp`, `$env.NAME` (an environment
--- variable). Returns `nil, "undefined variable {{name}}"` for an unknown one.
function M.expand(text, vars)
  if not text:find('{{', 1, true) then return text end
  local missing
  local out = text:gsub('{{%s*([^{}]-)%s*}}', function(name)
    local builtin = BUILTIN[name]
    if builtin then return builtin() end
    local env = name:match('^%$env%.(.+)$')
    if env then
      local v = os.getenv(env)
      if v == nil then missing = missing or ('environment variable ' .. env .. ' is not set') end
      return v or ''
    end
    local v = vars and vars[name]
    if v == nil then
      missing = missing or ('undefined variable {{' .. name .. '}}')
      return ''
    end
    return v
  end)
  if missing then return nil, missing end
  return out
end

local function parse_deadline(text)
  local n, unit = text:match('^(%d+%.?%d*)%s*(%a*)$')
  if not n then return nil end
  local factor = ({ ms = 1, s = 1000, [''] = 1000, m = 60000 })[unit:lower()]
  return factor and math.floor(tonumber(n) * factor) or nil
end

--- The request line `GET /x`, `/x`, `GET /x HTTP/1.1` or `GRPC pkg.Svc/Method`
--- as `{ kind = 'http', method, url }` / `{ kind = 'grpc', service, method }`.
local function request_line(line)
  line = vim.trim(line)
  local target = line:match('^[Gg][Rr][Pp][Cc]%s+(%S+)$')
  if target then
    local service, method = target:match('^(.+)/([^/]+)$')
    if not service then return nil, 'GRPC needs service/Method, e.g. GRPC users.UserService/GetUser' end
    return { kind = 'grpc', service = service, method = method }
  end
  line = line:gsub('%s+HTTP/%d[%.%d]*$', '')
  local verb, url = line:match('^(%a+)%s+(%S.*)$')
  verb = verb and verb:upper()
  if not (verb and HTTP_METHODS[verb]) then verb, url = 'GET', line end
  if url == '' then return nil, 'no URL on the request line' end
  return { kind = 'http', method = verb, url = url }
end

--- The URL without scheme, host and query -- what names a request's tab.
local function path_of(url)
  local p = url:gsub('^%a[%w+.-]*://[^/]*', ''):gsub('[?#].*$', '')
  return p == '' and '/' or p
end

--- Parses `lines[block.first..block.last]` into a request:
---   http: { kind, method, url, headers = {{name, value}, ...}, body, title, line }
---   grpc: { kind, service, method, metadata = {name = value}, timeout_ms, body, title, line }
--- (`body` of a gRPC request is the decoded JSON.) Variables are expanded in
--- the request line, headers and body. Returns `nil, message` on a mistake.
function M.parse(lines, block, vars)
  vars = vars or M.variables(lines)
  local i, last = block.first, block.last
  local req
  while i <= last do
    local line = lines[i]
    if is_separator(line) or is_blank(line) or is_comment(line) or line:match('^@') then
      i = i + 1
    else
      local err
      local raw = line
      local expanded, why = M.expand(raw, vars)
      if not expanded then return nil, ('line %d: %s'):format(i, why) end
      req, err = request_line(expanded)
      if not req then return nil, ('line %d: %s'):format(i, err) end
      req.line = i
      i = i + 1
      break
    end
  end
  if not req then return nil, 'no request in this block -- start it with `GET /path` or `GRPC service/Method`' end
  req.title = block.title

  -- Header lines run to the first blank line.
  local headers, metadata = {}, {}
  while i <= last and not is_blank(lines[i]) do
    local line = lines[i]
    if not is_comment(line) then
      local name, value = line:match("^%s*([%w!#%$%%&'%*%+%.%^_`|~%-]+)%s*:%s*(.-)%s*$")
      if not name then
        return nil, ('line %d: expected `Name: value` -- put a blank line between the headers and the body'):format(i)
      end
      local expanded, why = M.expand(value, vars)
      if not expanded then return nil, ('line %d: %s'):format(i, why) end
      if req.kind == 'grpc' then
        if name:lower() == 'deadline' then
          req.timeout_ms = parse_deadline(expanded)
          if not req.timeout_ms or req.timeout_ms <= 0 then
            return nil, ('line %d: bad deadline "%s" -- use e.g. 5s, 500ms, 2m'):format(i, expanded)
          end
        else
          metadata[name:lower()] = expanded
        end
      else
        headers[#headers + 1] = { name, expanded }
      end
    end
    i = i + 1
  end

  -- Everything after that blank line is the body, minus trailing blank lines.
  local body = {}
  for j = i + 1, last do
    if not (req.kind == 'grpc' and lines[j]:match('^%s*//')) then body[#body + 1] = lines[j] end
  end
  while #body > 0 and is_blank(body[#body]) do body[#body] = nil end
  local text, why = M.expand(table.concat(body, '\n'), vars)
  if not text then return nil, why end

  if req.kind == 'http' then
    req.headers, req.body = headers, text
    return req
  end
  req.metadata = next(metadata) and metadata or nil
  if text:find('%S') then
    local ok, decoded = pcall(vim.json.decode, text, { luanil = { object = true, array = true } })
    if not ok then return nil, 'request body is not valid JSON: ' .. tostring(decoded) end
    req.body = decoded
  else
    req.body = vim.empty_dict()
  end
  return req
end

--- `key, label, query` of the result tab a request fills: the same method and
--- path (GET /users and GET /users?limit=5) share one tab.
function M.slot(req, conn)
  if req.kind == 'grpc' then
    local key = ('%s\ng:%s/%s'):format(conn or '', req.service, req.method)
    return key, req.method, 'GRPC ' .. req.service .. '/' .. req.method
  end
  local path = path_of(req.url)
  return ('%s\nh:%s %s'):format(conn or '', req.method, path), req.method .. ' ' .. path, req.method .. ' ' .. req.url
end

local function shq(s) return "'" .. tostring(s):gsub("'", [['\'']]) .. "'" end

--- The full URL of an HTTP request on a connection whose base is `base`.
function M.full_url(base, url)
  if url:match('^https?://') or not base or base == '' then return url end
  return base:gsub('/+$', '') .. '/' .. url:gsub('^/+', '')
end

--- A `curl` command for an HTTP request.
function M.as_curl(req, base)
  local parts = { 'curl' }
  if req.method ~= 'GET' then parts[#parts + 1] = '-X ' .. req.method end
  parts[#parts + 1] = shq(M.full_url(base, req.url))
  for _, h in ipairs(req.headers or {}) do parts[#parts + 1] = '-H ' .. shq(h[1] .. ': ' .. h[2]) end
  if req.body and req.body:find('%S') then parts[#parts + 1] = '--data-raw ' .. shq(req.body) end
  return table.concat(parts, ' \\\n  ')
end

--- A `grpcurl` command for a gRPC request on a connection whose target is
--- `target` (`host:port`, or `https://host:port` for TLS).
function M.as_grpcurl(req, target)
  local host = (target or ''):gsub('^https?://', ''):gsub('[?].*$', '')
  local parts = { 'grpcurl' }
  if not (target or ''):match('^https://') then parts[#parts + 1] = '-plaintext' end
  for k, v in pairs(req.metadata or {}) do parts[#parts + 1] = '-H ' .. shq(k .. ': ' .. v) end
  if req.timeout_ms then parts[#parts + 1] = '-max-time ' .. tostring(req.timeout_ms / 1000) end
  parts[#parts + 1] = '-d ' .. shq(vim.json.encode(req.body or vim.empty_dict()))
  parts[#parts + 1] = host
  parts[#parts + 1] = req.service .. '/' .. req.method
  return table.concat(parts, ' \\\n  ')
end

return M
