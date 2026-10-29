-- HTTP over `tradar-server` (`http.*` methods,
-- docs/backlog/http-server-2026-10-17.md): the requests live in the request
-- workbench shared with gRPC (`restbench.lua`, format in `reqparse.lua`) and the
-- responses in the result tabs.
local M = {}

--- `:TradarHttpConnect [name]`: records the connection on the server (no call
--- goes out, so it never fails) and remembers it as the one to use.
function M.connect(name)
  local rpc = require('tradar.rpc')
  rpc.ensure(require('tradar').rpc_opts(), function(err)
    if err then return vim.notify('tradar-http: ' .. err, vim.log.levels.ERROR) end
    local function go(conn)
      rpc.request('http.connect', { connection = conn }, function(e)
        if e then return vim.notify('tradar-http: ' .. e, vim.log.levels.ERROR) end
        vim.notify('tradar-http: connected to ' .. conn)
      end)
    end
    if name and name ~= '' then return go(name) end
    require('tradar.restbench').open(nil, 'http')
  end)
end

--- `:TradarHttpSend [name]`: inside a workbench, runs the block under the
--- cursor; elsewhere opens the connection's workbench.
function M.send(name) require('tradar.restbench').open(name, 'http') end

return M
