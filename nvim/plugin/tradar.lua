if vim.g.loaded_tradar then return end
vim.g.loaded_tradar = true

local function t() return require('tradar') end

vim.api.nvim_create_user_command('TradarConnect', function(a) t().connect(a.args) end, { nargs = '?' })
vim.api.nvim_create_user_command('TradarRun', function(a)
  if a.range > 0 then t().run(a.line1, a.line2) else t().run() end
end, { range = true })
vim.api.nvim_create_user_command('TradarRunAll', function() t().run(nil, nil, true) end, {})
vim.api.nvim_create_user_command('TradarMore', function() t().more() end, {})
vim.api.nvim_create_user_command('TradarSchema', function() t().schema() end, {})

-- Context-aware completion on any SQL buffer; returns nothing until a
-- connection is active, so it never gets in the way of a plain .sql file.
vim.api.nvim_create_autocmd('FileType', {
  pattern = 'sql',
  callback = function(a) vim.bo[a.buf].omnifunc = "v:lua.require'tradar'.omnifunc" end,
})
