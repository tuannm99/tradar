-- results_layout = bottom | right | tab, and zoom.
local H = dofile('nvim/tests/helpers.lua')
H.sql('create table p(id integer primary key, name text); insert into p values (1, \'a\')')
H.setup()
H.sql_buffer('layout.sql')
H.check('connects', H.connect())
local W, Hh = vim.o.columns, vim.o.lines

for _, layout in ipairs({ 'bottom', 'right', 'tab' }) do
  -- A fresh window layout for each case.
  pcall(vim.cmd, 'silent! tabonly')
  vim.api.nvim_set_current_buf(H.sqlbuf)
  vim.cmd('silent! only')
  if H.rbuf() ~= -1 then vim.api.nvim_buf_delete(H.rbuf(), { force = true }) end
  local t = H.setup({ results_layout = layout, results_height = 12 })
  local sqlwin = vim.api.nvim_get_current_win()
  H.run('select * from p')
  H.wait(4000, function() return H.rbuf() ~= -1 and vim.fn.bufwinid(H.rbuf()) ~= -1 and H.text():find('rows', 1, true) end)
  local win = vim.fn.bufwinid(H.rbuf())
  if layout == 'bottom' then
    H.check('bottom: uses results_height', vim.api.nvim_win_get_height(win) == 12, vim.api.nvim_win_get_height(win))
  elseif layout == 'right' then
    H.check('right: beside the editor', vim.api.nvim_win_get_height(win) > 15 and vim.api.nvim_win_get_width(win) < W * 0.6, vim.api.nvim_win_get_width(win))
  else
    H.check('tab: results in their own tab, editor keeps focus', #vim.api.nvim_list_tabpages() == 2 and vim.api.nvim_get_current_win() == sqlwin)
  end
  if layout ~= 'tab' then
    local h0, w0 = vim.api.nvim_win_get_height(win), vim.api.nvim_win_get_width(win)
    vim.api.nvim_set_current_win(sqlwin)
    t.zoom()
    -- Other windows keep their minimum size (1 line/column + separator).
    local hz, wz = vim.api.nvim_win_get_height(win), vim.api.nvim_win_get_width(win)
    H.check(layout .. ': zoom focuses the results and fills the screen',
      vim.api.nvim_get_current_win() == win and hz >= Hh - 5 and wz >= W - 2, hz .. 'x' .. wz)
    t.zoom()
    H.check(layout .. ': second zoom restores the size and the editor focus',
      vim.api.nvim_win_get_height(win) == h0 and vim.api.nvim_win_get_width(win) == w0 and vim.api.nvim_get_current_win() == sqlwin)
  end
end

H.done()
