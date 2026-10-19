local ctx = require('tradar.ctx')
local M = ctx.M
local HISTORY_MAX = ctx.HISTORY_MAX
local state = ctx.state

-- ── history ──────────────────────────────────────────────────────────────

local function history_path() return vim.fn.stdpath('state') .. '/tradar_history.json' end

function M.history()
  if state.history then return state.history end
  state.history = {}
  local f = io.open(history_path(), 'r')
  if f then
    local ok, decoded = pcall(vim.json.decode, f:read('*a'))
    f:close()
    if ok and type(decoded) == 'table' then state.history = decoded end
  end
  return state.history
end

local function remember(conn, text)
  local history = M.history()
  text = vim.trim(text)
  if text == '' or (history[1] and history[1].text == text and history[1].conn == conn) then return end
  table.insert(history, 1, { conn = conn, text = text })
  while #history > HISTORY_MAX do table.remove(history) end
  vim.fn.mkdir(vim.fn.fnamemodify(history_path(), ':h'), 'p')
  local f = io.open(history_path(), 'w')
  if f then
    f:write(vim.json.encode(history))
    f:close()
  end
end

ctx.remember = remember
