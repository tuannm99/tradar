# Redis: nháy trong lệnh; statusline

## Redis

`execute` tách lệnh bằng `split_whitespace` nên `SET greeting "hello world"` thành 3 đối số (`"hello`, `world"`) — không đặt được giá trị có dấu cách. `split_command` (redis-cli style) thay thế: token mở đầu bằng `"`/`'` chạy tới nháy đóng; escape `\n \r \t \b \a \\ \" \xHH` trong `"…"`, `\'` trong `'…'`; nháy giữa token là ký tự thường (`SET k it's` như cũ); nháy không đóng / ký tự dính sau nháy đóng → lỗi rõ ràng. Ngược lại `quote_arg` đặt nháy cho key có dấu cách/nháy/backslash trong snippet navigator (`GET "my key"`), round-trip qua `split_command` (test). Connector dùng chung với TUI nên sửa này đến cả hai.

## Statusline (`require('tradar').status()`)

- Trong buffer kết quả lấy connection của *kết quả* (`state.result.conn`) thay vì connection "chọn gần nhất".
- `✎ unsaved edits` khi lưới có sửa chưa commit.
- Badge `tx` cập nhật ngay sau `BEGIN`/`COMMIT`/`ROLLBACK`/`START`/`END`/`ABORT` bằng một lời gọi `status` (trước đó chờ chu kỳ ping 15 giây).

Kiểm chứng: headless Neovim + `tradar-server` thật + SQLite (badge `tx` hiện/tắt ngay, kết quả đúng connection, cờ unsaved); 6 unit test mới cho `split_command`/`quote_arg`. Test Docker của Redis không chạy được ở môi trường này.
