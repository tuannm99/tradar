# RabbitMQ qua server (2026-10-16)

Khác hẳn Kafka/Socket (`kafka-tail-server-2026-10-15.md`, `socket-tail-server-2026-10-15.md`): RabbitMQ **không cần push-channel**. Management HTTP API không có endpoint stream (xem `tradar-connector-rabbitmq`'s doc comment, đã ghi non-goal "tail real-time qua AMQP" từ đầu), nên mọi thao tác ở đây là request/response thuần — đúng hình dạng `grpc.rs` đã dùng, không có `ClientConnection`/notification nào liên quan.

## Server (`tradar-server`)

- **`rabbitmq.rs`** (file mới, không dùng lại `tradar-connector-rabbitmq` — type của crate đó `pub(crate)`-scoped cho `Screen` riêng, và kéo cả crate vào chỉ để dùng vài lời gọi `reqwest` nặng hơn viết lại trực tiếp): `parse_target`/`RabbitTarget` chép nguyên logic parse URL (vhost/auth/base_url) của TUI — phần duy nhất không tầm thường (~25 dòng), không đáng kéo cả crate chỉ để dùng nó. `RabbitConnection` bọc `reqwest::Client` + `RabbitTarget`, `#[derive(Clone)]` — rẻ để clone ra khỏi `Mutex<State>` guard trước khi `.await` thật, vì một `std::sync::MutexGuard` không thể sống qua điểm `.await` (khác Kafka/Socket's `TailHandle`, vốn đã tự bọc `Arc` sẵn — ở đây không có gì async bên trong `RabbitConnection` để cần `Arc`, clone cả struct là đủ rẻ).
- **7 method mới**: `rabbitmq.connect`/`rabbitmq.disconnect` (y hệt hình dạng `grpc.connect`/`grpc.disconnect` — probe `/api/overview` lúc connect, idempotent, trả `{connected, already}`/`{disconnected}`), `rabbitmq.queues {connection}` → `[QueueInfo]`, `rabbitmq.exchanges {connection}` → `[ExchangeInfo]` (exchange mặc định không tên bị lọc, giống TUI's `refresh_exchanges`), `rabbitmq.bindings {connection, exchange}` → `[BindingInfo]`, `rabbitmq.peek {connection, queue}` → `[MessageInfo]` (snapshot một lần, `ackmode=ack_requeue_true` nên không tiêu huỷ message), `rabbitmq.publish {connection, exchange, routing_key, payload}` → `{ok: true}`.
- **Feature `rabbitmq`** (`reqwest`+`serde` optional dependency, cả hai đã có sẵn cho gRPC/kênh HTTP khác trong crate) — như `socket`, không cần toolchain gì thêm, nên nằm trong `default`.
- `is_supported`/`connections()`'s cờ `connected`/`connected_rabbitmq` theo đúng mẫu `connected_grpc`/`connected_kafka`/`connected_socket` đã có.

## Plugin Neovim

- **`rabbitmq.lua`** (mới) — không dựng sidebar riêng như TUI's `RabbitScreen` (hai mode Queues/Exchanges + overlay compose): mỗi thao tác qua `vim.ui.select` (chọn queue/exchange nếu không truyền tường minh) rồi hiện kết quả trong một split dùng lại được (không sửa được, `q`/`g?`) — cùng idiom `tradar.grpc`/`tradar.kafka` đã dùng cho "chọn một tài nguyên không phải kết quả SQL, không cần buffer nguồn riêng".
- 4 command: `:TradarRabbitmqConnect [tên]`, `:TradarRabbitmqPeek [tên] [queue]`, `:TradarRabbitmqBindings [tên] [exchange]`, `:TradarRabbitmqPublish [tên]` (hỏi exchange/routing-key/payload tuần tự qua `vim.ui.input`). Không có `*.tail.*` nào — không tail, không push.
- Nhân tiện sửa một lỗi tài liệu từ PR Socket tail trước đó: README ghi buffer Socket tail "không sửa được nhưng hai chiều" — mâu thuẫn, vì buffer đó thực ra **sửa được** (đúng `socket.lua`'s code, không có dòng `modifiable = false` nào). Sửa lại README cho đúng code.

## Xác nhận

- `cargo build`/`clippy --all-targets -- -D warnings`/`fmt --all -- --check` sạch ở ba cấu hình: workspace mặc định (có `rabbitmq`), `-p tradar-server --features kafka` (cả bốn feature), và `--no-default-features --features postgres,sqlite` (không cái nào).
- `make test-unit` xanh (167 test TUI, và `tradar-server` nằm trong `cargo test --workspace` mặc định của `test-unit` — không bị loại trừ như 10 connector crate kia — nên 6 test rabbitmq cũng chạy luôn trong đó, không cần mục riêng).
- `cargo test -p tradar-server`: 18 unit test (bao gồm 6 test mới cho `parse_target`/struct parsing trong `rabbitmq.rs`) + 17 + 6 + 5 (RabbitMQ) + 6 (Socket) test tích hợp, tất cả xanh.
- **Test end-to-end không cần Docker lẫn broker giả lập** (`tests/rpc_rabbitmq.rs`, 5 bài): một HTTP/1.1 server viết tay tối giản (đọc request line + `Content-Length` rồi trả JSON cố định theo route — không axum/hyper, cả hai chỉ là dependency gián tiếp ở chỗ khác trong workspace, không đáng thêm trực tiếp cho vài route cố định) đứng thay Management API thật — cùng tinh thần "không cần Docker cho một HTTP client mỏng" `tests/rpc_socket.rs` đã đặt tiền lệ với TCP echo server. Cover: connect/already-connected, queues/exchanges (xác nhận exchange mặc định bị lọc), bindings/peek/publish round-trip, disconnect (và gọi lại sau khi disconnect là lỗi sạch, không panic), lỗi kết nối thất bại.
- Headless Neovim thật: `tradar-server` thật + một mock Management API Python (`http.server`, bảng route cố định, ~40 dòng) — `:TradarRabbitmqPeek` hiện đúng payload, `:TradarRabbitmqBindings` thay nội dung split đúng, `:TradarRabbitmqPublish` chạy qua chuỗi `vim.ui.input` giả lập (`exchange`/`routing_key`/`payload`), `connections.list` báo đúng trạng thái connected — 6/6 kiểm tra qua.

## Chưa làm

- HTTP qua server — không cần push, có thể làm độc lập bất cứ lúc nào.
- Giới hạn gì đó cho số connection RabbitMQ đồng thời — chưa cần, số lượng tự nhiên bị chặn bởi số connection saved trong `connections.toml`, không giống Kafka/Socket's subscription có thể sinh vô hạn theo mỗi lần gọi `tail.start`.
