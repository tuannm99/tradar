-- Colours for request text (HTTP / gRPC): the `###` lines, `@variables`, the
-- request line, headers, `{{placeholders}}` and a JSON body. Set as extmarks
-- by a line scanner, like the JSON view (`jsonhl.lua`), so it needs no
-- tree-sitter grammar for `http` and works the same in a workbench, a `.http`
-- file and the ```http / ```grpc blocks of a `.tdb`.
local reqparse = require('tradar.reqparse')
local jsonhl = require('tradar.jsonhl')

local M = {}

local ns = vim.api.nvim_create_namespace('tradar_request_colours')

--- Default links; `highlight TradarReqMethod ...` in a colorscheme overrides.
function M.define()
  for name, link in pairs({
    TradarReqSeparator = 'Title',
    TradarReqComment = 'Comment',
    TradarReqVarDef = 'Identifier',
    TradarReqVar = 'Special',
    TradarReqMethod = 'Keyword',
    TradarReqUrl = 'Underlined',
    TradarReqService = 'Type',
    TradarReqRpc = 'Function',
    TradarReqHeader = 'Label',
    TradarReqValue = 'String',
    TradarReqDeadline = 'Special',
  }) do
    vim.api.nvim_set_hl(0, name, { link = link, default = true })
  end
end

local VERBS = { GET = true, POST = true, PUT = true, PATCH = true, DELETE = true, HEAD = true, OPTIONS = true }

local function is_comment(line) return line:match('^%s*#') ~= nil or line:match('^%s*//') ~= nil end

