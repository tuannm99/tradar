-- Shared by the Kafka and Socket tail buffers: append pushed lines without
-- fighting the reader. Follows the newest line only while the cursor is
-- already on the last line (scroll up to read, and the buffer stays put),
-- and drops the oldest lines past `MAX_LINES` so a tail left open for hours
-- does not grow without bound.
local M = {}

M.MAX_LINES = 5000

--- Appends `lines` to `buf`. With `opts.input` the buffer's last line is a
--- live input line (Socket): the lines go *above* it, the buffer stays
--- modifiable, and the cursor col is kept when the cursor sits on it.
--- Without it the buffer is a read-only transcript (Kafka).
function M.append(buf, lines, opts)
  if not vim.api.nvim_buf_is_valid(buf) then return end
  opts = opts or {}
  local win = vim.fn.bufwinid(buf)
  local total = vim.api.nvim_buf_line_count(buf)
  local cursor = win ~= -1 and vim.api.nvim_win_get_cursor(win) or nil
  local follow = cursor ~= nil and cursor[1] >= total

  vim.bo[buf].modifiable = true
  if opts.input then
    vim.api.nvim_buf_set_lines(buf, total - 1, total - 1, false, lines)
  else
    -- The buffer opens with one empty placeholder line; the first real
    -- message replaces it instead of leaving a blank line above the tail.
    local placeholder = total == 1 and vim.api.nvim_buf_get_lines(buf, 0, 1, false)[1] == ''
    vim.api.nvim_buf_set_lines(buf, placeholder and 0 or total, total, false, lines)
  end
  local keep = M.MAX_LINES + (opts.input and 1 or 0)
  local count = vim.api.nvim_buf_line_count(buf)
  if count > keep then vim.api.nvim_buf_set_lines(buf, 0, count - keep, false, {}) end
  vim.bo[buf].modifiable = opts.input and true or false

  if follow then
    pcall(vim.api.nvim_win_set_cursor, win, { vim.api.nvim_buf_line_count(buf), opts.input and cursor[2] or 0 })
  end
end

return M
