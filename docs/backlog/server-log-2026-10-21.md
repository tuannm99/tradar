# Log của tradar-server + `:TradarLog` (2026-10-21)

User hỏi "làm sao xem log tradar server" — chưa có: server do plugin tự chạy `detach` (stdout/stderr bỏ đi) và chỉ in một dòng `listening on …`.

- `tradar-server/src/logging.rs`: `tracing` + `tracing-subscriber` ghi file (`$XDG_STATE_HOME/tradar/server.log`; `TRADAR_LOG_FILE`, `TRADAR_LOG` đổi đường dẫn/mức). Không bao giờ fatal: không mở được file thì server vẫn chạy. Quay vòng đơn giản: > 5 MB thì lúc khởi động đổi thành `.1`.
- `Server::handle` ghi `method [connection] ok|failed <ms>`; `status`/`version`/`complete`/`keywords`/`split`/`fetch`/`cursor.close` ở `debug` (tránh ngập log vì poll nền). Lỗi cắt dòng đầu, tối đa 300 ký tự. Câu lệnh và mật khẩu không bao giờ vào log (kiểm tra: `select … secret_marker` và `password = 'hunter2'` không xuất hiện).
- Protocol 1.11: `version` trả thêm `log_path`.
- Plugin: `lua/tradar/serverlog.lua` (`:TradarLog`): split dưới, `autoread` + timer 1 s `checktime`, bám dòng mới khi con trỏ ở dòng cuối; `:checkhealth tradar` hiện đường dẫn.

**Lỗi có sẵn phát hiện khi viết log dòng "client N disconnected" (đã sửa):** `serve_connection` đợi `writer.await` *trước* khi gọi `client_disconnected`, nhưng `client` (và các subscription nền) giữ đầu gửi của kênh nên writer không bao giờ kết thúc → mỗi socket đã đóng để lại một task treo và subscription Kafka/Socket tail không bị abort. Giờ dọn trước (`drop(in_flight)` → `client_disconnected`), rồi chờ writer tối đa 2 s. Thêm `Server::connected_clients()`; test `a_closed_socket_is_cleaned_up_even_with_replies_pending` (thất bại ở thứ tự cũ).
