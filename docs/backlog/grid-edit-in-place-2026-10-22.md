# Sửa trực tiếp trên lưới kết quả (`:w` commit một lượt)

Thay luồng staged edit của `result-buffer-views-staged-edit-2026-10-20.md`: `i` mở prompt cho từng ô và `dd` từng dòng nên không dùng được các lệnh vim thật (`cw`, `.`, `:s`, `:g`, macro, `u`), và mỗi thao tác là một bước riêng.

## Thiết kế

- Khi kết quả sửa được (`edit.source` tìm ra một bảng + cột khoá nằm trong kết quả; `resolve_edit`, bất đồng bộ, cache theo `conn`+`query` để chạy lại cùng truy vấn thì sửa được ngay) buffer `tradar://results` là `buftype=acwrite`, `modifiable`. Không thì chỉ đọc và `i`/`dd`/`o`… (expr map) nói lý do.
- `:w` → `BufWriteCmd` → `M.write`: kiểm header/rule/footer không bị sửa, rồi `gridedit.diff` (hàm thuần, `nvim/lua/tradar/gridedit.lua`) so text lưới với các dòng đã vẽ. Mỗi dòng được **khớp theo giá trị cột khoá hiển thị**; `render.split_line` tách lại thành ô (cắt theo ` │ `), `render.display_cell` cho dạng hiển thị của ô gốc (nên ô bị cắt `…`, ô có xuống dòng/tab, `NULL` không bị coi là đã sửa).
- Kết quả diff: `updates` (ô đổi, mỗi ô một `UPDATE`), `deletes` (dòng gốc không còn), `inserts` (khoá lạ; ô trống bị bỏ → `DEFAULT`), `errors` (khoá trùng, không tách đủ cột, khoá hiển thị trùng giữa hai dòng gốc). Dòng trắng bị bỏ qua. Sửa chính cột khoá = `DELETE` + `INSERT` (thấy trong xem trước).
- Câu lệnh do server sinh (`edit.sql`, như trước — server chỉ sinh, không chạy), thứ tự update → delete → insert, rồi mở buffer xem trước `tradar://commit` (`filetype=sql`, chỉ đọc). `:w`/`<CR>` ở đó chạy tuần tự bằng `run_text(confirmed=true)` (hộp xem trước thay cho guard), xong chạy lại truy vấn gốc và đặt lại con trỏ; `q` huỷ. Lỗi giữa chừng → `flags.on_error` mở lại xem trước chỉ với các câu chưa chạy (không transaction, xem dưới).
- Trong lúc lưới bẩn: `M.more` (kể cả cuộn tới cuối), `toggle_view` và `cell_at` (nên `gyc`/`K`/`gd`/`gK`) bị chặn vì repaint/lệch số dòng sẽ mất sửa hoặc trỏ nhầm dòng; `show_result` cảnh báo khi bỏ sửa chưa commit; `<leader>ru` (`M.revert`) vẽ lại lưới.

- Lưới tài liệu (Mongo/ES, dạng bảng) cũng sửa được bằng cùng đường (`grid_of(r)` → `tcols`/`trows`/`twidths`; chỉ số cột khoá tính lại mỗi lần `:w` vì cột phẳng có thể đổi khi tải thêm trang). `UPDATE`/`DELETE` như trước; `INSERT` chưa hỗ trợ ở các connector này (`insert_sql` mặc định `None`) nên `:w` báo "cannot insert rows" và không mở xem trước.

## Vì sao khớp theo khoá

- Extmark `invalidate` chỉ có từ Neovim 0.10 và `cc` (thay cả dòng) sẽ bị nhầm là xoá.
- So theo vị trí dòng (`vim.diff`) ghép sai khi xoá một dòng và sửa dòng kề nó (hunk lẫn xoá+sửa) → `UPDATE` nhầm dòng.
- ID ẩn trong text (kiểu oil.nvim) làm lệch cột/`virtcol` mà `cell_at` đang dựa vào.
- Khoá hiển thị có sẵn trên dòng nên không cần trạng thái phụ, và đúng với mọi lệnh vim (kể cả `:g/pat/d`, `:sort`).

## Server (protocol 1.2)

`edit.sql` nhận thêm `insert: {cột: giá trị}` (không `key`/`change`): `QueryDriver::insert_sql(table, values)` mới (mặc định `None`), `query_driver::build_sql_insert`; Postgres/MySQL/SQLite ghi đè, các connector khác trả `sql: null` → plugin báo "cannot insert rows". `PROTOCOL_VERSION` 1.1 → 1.2 (thêm thuần tuý).

## Chưa làm / hạn chế

- ~~Một dòng đổi k ô sinh k câu `UPDATE`~~ — đã gộp, xem `update-multi-cell-2026-10-29.md`.
- ~~Không transaction~~ — đã làm, xem `commit-transaction-json-view-fixes-2026-10-23.md`.
- Lưới không tự căn lại cột khi gõ ô dài hơn (căn lại ở lần vẽ sau commit/revert).
- Giá trị có khoảng trắng đầu/cuối bị cắt (trim) khi so và khi ghi.

## Kiểm chứng

- `gridedit.diff`: 10 ca (nguyên vẹn gồm ô bị cắt/xuống dòng/`NULL`, đổi ô, `dd`, xoá + sửa dòng kề, dòng mới, `yyp` trùng khoá, `:sort`, tách sai, dòng trắng, sửa khoá).
- Smoke test headless Neovim + `tradar-server` thật + SQLite: sửa 2 ô + `dd` + dòng mới → xem trước đúng 4 câu, DB chưa đổi; `<CR>` commit cả 4 trong một lượt và lưới tải lại, sửa được ngay; `revert`; sửa header bị chặn; `:w` trong xem trước; lỗi ở `INSERT` (`NOT NULL`) → xem trước mở lại với phần chưa chạy; kết quả join/thiếu cột khoá chỉ đọc và `dd` nói lý do.
- Lưới tài liệu: smoke test với `rpc` được stub (Mongo giả): sửa ô + xoá dòng → xem trước đúng; thêm dòng → bị chặn vì connector không insert.
- `cargo test -p tradar-server --test rpc` (thêm ca `insert`), `-p tradar-query-workbench` (`build_sql_insert`).

## Bổ sung: thứ tự cột của INSERT (2026-10-31)

Câu `INSERT` từ dòng thêm tay trước đây có thứ tự cột ngẫu nhiên (object JSON không có thứ tự). Plugin gửi thêm `insert_columns` (tên cột theo thứ tự lưới), server sắp lại bằng `order_insert_values` (protocol 1.8); cột không được nêu giữ thứ tự tương đối ở cuối.
