-- :checkhealth tradar
local M = {}

function M.check()
  vim.health.start('tradar')
  -- The plugin uses `vim.keycode` / `vim.islist` (both 0.10).
  if vim.fn.has('nvim-0.10') == 1 then
    vim.health.ok('Neovim ' .. tostring(vim.version()))
  else
    vim.health.error('Neovim >= 0.10 is required (vim.keycode, vim.islist)', { 'upgrade Neovim' })
  end
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
    local ve, vresult, vcode = rpc.request_sync('version', nil, 1000)
    if ve then
      -- -32601 (method not found): a pre-versioning server, built before
      -- this `version` method existed at all -- implicit major 0.
      local server_major = vcode == -32601 and 0 or nil
      if server_major then
        vim.health.warn(('tradar-server predates protocol versioning (expects major %d)'):format(rpc.CLIENT_PROTOCOL_MAJOR), {
          'rebuild tradar-server: cargo build --release -p tradar-server (in the tradar repo)',
        })
      else
        vim.health.warn('version check failed: ' .. ve)
      end
    elseif vresult.major ~= rpc.CLIENT_PROTOCOL_MAJOR then
      vim.health.warn(('protocol mismatch: server speaks major %d, plugin expects %d'):format(vresult.major, rpc.CLIENT_PROTOCOL_MAJOR), {
        'rebuild tradar-server or update the plugin so both match',
      })
    else
      vim.health.ok(('protocol v%d.%d'):format(vresult.major, vresult.minor))
    end

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