--- `{ { col_start, col_end, group }, ... }` (0-based, end exclusive) for the
--- request line `line`.
local function request_tokens(line)
  local out = {}
  local lead = line:match('^%s*')
  local word, gap, rest = line:match('^%s*(%a+)(%s+)(.*)$')
  if word and word:upper() == 'GRPC' then
    out[#out + 1] = { #lead, #lead + #word, 'TradarReqMethod' }
    local at = #lead + #word + #gap
    local first, between, second = rest:match('^(%S+)(%s+)(%S+)')
    local path = second or rest:match('^(%S+)')
    local path_at = at
    if second then
      out[#out + 1] = { at, at + #first, 'TradarReqUrl' }
      path_at = at + #first + #between
    end
    local service, method = (path or ''):match('^(.+)/([^/]+)$')
    if service then
      out[#out + 1] = { path_at, path_at + #service, 'TradarReqService' }
      out[#out + 1] = { path_at + #service + 1, path_at + #path, 'TradarReqRpc' }
    elseif path then
      out[#out + 1] = { path_at, path_at + #path, 'TradarReqService' }
    end
    return out
  end
  local url_at
  if word and VERBS[word:upper()] then
    out[#out + 1] = { #lead, #lead + #word, 'TradarReqMethod' }
    url_at = #lead + #word + #gap
  else
    url_at = #lead
  end
  local url = line:sub(url_at + 1)
  local version = url:match('%s+HTTP/%d[%.%d]*%s*$')
  if version then
    out[#out + 1] = { url_at + #url - #version, #line, 'TradarReqComment' }
    url = url:sub(1, #url - #version)
  end
  if #url > 0 then out[#out + 1] = { url_at, url_at + #url, 'TradarReqUrl' } end
  return out
end

--- `{{name}}` placeholders anywhere in `line`.
local function placeholders(line)
  local out = {}
  local pos = 1
  while true do
    local s, e = line:find('{{[^{}]-}}', pos)
    if not s then break end
    out[#out + 1] = { s - 1, e, 'TradarReqVar' }
    pos = e + 1
  end
  return out
end

--- Colours for a text of requests: `lines` -> `{ [row] = tokens }` (1-based rows
--- of `lines`; a row without any has no entry). `placeholders` are marked
--- after (above) the rest.
function M.tokens(lines)
  local rows = {}
  for _, block in ipairs(reqparse.blocks(lines)) do
    local state = 'start' -- start -> headers -> body
    local grpc, json = false, nil
    for i = block.first, block.last do
      local line = lines[i]
      local t = {}
      local blank = line:match('^%s*$') ~= nil
      if line:match('^###') then
        t[1] = { 0, #line, 'TradarReqSeparator' }
        state, grpc, json = 'start', false, nil
      elseif state == 'start' then
        if blank then
          -- nothing
        elseif is_comment(line) then
          t[1] = { 0, #line, 'TradarReqComment' }
        elseif line:match('^@') then
          local name_end = line:find('%s*=') or (#line + 1)
          t[1] = { 0, name_end - 1, 'TradarReqVarDef' }
          local value_at = select(2, line:find('^@[%w_%.%-]+%s*=%s*'))
          if value_at and value_at < #line then t[2] = { value_at, #line, 'TradarReqValue' } end
        else
          t = request_tokens(line)
          grpc = line:match('^%s*[Gg][Rr][Pp][Cc]%s') ~= nil
          state = 'headers'
        end
      elseif state == 'headers' then
        if blank then
          state = 'body'
        elseif is_comment(line) then
          t[1] = { 0, #line, 'TradarReqComment' }
        else
          local lead, name, colon = line:match('^(%s*)([%w!#%$%%&\'%*%+%.%^_`|~%-]+)(%s*:%s*)')
          if name then
            local special = grpc and name:lower() == 'deadline'
            t[1] = { #lead, #lead + #name, special and 'TradarReqDeadline' or 'TradarReqHeader' }
            if #line > #lead + #name + #colon then t[2] = { #lead + #name + #colon, #line, 'TradarReqValue' } end
          end
        end
      else -- body
        if grpc and line:match('^%s*//') then
          t[1] = { 0, #line, 'TradarReqComment' }
        elseif not blank then
          if json == nil then
            local first = line:match('^%s*(.)')
            json = grpc or first == '{' or first == '[' or first == '"'
          end
          if json then t = jsonhl.tokens(line) end
        end
      end
      if not (line:match('^###') or (is_comment(line) and state ~= 'body')) then
        for _, p in ipairs(placeholders(line)) do t[#t + 1] = p end
      end
      if #t > 0 then rows[i] = t end
    end
  end
  return rows
end

local MAX_LINES = 20000
local PLACEHOLDER = 'TradarReqVar'

--- Colours `buf`: all of it for a workbench / `.http` file, the ```http and
--- ```grpc blocks only for a `.tdb`. Replaces the previous colours.
function M.paint(buf)
  if not vim.api.nvim_buf_is_valid(buf) then return end
  M.define()
  vim.api.nvim_buf_clear_namespace(buf, ns, 0, -1)
  local all = vim.api.nvim_buf_get_lines(buf, 0, MAX_LINES, false)
  local function put(rows, offset)
    for row, tokens in pairs(rows) do
      for _, tk in ipairs(tokens) do
        pcall(vim.api.nvim_buf_set_extmark, buf, ns, row - 1 + offset, tk[1], {
          end_col = tk[2],
          hl_group = tk[3],
          priority = tk[3] == PLACEHOLDER and 200 or 100,
        })
      end
    end
  end
  if require('tradar.ctx').M.is_tdb(buf) then
    for _, block in ipairs(require('tradar.blocks').parse(all)) do
      local lang = block.lang and block.lang:lower()
      if lang == 'http' or lang == 'grpc' then
        put(M.tokens(vim.list_slice(all, block.start, block.finish)), block.start - 1)
      end
    end
  else
    put(M.tokens(all), 0)
  end
end

--- Keeps `buf` coloured as it is edited (repainted shortly after a change).
function M.attach(buf)
  if vim.b[buf].tradar_reqhl then return end
  vim.b[buf].tradar_reqhl = true
  local timer = (vim.uv or vim.loop).new_timer()
  local function later()
    timer:stop()
    timer:start(30, 0, vim.schedule_wrap(function() M.paint(buf) end))
  end
  vim.api.nvim_create_autocmd({ 'TextChanged', 'TextChangedI', 'TextChangedP' }, { buffer = buf, callback = later })
  vim.api.nvim_create_autocmd('BufWipeout', {
    buffer = buf,
    once = true,
    callback = function()
      timer:stop()
      if not timer:is_closing() then timer:close() end
    end,
  })
  M.paint(buf)
end

M.ns = ns

return M
