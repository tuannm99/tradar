-- `:Telescope tradar tables|history`
return require('telescope').register_extension({
  exports = {
    tables = function(opts) require('tradar.telescope').tables(opts) end,
    history = function(opts) require('tradar.telescope').history(opts) end,
  },
})
