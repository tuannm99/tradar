-- A small jq-style path for looking into a JSON response: `.data.items[0].id`,
-- `.items[].name`, `.["odd key"]`, `.[-1]`. Pure helpers; `results.lua` uses
-- it for `<leader>rF` on an HTTP / gRPC response.
local M = {}

--- `{ {kind='key', name}, {kind='index', n}, {kind='each'} }` for `path`, or
--- nil and a message.
function M.parse(path)
  local tokens, i, len = {}, 1, #path
  if path:sub(1, 1) == '.' and path:sub(2, 2) ~= '[' and #path == 1 then return tokens end
  while i <= len do
    local c = path:sub(i, i)
    if c == '.' then
      local nxt = path:sub(i + 1, i + 1)
      if nxt == '"' then
        local s, e = path:find('^"[^"]*"', i + 1)
        if not s then return nil, 'unterminated "…" in the path' end
        tokens[#tokens + 1] = { kind = 'key', name = path:sub(s + 1, e - 1) }
        i = e + 1
      elseif nxt == '[' then
        i = i + 1
      else
        local name = path:match('^[%w_%-%$@]+', i + 1)
        if not name then return nil, ('expected a key name after "." at column %d'):format(i) end
        tokens[#tokens + 1] = { kind = 'key', name = name }
        i = i + 1 + #name
      end
    elseif c == '[' then
      local inner, close = path:match('^%[(.-)%]()', i)
      if not inner then return nil, 'missing "]" in the path' end
      if inner == '' then
        tokens[#tokens + 1] = { kind = 'each' }
      elseif inner:match('^%-?%d+$') then
        tokens[#tokens + 1] = { kind = 'index', n = tonumber(inner) }
      elseif inner:match('^".*"$') then
        tokens[#tokens + 1] = { kind = 'key', name = inner:sub(2, -2) }
      else
        return nil, ('cannot read [%s] -- use [], [0], [-1] or ["key"]'):format(inner)
      end
      i = close
    else
      return nil, ('unexpected "%s" at column %d'):format(c, i)
    end
  end
  return tokens
end

local function is_array(t)
  return type(t) == 'table' and (#t > 0 or next(t) == nil) and vim.islist(t)
end

--- The values `path` selects in `value` (a list; `[]` fans out), or nil and why.
function M.query(value, path)
  local tokens, err = M.parse(path)
  if not tokens then return nil, err end
  local set = { value }
  for _, tok in ipairs(tokens) do
    local nextset = {}
    for _, v in ipairs(set) do
      if tok.kind == 'key' then
        if type(v) == 'table' and not is_array(v) and v[tok.name] ~= nil then nextset[#nextset + 1] = v[tok.name] end
      elseif tok.kind == 'index' then
        if is_array(v) then
          local at = tok.n >= 0 and tok.n + 1 or #v + tok.n + 1
          if v[at] ~= nil then nextset[#nextset + 1] = v[at] end
        end
      else
        if type(v) == 'table' then
          if is_array(v) then
            for _, item in ipairs(v) do nextset[#nextset + 1] = item end
          else
            local keys = vim.tbl_keys(v)
            table.sort(keys)
            for _, k in ipairs(keys) do nextset[#nextset + 1] = v[k] end
          end
        end
      end
    end
    set = nextset
    if #set == 0 then return nil, ('nothing at %s'):format(path) end
  end
  return set
end

return M
