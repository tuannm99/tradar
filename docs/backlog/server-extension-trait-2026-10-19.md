# Tách connector phi-`QueryDriver` khỏi `server.rs` bằng trait `Extension` (2026-10-19)

Refactor thuần, không đổi hành vi wire. Trước đó `server.rs` (1426 dòng, 80 chỗ `#[cfg(feature)]`) giữ state của cả 5 connector phi-`QueryDriver` (gRPC/Kafka/Socket/RabbitMQ/HTTP) trong `State`, và mỗi lần thêm một connector phải sửa cùng lúc 5 chỗ: field `State`, nhánh `dispatch`, handler, cặp hàm `connected_X` (một bản có cfg, một bản không), nhánh `is_supported`.

- **`extension.rs`**: trait `Extension` (`driver`, `handle(method, params, ctx)`, `is_connected`, `client_disconnected`, `cancel`) + `Ctx` (store, client) + `all()` — nơi duy nhất chứa feature gate của các extension.
- Mỗi module (`grpc.rs`/`kafka.rs`/`socket.rs`/`rabbitmq.rs`/`http.rs`) có struct `XExtension` giữ state riêng và xử lý method `<driver>.*`. `server.rs` chỉ route theo tiền tố (`dispatch_extension`).
- `str_param`/`u64_param`/`usize_param` chuyển sang `protocol.rs` (`pub(crate)`) để extension dùng chung.
- Kết quả: `server.rs` 1426 → 658 dòng, `#[cfg]` 80 → 3.

Khác biệt nhìn thấy được duy nhất: lỗi "no saved … connection named" của gRPC giờ viết `grpc` thường (trước là `gRPC`) vì dùng chung `Ctx::saved`.

Chưa làm (mục 2 trong đề xuất cùng đợt): Kafka và Socket vẫn lặp quản lý subscription (`tails`/`by_client`/bộ đếm id) — có thể gộp thành `Subscriptions<Handle>` generic.

Xác nhận: `cargo build`/`clippy --all-targets -- -D warnings` sạch ở 4 cấu hình (mặc định, `--features kafka`, `--no-default-features --features postgres,sqlite`, `--features sqlite`); `make test-unit` xanh với số test không đổi.
