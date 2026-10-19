# Huỷ query báo lỗi; server khởi động lại phải chạy hai lần

Rà luồng chạy/lỗi bằng cách thử các tình huống hỏng thật (headless Neovim + `tradar-server` thật + SQLite).

## Huỷ query chậm hiện là lỗi

`<leader>rx` trên query chậm báo `error` (status `db local · error`, `show_message` kèm `(code: 9) interrupted`) thay vì `cancelled`. `cancel` đặt permit `Notify` *rồi* gọi `driver.cancel_query()`; câu bị ngắt thất bại ngay (SQLite: "interrupted") nên cả hai nhánh của `select!` trong `execute` sẵn sàng cùng lúc và tokio chọn ngẫu nhiên. Sửa: `select! { biased; cancel => None, execute => … }` — lần huỷ của user luôn thắng. Test `cancelling_a_running_query_reports_cancelled_not_the_drivers_interrupt_error` (5 vòng; fail ở vòng 0 khi bỏ `biased`).

## Server khởi động lại: lần chạy đầu luôn lỗi

Server bị kill/khởi động lại → `state.connected[conn]` của plugin vẫn `true`; `M.run` gọi `split` đầu tiên và nhận `` `local` is not connected `` (chỉ nhánh `execute` mới có nhánh thử lại). `call` giờ xử lý chung: lỗi "not connected" trên connection mà plugin tưởng đã kết nối → xoá cờ, `ensure_connected`, lặp lại đúng một lần (không áp dụng cho chính `connect`, không vòng lặp). Mọi lời gọi (split, schema, completion, hover, edit.source…) được hưởng.

Còn lại không đổi (đã thử, ổn): lỗi cú pháp hiện thông điệp + dấu `^` và diagnostic tại câu; chạy chồng bị từ chối kèm gợi ý `<leader>rx`.
