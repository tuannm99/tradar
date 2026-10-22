-- The results buffer: footer, paging up to max_rows, export of a capped result.
local H = dofile('nvim/tests/helpers.lua')
H.sql([[create table p(id integer primary key, name text);
insert into p values (1,'a')]])
local t = H.setup({ max_rows = 150 })
H.sql_buffer('results.sql')
H.check('connects', H.connect())

H.check('select', H.run_until('select * from p', 'rows'))
H.check('footer shows rows and time', H.text('|'):match('1/1 rows · %d+ms'), H.text('|'))

H.check('update', H.run_until("update p set name = 'q' where id = 1", 'affected'))
H.check('affected line shows time', H.text('|'):match('1 row%(s%) affected · %d+ms'), H.text('|'))

local many = 'with recursive n(i) as (select 1 union all select i+1 from n where i < 5000) select i from n'
H.check('large select', H.run_until(many, 'rows'))
H.check('first page, capped at max_rows, says so', H.text():find('20/150 rows (stopped at 150 rows', 1, true), H.text():sub(-160))
vim.api.nvim_set_current_win(vim.fn.bufwinid(H.rbuf()))
for _ = 1, 8 do
  t.more()
  H.sleep(200)
end
H.check('paging stops at the cap', H.text():find('150/150 rows', 1, true), H.text():sub(-120))

local csv = H.tmp .. '/out.csv'
H.notes = {}
t.export('csv', csv)
H.wait(3000, function() return #H.notes_with('wrote') > 0 end)
local note = H.notes_with('wrote')[1] or {}
H.check('export of a capped result warns it is partial',
  (note.msg or ''):find('NOT the whole result', 1, true) and note.level == vim.log.levels.WARN, vim.inspect(H.notes))
H.check('export file written', vim.fn.filereadable(csv) == 1)

H.check('next query replaces it (old cursor released)', H.run_until('select 1 as a', '1/1 rows'), H.text())
H.check('no error notifications', #vim.tbl_filter(function(n) return n.level == vim.log.levels.ERROR end, H.notes) == 0, vim.inspect(H.notes))

H.done()
