local ctx = require('tradar.ctx')
local M = ctx.M
local state = ctx.state
local notify = ctx.notify
local call = ctx.call

-- ── connections file ────────────────────────────────────────────────────

local CONNECTIONS_TEMPLATE = {
  '# tradar connections -- one [[connections]] table per backend.',
  '# driver: postgres | mysql | sqlite | mongo | elasticsearch | redis |',
  '#         cassandra | clickhouse | kafka | rabbitmq | http | socket | grpc',
  '# target: connection string / URL / file path / host:port for that driver.',
  '#',
  '# [[connections]]',
  '# name = "local-pg"',
  '# driver = "postgres"',
  '# target = "postgres://user:pass@localhost:5432/db"',
  '',
}

--- `:TradarConnections`: opens the `connections.toml` the server reads (the
--- same file as the TUI) in a buffer -- a commented template when it does
--- not exist yet. The server re-reads it on every use, so saving is all it
--- takes; a save is also checked right away, so a typo is reported there
--- and not on the next connect.
function M.edit_connections()
  local function open(path)
    if not path then return notify('the server has no connections file', vim.log.levels.WARN) end
    vim.fn.mkdir(vim.fn.fnamemodify(path, ':h'), 'p')
    vim.cmd.edit(vim.fn.fnameescape(path))
    local buf = vim.api.nvim_get_current_buf()
    if vim.fn.filereadable(path) == 0 and vim.api.nvim_buf_line_count(buf) == 1
        and vim.api.nvim_buf_get_lines(buf, 0, 1, false)[1] == '' then
      vim.api.nvim_buf_set_lines(buf, 0, -1, false, CONNECTIONS_TEMPLATE)
    end
    if vim.bo[buf].filetype == '' then vim.bo[buf].filetype = 'toml' end
    if vim.b[buf].tradar_connections_attached then return end
    vim.b[buf].tradar_connections_attached = true
    vim.api.nvim_create_autocmd('BufWritePost', {
      buffer = buf,
      callback = function()
        state.drivers = nil -- re-read: a connection may have been added or retargeted
        call('connections.list', nil, function(list)
          notify(('saved — %d connection(s)'):format(#list))
        end, function(err) notify('connections.toml: ' .. err, vim.log.levels.ERROR) end)
      end,
    })
  end
  -- An older server has no `connections.path`: fall back to the platform's
  -- usual config dir (what `directories` resolves to on Linux).
  call('connections.path', nil, function(res) open(res.path) end, function()
    open(vim.fn.stdpath('config'):gsub('[^/]+$', '') .. 'tradar/connections.toml')
  end)
end
