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

-- Query files for the non-SQL connectors. Highlighting borrows a real
-- grammar where one fits (`.mongo` is JavaScript) -- see README.
vim.filetype.add({ extension = { mongo = 'mongo', redis = 'redis', esq = 'esq' } })
pcall(vim.treesitter.language.register, 'javascript', 'mongo')
pcall(vim.treesitter.language.register, 'json', 'esq')

-- Keymaps, omnifunc and a quiet background connect on every SQL buffer, and
-- on any buffer (whatever its filetype) that names a connection in a
-- `-- tradar: name` / `// tradar: name` / `# tradar: name` line.
local group = vim.api.nvim_create_augroup('tradar_sql', { clear = true })
vim.api.nvim_create_autocmd('FileType', {
  pattern = { 'sql', 'mongo', 'redis', 'esq' },
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
