if vim.g.loaded_tradar then return end
vim.g.loaded_tradar = true

local function t() return require('tradar') end

vim.api.nvim_create_user_command('TradarConnect', function(a) t().connect(a.args) end, { nargs = '?' })
vim.api.nvim_create_user_command('TradarRun', function(a)
  if a.range > 0 then t().run(a.line1, a.line2) else t().run() end
end, { range = true })
vim.api.nvim_create_user_command('TradarRunAll', function() t().run(nil, nil, true) end, {})
vim.api.nvim_create_user_command('TradarCancel', function() t().cancel() end, {})
vim.api.nvim_create_user_command('TradarMore', function() t().more() end, {})
vim.api.nvim_create_user_command('TradarSchema', function() t().schema() end, {})
vim.api.nvim_create_user_command('TradarTables', function() require('tradar.telescope').tables() end, {})
vim.api.nvim_create_user_command('TradarHistory', function() require('tradar.telescope').history() end, {})
vim.api.nvim_create_user_command('TradarExport', function(a)
  local fmt, path = a.fargs[1], a.fargs[2]
  t().export(fmt, path)
end, { nargs = '*', complete = function() return { 'csv', 'json', 'md', 'tsv' } end })

-- Keymaps, omnifunc and a quiet background connect on every SQL buffer.
vim.api.nvim_create_autocmd('FileType', {
  pattern = 'sql',
  group = vim.api.nvim_create_augroup('tradar_sql', { clear = true }),
  callback = function(a) t().attach(a.buf) end,
})
