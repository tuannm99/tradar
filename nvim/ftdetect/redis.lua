-- Sourced with the runtimepath at startup, before any buffer is read -- so the
-- filetype is right even when plugin/tradar.lua itself loads late.
vim.filetype.add({ extension = { redis = 'redis' } })
