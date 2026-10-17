if vim.g.loaded_tradar then return end
vim.g.loaded_tradar = true

local function t() return require('tradar') end

vim.api.nvim_create_user_command('TradarConnect', function(a) t().connect(a.args) end, { nargs = '?' })
vim.api.nvim_create_user_command('TradarConnections', function() t().edit_connections() end, {})
vim.api.nvim_create_user_command('TradarRun', function(a)
  if a.range > 0 then t().run(a.line1, a.line2) else t().run() end
end, { range = true })
vim.api.nvim_create_user_command('TradarRunAll', function() t().run(nil, nil, true) end, {})
vim.api.nvim_create_user_command('TradarRestart', function() t().restart() end, {})
vim.api.nvim_create_user_command('TradarCancel', function() t().cancel() end, {})
vim.api.nvim_create_user_command('TradarMore', function() t().more() end, {})
vim.api.nvim_create_user_command('TradarSchema', function() t().schema() end, {})
vim.api.nvim_create_user_command('TradarTables', function() require('tradar.telescope').tables() end, {})
vim.api.nvim_create_user_command('TradarHistory', function() require('tradar.telescope').history() end, {})
vim.api.nvim_create_user_command('TradarExport', function(a)
  local fmt, path = a.fargs[1], a.fargs[2]
  t().export(fmt, path)
end, { nargs = '*', complete = function() return { 'csv', 'json', 'md', 'tsv' } end })

-- gRPC: unary-only v1 (docs/architecture.md, "Chốt lại scope v1" under
-- gRPC) -- a connection, not a buffer's filetype, so these have no `attach`
-- autocmd the way sql/mongo/redis/esq do; run them from anywhere.
vim.api.nvim_create_user_command('TradarGrpcConnect', function(a) require('tradar.grpc').connect(a.args) end, { nargs = '?' })
vim.api.nvim_create_user_command('TradarGrpcServices', function() require('tradar.grpc').services() end, {})
vim.api.nvim_create_user_command('TradarGrpcMethods', function(a) require('tradar.grpc').methods(nil, a.args) end, { nargs = 1 })
vim.api.nvim_create_user_command('TradarGrpcLoadProto', function(a)
  require('tradar.grpc').load_proto(nil, a.args)
end, { nargs = 1, complete = 'file' })

-- Kafka tail: a push-channel notification stream, not a request/response
-- method the way everything above is -- see docs/backlog/kafka-tail-server-2026-10-15.md.
vim.api.nvim_create_user_command('TradarKafkaConnect', function(a) require('tradar.kafka').connect(a.args) end, { nargs = '?' })
vim.api.nvim_create_user_command('TradarKafkaTail', function(a)
  require('tradar.kafka').tail(a.fargs[1], a.fargs[2])
end, { nargs = '*' })

-- Socket tail: same push-channel, two-way (typing a line sends it) --
-- see docs/backlog/socket-tail-server-2026-10-15.md.
vim.api.nvim_create_user_command('TradarSocketTail', function(a) require('tradar.socket').tail(a.args) end, { nargs = '?' })

-- RabbitMQ: no push channel needed (Management HTTP API has no streaming
-- endpoint) -- plain request/response, see docs/backlog/rabbitmq-server-2026-10-16.md.
vim.api.nvim_create_user_command('TradarRabbitmqConnect', function(a) require('tradar.rabbitmq').connect(a.args) end, { nargs = '?' })
vim.api.nvim_create_user_command('TradarRabbitmqPeek', function(a)
  require('tradar.rabbitmq').peek(a.fargs[1], a.fargs[2])
end, { nargs = '*' })
vim.api.nvim_create_user_command('TradarRabbitmqBindings', function(a)
  require('tradar.rabbitmq').bindings(a.fargs[1], a.fargs[2])
end, { nargs = '*' })
vim.api.nvim_create_user_command('TradarRabbitmqPublish', function(a) require('tradar.rabbitmq').publish(a.args) end, { nargs = '?' })

-- HTTP: same reasoning as RabbitMQ above -- no push channel, plain
-- request/response, see docs/backlog/http-server-2026-10-17.md.
vim.api.nvim_create_user_command('TradarHttpConnect', function(a) require('tradar.http').connect(a.args) end, { nargs = '?' })
vim.api.nvim_create_user_command('TradarHttpSend', function(a) require('tradar.http').send(a.args) end, { nargs = '?' })

-- Query files for the non-SQL connectors. Highlighting borrows a real
-- grammar where one fits (`.mongo` is JavaScript) -- see README.
vim.filetype.add({ extension = { mongo = 'mongo', redis = 'redis', esq = 'esq' } })
pcall(vim.treesitter.language.register, 'javascript', 'mongo')
pcall(vim.treesitter.language.register, 'json', 'esq')

-- `.tdb`: one file, several ```sql/```mongo/```redis fenced blocks (see
-- "The .tdb format" in README) instead of one filetype per dialect.
-- Backed by the markdown parser so each fence's own info string drives
-- markdown-treesitter's own fenced-code injection -- the two
-- `language.register` calls above are what let a ```mongo fence resolve
-- to the javascript parser there (the same alias both buffer-filetype
-- and injection-language resolution read), not anything specific to
-- `.tdb`. A plain ````redis` fence has no parser registered for it, so
-- it stays unhighlighted inside the block -- same as today's standalone
-- `.redis` filetype, not a regression.
vim.filetype.add({ extension = { tdb = 'tdb' } })
pcall(vim.treesitter.language.register, 'markdown', 'tdb')

-- Keymaps, omnifunc and a quiet background connect on every SQL buffer, and
-- on any buffer (whatever its filetype) that names a connection in a
-- `-- tradar: name` / `// tradar: name` / `# tradar: name` line.
local group = vim.api.nvim_create_augroup('tradar_sql', { clear = true })
vim.api.nvim_create_autocmd('FileType', {
  pattern = { 'sql', 'mongo', 'redis', 'esq', 'tdb' },
  group = group,
  callback = function(a) t().attach(a.buf) end,
})
vim.api.nvim_create_autocmd({ 'BufReadPost', 'BufNewFile' }, {
  group = group,
  callback = function(a) t().maybe_attach(a.buf) end,
})

-- An LSP's own on_attach sets K/gd after FileType; take them back once all
-- LspAttach handlers have run.
vim.api.nvim_create_autocmd('LspAttach', {
  group = vim.api.nvim_create_augroup('tradar_lsp', { clear = true }),
  callback = function(a)
    if vim.b[a.buf].tradar_attached then
      vim.schedule(function() if vim.api.nvim_buf_is_valid(a.buf) then t().attach_nav(a.buf) end end)
    end
  end,
})
