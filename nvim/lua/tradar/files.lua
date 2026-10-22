-- Saving and finding query files. Two places a query can live:
--   the library   `queries_dir` (default ~/.config/tradar/queries, the same
--                 directory the TUI saves to): every query in one spot,
--                 browsed with `:TradarOpen`, whatever project you are in
--   here          a path next to your project, saved/opened like any file
-- `:TradarSave` writes to the library, `:TradarSaveHere` next to the buffer
-- (plain `:w` also works for a buffer that already has a file).
local ctx = require('tradar.ctx')
local M = ctx.M
local opts = ctx.opts
local notify = ctx.notify

--- The library directory: `setup{ queries_dir = ... }`, else next to
--- `connections.toml` (`<config>/tradar/queries`).
function M.queries_dir()
  if opts.queries_dir and opts.queries_dir ~= '' then
    return vim.fn.fnamemodify(vim.fn.expand(opts.queries_dir), ':p'):gsub('/$', '')
  end
  return (vim.fn.stdpath('config'):gsub('[^/]+$', '') .. 'tradar/queries')
end

local function has_escape(input)
  return input:match('^[/~]') or input:match('^%.%.?/')
end

--- Where `input` goes in the library: a bare name or `folder/name` lands
--- under `queries_dir` (with `.sql` added when there is no extension);
--- `/abs`, `~/x`, `./x` and `../x` are taken as typed. Same rules as the
--- TUI's `resolve_query_path`. `..` inside a library name is refused.
function M.resolve_library(input)
  input = vim.trim(input or '')
  if input == '' then return nil, 'empty name' end
  if has_escape(input) then return vim.fn.fnamemodify(vim.fn.expand(input), ':p') end
  if input:find('%.%.') and vim.tbl_contains(vim.split(input, '/', { plain = true }), '..') then
    return nil, "'..' is not allowed in a library name (use ./ or an absolute path)"
  end
  if input:sub(-1) == '/' then return nil, 'a file name is needed after the folder' end
  if vim.fn.fnamemodify(input, ':e') == '' then input = input .. '.sql' end
  return M.queries_dir() .. '/' .. input
end

--- A bare name for `:TradarSaveHere` lands next to the current file, else in
--- the working directory.
function M.resolve_here(input)
  input = vim.trim(input or '')
  if input == '' then return nil, 'empty name' end
  if vim.fn.fnamemodify(input, ':e') == '' then input = input .. '.sql' end
  if has_escape(input) then return vim.fn.fnamemodify(vim.fn.expand(input), ':p') end
  local cur = vim.api.nvim_buf_get_name(0)
  local base = cur ~= '' and vim.fs.dirname(cur) or vim.fn.getcwd()
  return vim.fn.fnamemodify(base .. '/' .. input, ':p')
end

local function comment_prefix(conn)
  local driver = ctx.M.cached_driver(conn) or ''
  if driver == 'mongo' or driver == 'elasticsearch' then return '//' end
  if driver == 'redis' then return '#' end
  return '--'
end

--- A buffer bound with `:TradarConnect` loses that binding once it is a file
--- that reopens in another session -- so write it down as a modeline, the way
--- the rest of the plugin already reads it back.
local function pin_connection(buf)
  local conn = vim.b[buf].tradar_connection
  if not conn or ctx.M.is_tdb(buf) then return end
  for _, line in ipairs(vim.api.nvim_buf_get_lines(buf, 0, 10, false)) do
    if line:match('^%s*%-%-%s*tradar:') or line:match('^%s*//%s*tradar:') or line:match('^%s*#%s*tradar:') then return end
  end
  vim.api.nvim_buf_set_lines(buf, 0, 0, false, { comment_prefix(conn) .. ' tradar: ' .. conn })
end

--- Writes the current buffer to `path`. A buffer that already has a file is
--- copied (the buffer keeps its own file); an unnamed one becomes `path`.
local function write_to(path, force, adopt)
  if vim.fn.filereadable(path) == 1 and not force then
    return notify(('%s exists — add ! to overwrite'):format(vim.fn.fnamemodify(path, ':~')), vim.log.levels.WARN)
  end
  vim.fn.mkdir(vim.fs.dirname(path), 'p')
  local buf = vim.api.nvim_get_current_buf()
  pin_connection(buf)
  local unnamed = vim.api.nvim_buf_get_name(buf) == ''
  local adopting = unnamed or adopt
  if adopting then
    -- Another buffer already holds that path (the file open from the library):
    -- naming this one the same would be E95.
    local other = vim.fn.bufnr('^' .. vim.fn.escape(path, '\\[]*?~.^$') .. '$')
    if other ~= -1 and other ~= buf then
      if vim.bo[other].modified then
        return notify('another buffer has unsaved changes to that file — save or close it first', vim.log.levels.WARN)
      end
      vim.cmd('bwipeout ' .. other)
    end
  end
  local cmd = adopting and 'saveas! ' or 'write! '
  local ok, err = pcall(vim.cmd, cmd .. vim.fn.fnameescape(path))
  if not ok then return notify('save failed: ' .. tostring(err), vim.log.levels.ERROR) end
  if vim.bo[buf].filetype == '' then vim.bo[buf].filetype = 'sql' end
  notify('saved ' .. vim.fn.fnamemodify(path, ':~'))
  return true
