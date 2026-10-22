# Gom logic trùng của Kafka/RabbitMQ/HTTP/Socket (2026-10-22)

Bước 3 của review kiến trúc 2026-10-22. Server có module riêng cho bốn connector không có ngôn ngữ query (`kafka.rs`, `rabbitmq.rs`, `http.rs`, `socket.rs`) và từng **chép lại** phần thuần từ connector crate của TUI, với lý do "crate đó toàn `pub(crate)` và kéo theo TUI". Các bản chép giống hệt nhau từng dòng:

- RabbitMQ: `QueueInfo`/`ExchangeInfo`/`BindingInfo`/`MessageInfo`, `RabbitTarget`, `parse_target` (gồm cả bản sửa giải mã phần trăm của #77 — phải sửa hai nơi), request có basic auth.
- HTTP: `parse_headers`, `resolve_url`. Socket: `hex_preview` + `HEX_PREVIEW_BYTES`. Kafka: `ephemeral_group_id` (khác mỗi tiền tố).

Đã làm (cùng khuôn với `tradar-query` ở bước 2):
- Mỗi crate: module dùng chung không UI, luôn biên dịch — `api.rs` (RabbitMQ, thêm `RabbitTarget::request`), `request.rs` (HTTP), `preview.rs` (Socket), `tail.rs` (Kafka, `ephemeral_group_id(prefix)`). Phần TUI (`Session`, `Connector`, event, `screen.rs`) chuyển sang `session.rs` sau feature `tui` (mặc định); `tradar-core`/`tradar-connector-spi`/ratatui/crossterm thành optional. Hàm/field riêng tư mà `screen.rs` dùng được nâng lên `pub(crate)` (trước đây cùng ở gốc crate nên tự thấy nhau). Test của crate khai báo trong `session.rs` để vẫn thấy phần riêng.
- Server: dùng bốn crate với `default-features = false`, xoá các bản chép (−183 dòng ở `src/`), bỏ phụ thuộc trực tiếp `percent-encoding`. `cargo tree -p tradar-server --features kafka` vẫn không có crate TUI nào.

Không gom: phần IO (`connect`, `publish`, `start_tail`, `send`) — bản server và bản TUI khác hình dạng thật (server trả JSON-RPC/push ngay, TUI đẩy qua hàng đợi `tick()`), gom lại sẽ phải viết lại TUI đã deprecated.
