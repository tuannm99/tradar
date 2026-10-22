# Đợt UI/UX plugin sau review kiến trúc (2026-10-22)

User hỏi "phần UI/UX cần gì không"; rà plugin, chọn làm cả 5 mục:

1. **Cột giữa bị cắt 40 ký tự** (ClickHouse "report disk"): `zw` hiện trọn cột dưới con trỏ (tới `MAX_LAST_COLUMN_WIDTH` = 400, như cột cuối), bấm lại thu về; `zW` mọi cột. Lưu theo kết quả (`r.wide`/`r.twide` cho bảng tài liệu), kết quả mới bắt đầu lại từ đầu. Không làm khi lưới có sửa chưa commit (vẽ lại sẽ mất sửa). Con trỏ ở lại đúng cột. Sửa kèm chữ hỏng `ctx.show` trong `g?`.
2. **Statusline `✗` khi chỉ bị đóng vì idle**: poll gặp "not connected" giờ đặt `state.idle` (hiện `(idle)`), không đặt `alive = false`; `✗` chỉ còn cho ping thất bại thật. Kết nối lại thì bỏ `(idle)`.
3. **Tab-complete tên connection** cho 11 lệnh nhận connection, lọc theo loại (`'query'` cho `:TradarConnect`). Lấy từ server (`connections.list`, đồng bộ 300 ms) nếu đã nối, không thì đọc thẳng `connections.toml` — bấm Tab không được khởi động server. Chỉ gợi ý ở tham số đầu.
4. **Thông báo lẫn ngôn ngữ**: các lỗi tiếng Việt trong server (cả module gRPC: metadata/TLS/timeout/schema/stream, và "không kết nối được tới" của Kafka/RabbitMQ/Socket/gRPC) chuyển sang tiếng Anh như phần còn lại.
5. **`:Tradar <lệnh-con>`** (`commands.lua`): suy ra từ chính các lệnh `:Tradar<Words>` đang có (`TradarKafkaTail` → `kafka tail`), khớp dài nhất trước (`run all` trước `run`), chuyển range/`!`/tham số. Completion: từ kế tiếp, rồi completion của chính lệnh đã khớp (gộp cả hai khi một lệnh là tiền tố của lệnh khác). Lệnh cũ giữ nguyên.

Test: `columns_spec` (12 kiểm tra), `commands_spec` (16), `idle_spec` thêm `(idle)` không `✗` rồi mất khi nối lại.
