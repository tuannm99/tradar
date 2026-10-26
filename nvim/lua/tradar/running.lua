local ctx = require('tradar.ctx')
local params = require('tradar.params')
local M = ctx.M
local rpc = ctx.rpc
local guard = ctx.guard
local blocks = ctx.blocks
local uv = ctx.uv
local opts = ctx.opts
local ns = ctx.ns
local DEFAULT_PAGE = ctx.DEFAULT_PAGE
local state = ctx.state
local notify = ctx.notify
local call = ctx.call

-- ── running queries ──────────────────────────────────────────────────────

local function offset_to_pos(lines, offset)
  -- `offset` is a byte offset into table.concat(lines, '\n').
  local row = 0
  for _, l in ipairs(lines) do
    if offset <= #l then return row, offset end
    offset = offset - #l - 1
    row = row + 1
  end
  return row, 0
end

local function set_diagnostic(buf, base_row, base_col, message, data)
  if not (data and data.line and data.column) then return end
  local lnum = base_row + data.line - 1
  local col = (data.line == 1 and base_col or 0) + data.column
  vim.diagnostic.set(ns, buf, { {
    lnum = lnum, col = col, severity = vim.diagnostic.severity.ERROR,
    message = vim.split(message, '\n', { plain = true })[1], source = 'tradar',
  } })
  if not vim.b[buf].tradar_diag_clear then
    vim.b[buf].tradar_diag_clear = true
    -- Stale squiggles are worse than none: drop them on the next edit.
    vim.api.nvim_create_autocmd({ 'TextChanged', 'InsertEnter' }, {
      buffer = buf,
      once = true,
      callback = function()
        vim.diagnostic.reset(ns, buf)
        vim.b[buf].tradar_diag_clear = false
      end,
    })
  end
end

local TX_VERBS = { begin = true, start = true, commit = true, rollback = true, ['end'] = true, abort = true }

local flash_ns = vim.api.nvim_create_namespace('tradar_flash')
vim.api.nvim_set_hl(0, 'TradarRunning', { link = 'IncSearch', default = true })

