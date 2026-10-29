-- Endpoints of an OpenAPI 3 / Swagger 2 document as request blocks for the
-- workbench. Pure helpers: `restbench.lua` fetches the document and inserts
-- what this builds.
local M = {}

local METHODS = { 'get', 'put', 'post', 'delete', 'patch', 'head', 'options' }

--- The thing `ref` (`#/components/schemas/User`) points at, or nil.
local function resolve(doc, ref)
  if type(ref) ~= 'string' then return nil end
  local node = doc
  for part in ref:gsub('^#/', ''):gmatch('[^/]+') do
    part = part:gsub('~1', '/'):gsub('~0', '~')
    if type(node) ~= 'table' then return nil end
    node = node[part]
  end
  return node
end

local function deref(doc, node, depth)
  depth = depth or 0
  while type(node) == 'table' and node['$ref'] and depth < 10 do
    node = resolve(doc, node['$ref'])
    depth = depth + 1
  end
  return node
end

--- A JSON value that satisfies `schema`, with every property at a plain
--- default (`example`/`default`/first `enum` value when the schema gives one).
--- Nested objects stop after a few levels (so a schema that contains itself
--- ends in `{}`).
function M.sample(doc, schema, depth)
  schema = deref(doc, schema)
  if type(schema) ~= 'table' then return vim.NIL end
  depth = depth or 0
  if schema.example ~= nil then return schema.example end
  if schema.default ~= nil then return schema.default end
  if schema.enum and schema.enum[1] ~= nil then return schema.enum[1] end
  if schema.allOf then
    local merged = {}
    for _, part in ipairs(schema.allOf) do
      local v = M.sample(doc, part, depth + 1)
      if type(v) == 'table' then merged = vim.tbl_extend('force', merged, v) end
    end
    return merged
  end
  local alt = (schema.oneOf and schema.oneOf[1]) or (schema.anyOf and schema.anyOf[1])
  if alt then return M.sample(doc, alt, depth + 1) end
  local kind = schema.type
  if kind == nil and schema.properties then kind = 'object' end
  if kind == 'object' then
    if depth > 4 then return vim.empty_dict() end
    local out = {}
    local names = vim.tbl_keys(schema.properties or {})
    table.sort(names)
    for _, name in ipairs(names) do
      out[name] = M.sample(doc, schema.properties[name], depth + 1)
    end
    return next(out) and out or vim.empty_dict()
  elseif kind == 'array' then
    if depth > 4 then return {} end
    return { M.sample(doc, schema.items, depth + 1) }
  elseif kind == 'integer' or kind == 'number' then
    return 0
  elseif kind == 'boolean' then
    return false
  elseif kind == 'string' then
    if schema.format == 'date-time' then return '1970-01-01T00:00:00Z' end
    if schema.format == 'date' then return '1970-01-01' end
    if schema.format == 'uuid' then return '00000000-0000-0000-0000-000000000000' end
    return ''
  end
  return vim.NIL
end

--- Every operation of `doc`: `{ method = 'GET', path, summary, operation, params }`,
--- ordered by path then method. `params` are the path-level and operation-level
--- parameters, `$ref`s resolved.
function M.endpoints(doc)
  local out = {}
  if type(doc) ~= 'table' or type(doc.paths) ~= 'table' then return out end
  local paths = vim.tbl_keys(doc.paths)
  table.sort(paths)
  for _, path in ipairs(paths) do
    local item = doc.paths[path]
    for _, method in ipairs(METHODS) do
      local op = type(item) == 'table' and item[method]
      if type(op) == 'table' then
        local params = {}
        for _, list in ipairs({ item.parameters or {}, op.parameters or {} }) do
          for _, p in ipairs(list) do
            p = deref(doc, p)
            if type(p) == 'table' and p.name then params[#params + 1] = p end
          end
        end
        out[#out + 1] = {
          method = method:upper(),
          path = path,
          summary = op.summary or op.operationId or '',
          operation = op,
          params = params,
        }
      end
    end
  end
  return out
end

--- The JSON schema of an operation's request body (OpenAPI 3 `requestBody`,
--- Swagger 2 `in: body` parameter), or nil.
local function body_schema(doc, ep)
  local rb = deref(doc, ep.operation.requestBody)
  if type(rb) == 'table' and type(rb.content) == 'table' then
    for _, media in ipairs({ 'application/json', 'application/*+json' }) do
      if rb.content[media] then return rb.content[media].schema, true end
    end
    local _, any = next(rb.content)
    return any and any.schema, false
  end
  for _, p in ipairs(ep.params) do
    if p['in'] == 'body' then return p.schema, true end
  end
end

--- Request block lines for `ep`: `@x = ` for each path parameter (fill it in
--- above or in `setup{ request_vars }`), `{x}` in the path turned into
--- `{{x}}`, required query parameters as `?a=&b=`, a JSON `Content-Type` and
--- a sample body when the operation takes one.
function M.block(doc, ep)
  local lines = { ('### %s %s%s'):format(ep.method, ep.path, ep.summary ~= '' and (' — ' .. ep.summary) or '') }
  local path = ep.path:gsub('{([^{}]+)}', function(name)
    lines[#lines + 1] = ('@%s = '):format(name)
    return '{{' .. name .. '}}'
  end)
  local query = {}
  for _, p in ipairs(ep.params) do
    if p['in'] == 'query' and p.required then query[#query + 1] = p.name .. '=' end
  end
  lines[#lines + 1] = ('%s %s%s'):format(ep.method, path, #query > 0 and ('?' .. table.concat(query, '&')) or '')
  for _, p in ipairs(ep.params) do
    if p['in'] == 'header' and p.required then lines[#lines + 1] = p.name .. ': ' end
  end
  local schema, is_json = body_schema(doc, ep)
  if schema ~= nil then
    if is_json then
      lines[#lines + 1] = 'Content-Type: application/json'
      lines[#lines + 1] = ''
      local sample = M.sample(doc, schema)
      local text = require('tradar.render').pretty(sample == vim.NIL and vim.empty_dict() or sample)
      vim.list_extend(lines, vim.split(text, '\n', { plain = true }))
    else
      lines[#lines + 1] = ''
    end
  end
  return lines
end

--- Paths where a document is usually served, tried in this order.
M.CANDIDATES = { '/openapi.json', '/swagger.json', '/v3/api-docs', '/v2/api-docs', '/api-docs', '/docs/openapi.json' }

return M
