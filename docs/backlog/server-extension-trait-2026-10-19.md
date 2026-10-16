# Tách connector phi-`QueryDriver` khỏi `server.rs` bằng trait `Extension` (2026-10-19)

Refactor thuần, không đổi hành vi wire. Trước đó `server.rs` (1426 dòng, 80 chỗ `#[cfg(feature)]`) giữ state của cả 5 connector phi-`QueryDriver` (gRPC/Kafka/Socket/RabbitMQ/HTTP) trong `State`, và mỗi lần thêm một connector phải sửa cùng lúc 5 chỗ: field `State`, nhánh `dispatch`, handler, cặp hàm `connected_X` (một bản có cfg, một bản không), nhánh `is_supported`.

- **`extension.rs`**: trait `Extension` (`driver`, `handle(method, params, ctx)`, `is_connected`, `client_disconnected`, `cancel`) + `Ctx` (store, client) + `all()` — nơi duy nhất chứa feature gate của các extension.
- Mỗi module (`grpc.rs`/`kafka.rs`/`socket.rs`/`rabbitmq.rs`/`http.rs`) có struct `XExtension` giữ state riêng và xử lý method `<driver>.*`. `server.rs` chỉ route theo tiền tố (`dispatch_extension`).
- `str_param`/`u64_param`/`usize_param` chuyển sang `protocol.rs` (`pub(crate)`) để extension dùng chung.
- Kết quả: `server.rs` 1426 → 658 dòng, `#[cfg]` 80 → 3.

Khác biệt nhìn thấy được duy nhất: lỗi "no saved … connection named" của gRPC giờ viết `grpc` thường (trước là `gRPC`) vì dùng chung `Ctx::saved`.

## Gộp quản lý subscription của Kafka và Socket

`subscriptions.rs`: `Subscriptions<H, M>` — id (`<prefix>-<n>`), ownership theo client, `remove`/`remove_client`. `M` là metadata đi kèm (Socket lưu tên connection để `connections.list` báo `connected`; Kafka dùng `()`). Hai hàm gỡ **trả handle về** thay vì tự `stop()`, để caller nhả lock state trước khi abort task — giữ đúng thứ tự cũ. Thay cho bộ ba `tails`/`by_client`/`next_subscription` chép y hệt ở cả hai module (kèm hai bản `*_forget_client`). 4 unit test mới cho chính cấu trúc này.

Nhân tiện bắt được và sửa 3 cảnh báo dead-code chỉ lộ ra ở tổ hợp feature hẹp (`not_connected`, `Ctx::client`, các field/method của `Subscriptions`) — build/clippy mặc định không thấy, nên giờ clippy được chạy ở 8 tổ hợp: mặc định, `--features kafka`, và `sqlite` + từng extension riêng lẻ (`http`/`grpc`/`rabbitmq`/`socket`/`kafka`) + không extension nào.

Xác nhận: `clippy --all-targets -- -D warnings` sạch ở 8 tổ hợp feature nêu trên; `make test-unit` xanh, số test tích hợp không đổi (rpc 17, grpc 6, http 4, rabbitmq 5, socket 6), test lib 26 → 30.
