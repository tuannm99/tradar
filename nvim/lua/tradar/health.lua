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

  -- `tradar-server` on PATH wins over a build next to the plugin, so a stale
  -- install there silently shadows a fresh `cargo build`.
  local on_path = vim.fn.exepath('tradar-server')
  if on_path ~= '' then
    local here = debug.getinfo(1, 'S').source:sub(2)
    local repo = vim.fn.fnamemodify(here, ':p:h:h:h:h')
    local path_stat = vim.uv.fs_stat(on_path)
    for _, profile in ipairs({ 'release', 'debug' }) do
      local built = repo .. '/target/' .. profile .. '/tradar-server'
      local built_stat = vim.uv.fs_stat(built)
      if built_stat and path_stat and vim.fn.resolve(built) ~= vim.fn.resolve(on_path)
          and built_stat.mtime.sec > path_stat.mtime.sec then
        vim.health.warn(('`tradar-server` on PATH (%s) is older than your build (%s) and is the one used'):format(on_path, built), {
          'copy the fresh build over it, remove it from PATH, or set setup{ server_cmd = "' .. built .. '" }',
        })
        break
      end
    end
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
    if vresult and vresult.exe then
      if vresult.stale then
        vim.health.warn('the running server is older than the binary on disk: ' .. vresult.exe, { ':TradarRestart (the next command starts the new build)' })
      else
        vim.health.ok('running server binary: ' .. vresult.exe)
      end
    end

    if vresult and vresult.idle_ttl_secs then
      vim.health.ok(('idle connections close after %ds without use (TRADAR_IDLE_TTL / setup{ idle_ttl })'):format(vresult.idle_ttl_secs))
    elseif vresult and vresult.idle_ttl_secs == nil and vresult.major then
      vim.health.info('idle disconnect: off (or an older server)')
    end
    if vresult and vresult.log_path then
      vim.health.ok('server log: ' .. vresult.log_path .. ' (:TradarLog)')
    elseif vresult then
      vim.health.info('server log: none reported (older server, or TRADAR_LOG=off)')
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

  local path = rpc.socket_path(require('tradar').rpc_opts())
  local too_long = rpc.check_path(path)
  if too_long then
    vim.health.error(too_long)
  else
    vim.health.ok('socket: ' .. path .. (#path > 70 and (' (%d of %d bytes)'):format(#path, rpc.max_socket_path()) or ''))
  end
  for _, mod in ipairs({ 'telescope', 'blink.cmp' }) do
    if pcall(require, mod) then vim.health.ok(mod .. ' found') else vim.health.info(mod .. ' not installed (optional)') end
  end
end

return M
