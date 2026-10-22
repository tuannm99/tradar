-- Command-line helpers: completing saved connection names for the
-- `:Tradar…` commands, and `:Tradar <subcommand>`, one entry point over all
-- of them (`:Tradar kafka tail` = `:TradarKafkaTail`).
local M = {}

-- Drivers served by their own `<driver>.*` commands rather than as a query
-- connection (`:TradarConnect`).
local NON_QUERY = { kafka = true, rabbitmq = true, http = true, socket = true, grpc = true }

--- Saved connections as `{ name, driver }`: from the running server when
--- there is one (it knows its own `connections.toml`), else read straight
--- from the file -- completing a name must not start a server.
function M.saved()
  local rpc = require('tradar.rpc')
  if rpc.connected() then
    local err, list = rpc.request_sync('connections.list', nil, 300)
    if not err and type(list) == 'table' then return list end
  end
  local path = vim.fn.stdpath('config'):gsub('[^/]+$', '') .. 'tradar/connections.toml'
  local out, cur = {}, nil
  local ok, lines = pcall(vim.fn.readfile, path)
  for _, line in ipairs(ok and lines or {}) do
    if line:match('^%s*%[%[connections%]%]') then
      cur = {}
      out[#out + 1] = cur
    elseif cur then
      local k, v = line:match('^%s*([%w_]+)%s*=%s*"(.-)"')
      if k == 'name' or k == 'driver' then cur[k] = v end
    end
  end
  return vim.tbl_filter(function(c) return c.name end, out)
end

--- A `complete` function for a command whose first argument is a saved
--- connection: `driver` limits it to that driver, `'query'` to query
--- connections (everything `:TradarConnect` serves). Later arguments get
--- nothing (topic/queue names need a live connection).
function M.connection_names(driver)
  return function(lead, cmdline, cursor)
    local before = cmdline:sub(1, cursor)
    -- Strip the command (and the `:Tradar <sub>` words) to count args.
    local args = vim.split(vim.trim(before:gsub('^%s*%S+', '', 1)), '%s+', { trimempty = true })
    if #args > (lead == '' and 0 or 1) then return {} end
    local names = {}
    for _, c in ipairs(M.saved()) do
      local fits = driver == nil or c.driver == driver or (driver == 'query' and not NON_QUERY[c.driver])
      if fits and vim.startswith(c.name, lead) then names[#names + 1] = c.name end
    end
    table.sort(names)
    return names
  end
end

--- Every `:Tradar<Words>` command as `{ words = {'kafka', 'tail'}, name = 'TradarKafkaTail' }`.
local function commands()
  local out = {}
  for name in pairs(vim.api.nvim_get_commands({})) do
    local rest = name:match('^Tradar(%u%w*)$')
    if rest then
      local words = {}
      for w in rest:gmatch('%u[%l%d]*') do words[#words + 1] = w:lower() end
      out[#out + 1] = { words = words, name = name }
    end
  end
  return out
end

--- The command `args` start with, longest match first (`run all` before
--- `run`), plus how many words it took.
local function resolve(args)
  local best, taken
  for _, c in ipairs(commands()) do
    local n = #c.words
    if n <= #args and (not taken or n > taken) then
      local same = true
      for i = 1, n do
        if args[i] ~= c.words[i] then same = false break end
      end
      if same then best, taken = c, n end
    end
  end
  return best, taken
end

--- `:Tradar <words> [args]`: runs the matching `:Tradar<Words>` with the
--- same range, bang and remaining arguments.
function M.dispatch(a)
  local args = a.fargs
  if #args == 0 then
    local names = vim.tbl_map(function(c) return table.concat(c.words, ' ') end, commands())
    table.sort(names)
    return vim.notify('tradar: :Tradar <' .. table.concat(names, ' | ') .. '>', vim.log.levels.INFO)
  end
  local c, taken = resolve(args)
  if not c then
    return vim.notify('tradar: unknown subcommand `' .. table.concat(args, ' ') .. '` -- :Tradar <Tab> lists them', vim.log.levels.WARN)
  end
  local range = ''
  if a.range == 2 then range = ('%d,%d'):format(a.line1, a.line2)
  elseif a.range == 1 then range = tostring(a.line1) end
  local rest = { unpack(args, taken + 1) }
  vim.cmd(range .. c.name .. (a.bang and '!' or '') .. (#rest > 0 and (' ' .. table.concat(rest, ' ')) or ''))
end

--- Completion for `:Tradar`: the next subcommand word, then -- once a
--- whole command is typed -- that command's own completion.
function M.complete(lead, cmdline, cursor)
  local before = cmdline:sub(1, cursor)
  local words = vim.split(vim.trim(before:gsub('^%s*%S+', '', 1)), '%s+', { trimempty = true })
  if lead ~= '' then table.remove(words) end
  local seen, out = {}, {}
  -- A whole command typed so far (`run`): its own arguments -- alongside the
  -- longer commands it starts (`run all`), collected below.
  local c, taken = resolve(words)
  if c and taken == #c.words then
    local rest = table.concat({ unpack(words, taken + 1) }, ' ')
    for _, item in ipairs(vim.fn.getcompletion(c.name .. ' ' .. (rest ~= '' and rest .. ' ' or '') .. lead, 'cmdline')) do
      if not seen[item] then
        seen[item] = true
        out[#out + 1] = item
      end
    end
    if taken < #words then return out end
  end
  for _, cmd in ipairs(commands()) do
    local prefix_ok = #cmd.words > #words
    for i = 1, #words do
      if cmd.words[i] ~= words[i] then prefix_ok = false break end
    end
    local nxt = prefix_ok and cmd.words[#words + 1]
    if nxt and vim.startswith(nxt, lead) and not seen[nxt] then
      seen[nxt] = true
      out[#out + 1] = nxt
    end
  end
  table.sort(out)
  return out
end

return M
