-- :checkhealth tradar
local M = {}

function M.check()
  vim.health.start('tradar')
  local rpc = require('tradar.rpc')
  local cmd = rpc.server_cmd({})
  if cmd then
    vim.health.ok('server binary: ' .. cmd)
  else
    vim.health.error('no tradar-server binary found', {
      'build it: cargo build --release -p tradar-server (in the tradar repo)',
      'or put tradar-server on PATH, or pass setup{ server_cmd = "/path/to/tradar-server" }',
    })
  end

  local ok_rpc, err = false, nil
  rpc.ensure({ autostart = false }, function(e) ok_rpc, err = (e == nil), e end)
  vim.wait(500, function() return ok_rpc or err ~= nil end, 10)
  if ok_rpc then
    vim.health.ok('server reachable')
    local e, list = rpc.request_sync('connections.list', nil, 1000)
    if e then
      vim.health.warn('connections.list failed: ' .. e)
    elseif #list == 0 then
      vim.health.warn('no saved connections', { 'add one in the tradar TUI (`a` in the picker) or edit ~/.config/tradar/connections.toml' })
    else
      vim.health.ok(('%d saved connection(s)'):format(#list))
    end
  else
    vim.health.info('server not running yet (it autostarts on first use): ' .. tostring(err))
  end

  local sock = vim.env.XDG_RUNTIME_DIR
  if sock and #sock > 70 then
    vim.health.warn('XDG_RUNTIME_DIR is long; unix sockets are limited to ~108 bytes', { 'pass a shorter socket path in setup{ socket = ... }' })
  end
  for _, mod in ipairs({ 'telescope', 'blink.cmp' }) do
    if pcall(require, mod) then vim.health.ok(mod .. ' found') else vim.health.info(mod .. ' not installed (optional)') end
  end
end

return M