--- Briefly marks the statement that was just sent (`setup{ flash_ms }`,
--- default 200; 0 turns it off) -- with several statements in a buffer,
--- which one ran is otherwise a guess. `row`/`col` are where `text` starts.
local function flash(buf, row, col, text)
  local ms = opts.flash_ms
  if ms == nil then ms = 200 end
  if ms <= 0 or not vim.api.nvim_buf_is_valid(buf) then return end
  local lines = vim.split(text, '\n', { plain = true })
  local ok, mark = pcall(vim.api.nvim_buf_set_extmark, buf, flash_ns, row, col, {
    end_row = row + #lines - 1,
    end_col = (#lines == 1 and col or 0) + #lines[#lines],
    hl_group = 'TradarRunning',
  })
  if not ok then return end
  vim.defer_fn(function() pcall(vim.api.nvim_buf_del_extmark, buf, flash_ns, mark) end, ms)
end

local function run_text(buf, text, base_row, base_col, flags)
  flags = flags or {}
  if state.running then
    return notify('a query is already running — cancel it with <leader>rx', vim.log.levels.WARN)
  end
  -- `base_row` is 0-based (it is a buffer line offset, used below for
  -- diagnostics positions); `block_connection`/`blocks.at` want 1-based.
  -- `flags.refresh`: re-running a shown result (`<leader>rR`, `:TradarWatch`)
  -- -- on that result's own connection, with no place in any buffer.
  local conn = flags.conn or M.block_connection(buf, base_row + 1)
  if not conn then
    return M.connect(nil, function() run_text(buf, text, base_row, base_col) end)
  end
  if not flags.refresh then state.sql_buf = buf end

  if not flags.confirmed and opts.confirm ~= false then
    local confirmed_flags = vim.tbl_extend('force', flags, { confirmed = true })
    local protected = M.is_protected(conn)
    return ctx.driver_of(conn, function(driver)
      local reason = guard.assess(text, protected, driver)
      if not reason then return run_text(buf, text, base_row, base_col, confirmed_flags) end
      local first = guard.first_code_line(text)
      -- "Cancel" is first so a reflexive <CR> is the safe answer.
      vim.ui.select({ 'Cancel', 'Run anyway' }, {
        prompt = ('tradar [%s]: %s\n  %s'):format(conn, reason, first:sub(1, 70)),
      }, function(choice)
        if choice == 'Run anyway' then run_text(buf, text, base_row, base_col, confirmed_flags) end
      end)
    end)
  end

  ctx.ensure_connected(conn, function()
    -- `flags.flash` is the statement as it stands in the buffer (an EXPLAIN
    -- sends a rewritten text, but the one to mark is still the original).
    if flags.flash then flash(buf, base_row, base_col, flags.flash) end
    state.counter = state.counter + 1
    local id = ('nv%d-%d'):format(vim.fn.getpid(), state.counter)
    state.running = { id = id, conn = conn, started = uv.hrtime() }
    ctx.start_spinner()
    local started = uv.hrtime()
    if not flags.refresh then ctx.remember(conn, text) end
    local function finish()
      state.running = nil
      ctx.stop_spinner()
    end
    call('execute', { connection = conn, query = text, page_size = opts.page_size or DEFAULT_PAGE, max_rows = opts.max_rows, query_id = id }, function(r)
      finish()
      -- The database's own time when the server reports it, else what
      -- this client saw (round trip included).
      local ms = r.elapsed_ms or (uv.hrtime() - started) / 1e6
      r.took_ms = ms
      local took = ctx.took_text(ms)
      state.last = r.kind == 'affected' and ('%d affected · %s'):format(r.rows, took)
        or ('%d/%d rows · %s'):format(#r.rows, r.total, took)
      if vim.api.nvim_buf_is_valid(buf) then vim.diagnostic.reset(ns, buf) end
      if guard.changes_schema(text) then state.schema_cache[conn] = nil end
      ctx.show_result(r, { conn = conn, query = text, refresh = flags.refresh })
      -- BEGIN/COMMIT/ROLLBACK flip the `tx` badge now, not at the next 15s poll.
      local verb = text:match('^%s*(%a+)')
      if verb and TX_VERBS[verb:lower()] then
        rpc.request('status', { connection = conn }, function(err, st)
          if not err then
            state.alive[conn], state.in_tx[conn] = st.alive, st.in_transaction
            vim.cmd.redrawstatus()
          end
        end)
      end
      if flags.after then flags.after() end
    end, function(err, data)
      finish()
      if err == 'query cancelled' then
        state.last = 'cancelled'
        return notify('cancelled')
      end
      if err:find('not connected', 1, true) and not flags.retried then
        -- The server restarted since we connected: reconnect once, quietly.
        state.connected[conn] = nil
        return run_text(buf, text, base_row, base_col, vim.tbl_extend('force', flags, { retried = true }))
      end
      state.last = 'error'
      if flags.refresh then
        -- Keep the grid on screen: a refresh that fails says so, nothing more.
        if flags.on_error then flags.on_error(err) end
        return notify('refresh failed: ' .. err, vim.log.levels.ERROR)
      end
      ctx.show_message(err)
      if flags.on_error then flags.on_error(err) end
      if vim.api.nvim_buf_is_valid(buf) then set_diagnostic(buf, base_row, base_col, err, data) end
    end)
  end, function(err)
    notify(err, vim.log.levels.ERROR)
  end)
end

--- Visual selection / range: runs exactly those lines.
--- No range: the statement under the cursor (boundaries are the driver's,
--- not a regex here). `all`: every statement in the buffer, in order.
function M.run(range_start, range_end, all, transform)
  local buf = vim.api.nvim_get_current_buf()
  -- A visual selection is run the way "run all" runs a buffer: the server
  -- splits it on the dialect's own statement boundaries (`;` for SQL), so
  -- several statements run one after another instead of being sent as one
  -- text the driver might refuse.
  local selection
  if range_start then
    selection = vim.api.nvim_buf_get_lines(buf, range_start - 1, range_end, false)
    all = true
  end

  -- `.tdb`: scoped to the fenced block under the cursor, not the whole
  -- buffer -- a buffer can mix dialects (one block SQL, the next Mongo),
  -- and sending that mix through one driver's own `split` would be
  -- nonsense. `base_row` (0-based) is where the block's own `lines`
  -- starts within the real buffer, for turning a block-relative position
  -- back into a real one before it reaches `run_text`/diagnostics, both
  -- of which only know real buffer coordinates.
  local base_row = 0
  local lines
  if selection then
    base_row, lines = range_start - 1, selection
  elseif M.is_tdb(buf) then
    local cursor_row = vim.api.nvim_win_get_cursor(0)[1]
    local block = M.block_at(buf, cursor_row, true)
    if not block then
      return notify('cursor is between blocks — put it inside a ```sql/```mongo/```redis block (or on its fence line)', vim.log.levels.WARN)
    end
    base_row = block.start - 1
    local all_lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
    lines = {}
    for i = block.start, block.finish do lines[#lines + 1] = all_lines[i] end
  else
    lines = vim.api.nvim_buf_get_lines(buf, 0, -1, false)
  end

  local conn = M.block_connection(buf, base_row + 1)
  if not conn then
    return M.connect(nil, function() M.run(nil, nil, all) end)
  end
  local text = table.concat(lines, '\n')
  ctx.ensure_connected(conn, function()
    call('split', { connection = conn, text = text }, function(statements)
      -- The binding comment (`-- tradar: x`) can come back as a statement of
      -- its own; it is not something to run.
      statements = vim.tbl_filter(function(st) return not guard.is_comment_only(st.text) end, statements)
      if #statements == 0 then return notify('nothing to run', vim.log.levels.WARN) end
      local function position(s)
        local row, col = offset_to_pos(lines, s.start)
        return row + base_row, col
      end
      if all then
        -- `${name}` parameters are asked once for the whole batch.
        local texts = vim.tbl_map(function(st) return st.text end, statements)
        params.expand(conn, texts, function(expanded)
          local function go()
            -- Sequentially: each run starts when the previous one finished,
            -- and the batch stops at the first error or cancel.
            local i = 0
            local function next_statement()
              i = i + 1
              local st = statements[i]
              if not st then return end
              local row, col = position(st)
              run_text(buf, expanded[i], row, col, { confirmed = true, flash = st.text })
              local timer = uv.new_timer()
              timer:start(50, 50, vim.schedule_wrap(function()
                if not state.running then
                  timer:stop()
                  timer:close()
                  if state.last ~= 'error' and state.last ~= 'cancelled' then next_statement() end
                end
              end))
            end
            next_statement()
          end

          -- One question for the whole batch, not one per statement. The
          -- driver is needed first: Mongo/Redis/Elasticsearch have their own
          -- notion of "risky", and the lookup is async the first time. What
          -- is judged is the text that will actually be sent.
          ctx.driver_of(conn, function(driver)
            local risky = {}
            for _, text in ipairs(expanded) do
              local why = opts.confirm ~= false and guard.assess(text, M.is_protected(conn), driver)
              if why then risky[#risky + 1] = why end
            end
            if #risky == 0 then return go() end
            vim.ui.select({ 'Cancel', 'Run all' }, {
              prompt = ('tradar [%s]: %d of %d statements need a second look\n  %s'):format(
                conn, #risky, #statements, table.concat(risky, '; '):sub(1, 90)),
            }, function(choice) if choice == 'Run all' then go() end end)
          end)
        end)
        return
      end
      local row, col = unpack(vim.api.nvim_win_get_cursor(0))
      row = row - base_row
      local offset = col
      for r = 1, row - 1 do offset = offset + #lines[r] + 1 end
      local chosen
      for _, s in ipairs(statements) do
        if offset >= s.start and offset <= s['end'] then chosen = s break end
        if s.start <= offset then chosen = s end -- between statements: the one above
      end
      chosen = chosen or statements[1]
      local srow, scol = position(chosen)
      params.expand(conn, { transform and transform(chosen.text) or chosen.text }, function(expanded)
        run_text(buf, expanded[1], srow, scol, { flash = chosen.text })
      end)
    end)
  end, function(err) notify(err, vim.log.levels.ERROR) end)
end

--- Stops the background server (it restarts itself on the next command).
--- For picking up a rebuilt binary: a long-running server keeps the old one.
function M.restart()
  if not rpc.connected() then return notify('no server is running; the next command starts a fresh one') end
  rpc.request('shutdown', nil, function()
    rpc.close()
    state.connected, state.connecting, state.drivers, state.schema_cache = {}, {}, nil, {}
    notify('server stopped; the next command starts a fresh one')
  end)
end

function M.cancel()
  if not state.running then return notify('nothing is running') end
  call('cancel', { query_id = state.running.id }, function() end)
end

--- Binds the current buffer to a saved connection (picker if `name` empty).
function M.connect(name, after)
  local buf = vim.api.nvim_get_current_buf()
  local function go(chosen)
    vim.b[buf].tradar_connection = chosen
    state.default = chosen
    ctx.ensure_connected(chosen, function()
      call('schema', { connection = chosen }, function(entries)
        notify(('connected to %s (%d objects)'):format(chosen, #entries))
        if after then after() end
      end)
    end, function(err) notify(err, vim.log.levels.ERROR) end)
  end
  if name and name ~= '' then return go(name) end
  call('connections.list', nil, function(list)
    local usable = vim.tbl_filter(function(c) return c.supported end, list)
    if #list == 0 then
      -- First run: nothing to pick from, so start on adding one.
      notify('no connections yet — let\'s add one (or edit the file yourself: :TradarConnections)')
      return M.add_connection()
    end
    if #usable == 0 then
      return notify('no saved connection this server can serve — add one with :TradarAddConnection (or edit the file: :TradarConnections)', vim.log.levels.WARN)
    end
    -- Telescope, when installed: a preview of each connection beside the list.
    if ctx.opts.connection_picker ~= 'ui' and require('tradar.telescope').available() then
      return require('tradar.telescope').connections(list, go)
    end
    local items = vim.list_extend(vim.deepcopy(usable), { { add = true }, { edit = true } })
    vim.ui.select(items, {
      prompt = 'tradar connection',
      format_item = function(c)
        if c.add then return '+ add a connection…' end
        if c.edit then return '+ edit connections file…' end
        return ('%s  (%s)%s'):format(c.name, c.driver, c.connected and '  ●' or '')
      end,
    }, function(c)
      if not c then return end
      if c.add then return M.add_connection() end
      if c.edit then return M.edit_connections() end
      go(c.name)
    end)
  end)
end

ctx.run_text = run_text