end

local function ask(prompt, default, cb)
  vim.ui.input({ prompt = prompt, default = default or '', completion = 'file' }, function(v)
    if v and vim.trim(v) ~= '' then cb(v) end
  end)
end

--- Library save. `name` omitted: asks, prefilled with the current file name.
function M.save(name, force)
  local function go(n)
    local path, err = M.resolve_library(n)
    if not path then return notify(err, vim.log.levels.WARN) end
    write_to(path, force, false)
  end
  if name and vim.trim(name) ~= '' then return go(name) end
  local cur = vim.api.nvim_buf_get_name(0)
  ask('Save to library (' .. vim.fn.fnamemodify(M.queries_dir(), ':~') .. '/): ', cur ~= '' and vim.fn.fnamemodify(cur, ':t') or '', go)
end

--- Save next to the buffer. `path` omitted: a buffer with a file is just
--- written; an unnamed one asks for a name.
function M.save_here(path, force)
  local function go(p)
    local resolved, err = M.resolve_here(p)
    if not resolved then return notify(err, vim.log.levels.WARN) end
    write_to(resolved, force, true)
  end
  if path and vim.trim(path) ~= '' then return go(path) end
  if vim.api.nvim_buf_get_name(0) ~= '' then
    local ok, err = pcall(vim.cmd, 'write')
    if not ok then return notify('save failed: ' .. tostring(err), vim.log.levels.ERROR) end
    return notify('saved ' .. vim.fn.fnamemodify(vim.api.nvim_buf_get_name(0), ':~'))
  end
  ask('Save as (relative to ' .. vim.fn.fnamemodify(vim.fn.getcwd(), ':~') .. '): ', '', go)
end

--- Every file in the library, newest first: `{ path, rel, mtime }`.
function M.library_files()
  local root = M.queries_dir()
  local out = {}
  if vim.fn.isdirectory(root) == 0 then return out end
  for name, kind in vim.fs.dir(root, { depth = 8 }) do
    if kind == 'file' and not name:match('^%.') then
      local path = root .. '/' .. name
      local st = (vim.uv or vim.loop).fs_stat(path)
      out[#out + 1] = { path = path, rel = name, mtime = st and st.mtime.sec or 0 }
    end
  end
  table.sort(out, function(a, b) return a.mtime > b.mtime end)
  return out
end

--- Library browser: telescope when installed (preview, fuzzy), else
--- `vim.ui.select`. `<CR>` opens the file.
function M.open(picker_opts)
  local files = M.library_files()
  if #files == 0 then
    return notify(('library is empty (%s) — save a query with :TradarSave'):format(vim.fn.fnamemodify(M.queries_dir(), ':~')), vim.log.levels.INFO)
  end
  local ok, pickers = pcall(require, 'telescope.pickers')
  if not ok then
    return vim.ui.select(files, { prompt = 'tradar library', format_item = function(f) return f.rel end }, function(f)
      if f then vim.cmd('edit ' .. vim.fn.fnameescape(f.path)) end
    end)
  end
  local finders = require('telescope.finders')
  local conf = require('telescope.config').values
  local actions = require('telescope.actions')
  local state = require('telescope.actions.state')
  picker_opts = picker_opts or {}
  pickers.new(picker_opts, {
    prompt_title = 'tradar library  (<CR> open · <C-s> split · <C-x> delete)',
    finder = finders.new_table({
      results = files,
      entry_maker = function(f)
        return { value = f, path = f.path, ordinal = f.rel, display = f.rel }
      end,
    }),
    sorter = conf.generic_sorter(picker_opts),
    previewer = conf.file_previewer(picker_opts),
    attach_mappings = function(bufnr, map)
      actions.select_default:replace(function()
        local e = state.get_selected_entry()
        actions.close(bufnr)
        if e then vim.cmd('edit ' .. vim.fn.fnameescape(e.value.path)) end
      end)
      map({ 'i', 'n' }, '<C-s>', function()
        local e = state.get_selected_entry()
        actions.close(bufnr)
        if e then vim.cmd('split ' .. vim.fn.fnameescape(e.value.path)) end
      end)
      map({ 'i', 'n' }, '<C-x>', function()
        local e = state.get_selected_entry()
        if not e then return end
        if vim.fn.confirm('Delete ' .. e.value.rel .. '?', '&Yes\n&No', 2) == 1 then
          os.remove(e.value.path)
          actions.close(bufnr)
          notify('deleted ' .. e.value.rel)
        end
      end)
      return true
    end,
  }):find()
end

return M
