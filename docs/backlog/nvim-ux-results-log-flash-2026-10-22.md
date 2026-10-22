# Giữ nhiều kết quả + nháy câu đang chạy (2026-10-22)

User: "cứ tiếp tục cải thiện UX". Rà plugin như người mới dùng, hai chỗ vướng thật:

**1. `<leader>ra` (run all) chỉ còn kết quả cuối.** Mỗi câu ghi đè kết quả trước trong cùng một buffer; muốn xem câu đầu phải chạy riêng. Giờ `state.result_log` giữ `results_kept` (10) kết quả gần nhất (cả `affected`), `state.result_pos` là cái đang xem; `[r`/`]r` (buffer kết quả) và `:TradarResults` (`vim.ui.select`). Footer `[i/n] <query> · …`.
- Server không đổi, bộ nhớ server không tăng: `release_result` đóng cursor của **mọi** kết quả trừ cái mới (trước đây chỉ đóng cái đang hiện), đánh dấu `closed`; kết quả cũ chỉ còn phần đã tải. Hệ quả có chủ đích: cũ thì chỉ đọc (`edit = nil`, vì DB có thể đã đổi), `more` từ chối kèm lý do thay vì treo (vòng `all_rows` phải tự thoát — dễ thành vòng lặp vô hạn), export/yank-all cảnh báo "NOT the whole result".
- Không chuyển khi lưới có sửa chưa commit (vẽ lại sẽ mất sửa). `show_message` (EXPLAIN văn bản…) đặt `result_pos = nil`, `[r` vẫn về kết quả cuối.
- Không làm sắp xếp/lọc dòng phía client như TUI: kết quả mặc định chỉ tải 20 dòng, sắp xếp 20 dòng dễ gây hiểu nhầm là sắp xếp cả bảng; ORDER BY/WHERE đúng chỗ hơn. `/` của vim đã tìm trong lưới.

**2. Không biết câu nào vừa chạy.** Trong file nhiều câu, `<leader>rr` không phản hồi gì cho tới khi kết quả về. Giờ câu vừa gửi được tô `TradarRunning` (link `IncSearch`, `default`) 200 ms (`flash_ms`, 0 = tắt) bằng extmark; với `run all` mỗi câu nháy khi nó bắt đầu. Đánh dấu theo văn bản gốc trong buffer, kể cả khi `transform` (EXPLAIN) gửi đi văn bản khác. `run_text` chỉ nháy khi người gọi truyền `flags.flash` (open_table/CRUD… không có vị trí trong buffer).

Test: `resultlog_spec` (20), `flash_spec` (9).
