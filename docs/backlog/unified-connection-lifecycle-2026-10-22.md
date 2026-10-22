# Vòng đời connection chung cho query và extension (2026-10-22)

Bước 4 của review kiến trúc 2026-10-22. Server có hai mô hình connection song song: connection truy vấn trong `State::drivers`, và mỗi extension (Kafka/RabbitMQ/HTTP/Socket/gRPC) tự giữ một `HashMap` riêng. Idle TTL (`idle-disconnect-2026-10-22.md`) và `disconnect` chỉ áp cho nhóm đầu.

Chọn mở rộng trait `Extension` thay vì dời state các extension về một registry trung tâm: mỗi extension vẫn giữ kiểu connection riêng (channel gRPC, client reqwest…), server chỉ cần biết tên và cách đóng.

- `Extension::connected()` / `close(name)` (mặc định: không có gì để đóng). Kafka/RabbitMQ/HTTP/gRPC implement; `<driver>.disconnect` của chúng gọi chính `close`. Socket không implement: "connection" của nó là tail đang chạy — đang được dùng, không bao giờ nhàn rỗi.
- `touch` ghi lần dùng cuối cho **mọi** request nêu `connection` (trước đây chỉ khi tên có trong `drivers`); `reap_idle` đóng thêm connection extension quá hạn; `disconnect` chung đóng cả connection extension (protocol 1.13).
- Đóng không cắt ngang việc đang chạy: call gRPC/stream và Kafka tail giữ bản sao client/broker riêng.
- gRPC: `grpc.load_proto` được nhớ theo tên connection (sống qua `close`), `grpc.connect` nạp lại — không thì idle TTL làm mất proto người dùng nạp tay.
- Plugin: `rpc.request` — call `<driver>.*` gặp lỗi "call `<driver>.connect` first" thì tự `<driver>.connect` rồi gửi lại một lần (buffer HTTP/gRPC/RabbitMQ để lâu vẫn gửi được; trước đây chỉ connection truy vấn có retry, trong `ctx.call`).

Test: `idle_reaping_and_disconnect_cover_extension_connections_too` (HTTP + SQLite), `rpc_grpc` (nạp lại proto sau `disconnect`), `idle_spec` (HTTP bị đóng rồi `http.send` vẫn trả 200; đỏ khi bỏ phần retry trong `rpc.request`).
