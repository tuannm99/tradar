# Buffer kết quả: JSON view cho SQL, row view, sửa theo kiểu staged + commit

Từ đợt rà UI/UX buffer kết quả của plugin (`nvim/lua/tradar/init.lua`).

## Đã làm

- **JSON view cho kết quả SQL**: `<leader>rv` (`M.toggle_view`) trước chỉ dùng được cho kết quả dạng tài liệu; nay `kind == 'table'` cũng đổi bảng ↔ JSON (`render.json`, mảng object). Ở view JSON `tabular()` trả `nil` nên các thao tác theo ô (`i`/`dd`/`gy*`/`gd`/`K`) tạm tắt, như view JSON của tài liệu. Sửa luôn comment cũ ghi nhầm phím `gT` (phím thật là `<leader>rv`; `gT` bị bỏ vì che lệnh tab có sẵn).
- **Row view `gK`** (`M.row_view`): float hiện dòng hiện tại, mỗi trường một dòng (kiểu `\x` của psql) — `K` chỉ cho xem một ô.
- **Staged edit + commit**: `i`/`dd` không còn hiện hộp `Run` cho từng thao tác; chúng chỉ gọi `edit.sql` (server vẫn chỉ sinh câu lệnh, không chạy) rồi đẩy vào `state.result.pending`, dòng được đánh dấu bằng extmark (`● cột = giá trị` / `✗ delete`). `<leader>rw` (`M.commit`) liệt kê mọi câu lệnh và chỉ chạy, tuần tự, sau khi chọn `Commit N change(s)`; `<leader>ru` (`M.discard`) bỏ hết. Quy tắc hàng đợi: sửa lại cùng ô thì thay thế; xoá dòng thay các sửa của dòng đó; dòng đã xếp xoá thì không sửa tiếp được.

## Quyết định

- Hộp xác nhận của `commit` thay cho guard (như hộp `Run` cũ) nên connection `protected` chỉ bị hỏi một lần.
- Không bọc transaction: `Session`/`QueryDriver` không có khái niệm transaction qua nhiều lần `execute`; commit dừng ở câu lỗi đầu tiên, các câu đã chạy giữ nguyên. Nếu cần all-or-nothing thì phải thêm phương thức batch ở server.
- Hàng đợi sống trong kết quả hiện tại: chạy truy vấn khác sẽ bỏ các thay đổi chưa commit.

## Kiểm chứng

Smoke test headless Neovim với `tradar-server` thật + SQLite: toggle JSON/bảng, row view, stage sửa + xoá (đánh dấu, DB chưa đổi), `Cancel` giữ nguyên DB, `Commit` áp dụng cả hai và tải lại 2 dòng, `discard` xoá đánh dấu.
