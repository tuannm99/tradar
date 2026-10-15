# Kafka tail qua server (2026-10-15)

Hiện thực đúng thiết kế đã chốt ở `push-channel-design-2026-10-14.md` — xem đó cho 5 quyết định kiến trúc, không lặp lại ở đây. Mục này chỉ ghi lại phần code thật và những gì khác với bản thiết kế.

## Server (`tradar-server`)

- **`ClientConnection`** (`crates/tradar-server/src/server.rs`) — abstraction mới cho "connection id" mà bản thiết kế gọi tên nhưng chưa thiết kế chi tiết: `{id: u64, outbox: mpsc::UnboundedSender<String>}`. Một cái mới mỗi connection socket (`transport.rs`'s `serve_connection` gọi `server.new_client(replies.clone())`), clone vào từng request task (`Server::handle`/`dispatch` nay nhận thêm `client: &ClientConnection`), và `server.client_disconnected(&client)` sau khi vòng đọc dừng — dọn mọi subscription (nền tảng cho Socket tail sau này, không chỉ Kafka). `ClientConnection::detached()` (id giả, receiver drop ngay) cho mọi test/caller lái `Server::handle` không qua socket thật — hai file test cũ (`tests/rpc.rs`, `tests/rpc_grpc.rs`) chỉ cần sửa `call()` helper của chúng, không phải từng test case.
- **`kafka.rs`** (file mới) — không tái dùng `tradar-connector-kafka` (lý do: xem doc comment đầu file, cùng lý do `grpc.rs` đã không tái dùng connector crate nào). `connect`/`list_topics` giữ nguyên logic probe-metadata/lọc topic nội bộ (`__`-prefix) của TUI. `start_tail` sinh hai task nối bằng `Arc<Mutex<VecDeque<Value>>>` cap 500 + `Arc<Notify>`: `reader` chỉ đọc Kafka (không bao giờ block chờ `push`), `forwarder` chỉ gọi `push`, bỏ tin cũ nhất khi buffer đầy thay vì block `reader`.
- **5 method mới**: `kafka.connect`/`kafka.disconnect`/`kafka.topics`/`kafka.tail.start`/`kafka.tail.stop`. `kafka.tail.start` trả về `{subscription}` ngay, message thật tới sau dưới dạng `kafka.tail.message` notification (`{subscription, partition, offset, key, value}`, hoặc `{subscription, error}` khi reader lỗi). `State` thêm `kafka_tails`/`kafka_tails_by_client`/`next_kafka_subscription`.
- Feature `kafka` (không trong `default`, cùng lý do `tradar-app`'s `kafka` feature).

## Plugin Neovim

- **`rpc.lua`**: `M.on(method, cb)` — bảng đăng ký theo method, trả về hàm unsubscribe. `on_line` rẽ nhánh `msg.id == nil and msg.method` (notification) trước khi tra `pending` (reply).
- **`kafka.lua`** (mới, cùng hình dạng module với `grpc.lua`): `:TradarKafkaConnect [tên]`, `:TradarKafkaTail [tên] [topic]` — thiếu gì hỏi nấy (connection qua picker lọc `driver = "kafka"`, topic qua `kafka.topics`, luôn hỏi `latest`/`earliest`). Mở một buffer transcript cuộn, không sửa được, tab riêng; mỗi buffer tự nhớ `subscription` của mình để lọc đúng notification giữa nhiều tail chạy song song. `BufWipeout` (kích hoạt khi đóng cửa sổ cuối cùng, vì `bufhidden = wipe`) tự `kafka.tail.stop` + gỡ `rpc.on` — `q` là đủ, không cần lệnh huỷ riêng như gRPC's `<leader>rx` (tail không có "đang chạy" để huỷ, chỉ có "đang tail" để dừng hẳn).

## Xác nhận

- `cargo build -p tradar-server --features kafka` và không có `--features kafka` (cfg-gating đúng cả hai chiều) — cả hai sạch.
- `cargo clippy --all-targets --workspace -- -D warnings` và riêng `-p tradar-server --features kafka` — sạch. `ClientConnection`'s field/method riêng cho push mang `#[cfg_attr(not(feature = "kafka"), allow(dead_code))]` vì build mặc định (không Kafka, chưa Socket) chưa có ai gọi `notify`/đọc `id`/`outbox` — bỏ đi khi Socket tail hiện thực.
- `cargo fmt --all -- --check`, `make test-unit` (167+ test TUI, 9+17+6 test `tradar-server` — bao gồm 2 test mới cho `push_notification`'s ring buffer, phần duy nhất test được không cần broker sống).
- Headless Neovim thật (`nvim --headless`): `rpc.lua`/`kafka.lua`/`init.lua`/`grpc.lua` load sạch, `plugin/tradar.lua` đăng ký đúng 2 command mới.

## Chưa làm

- Test end-to-end thật (connect → tail → nhận message → dọn khi disconnect) cần một Kafka broker sống — không có Docker trong sandbox lúc code đợt này, giới hạn giống mọi connector test Docker-gated khác trong workspace.
- Socket tail — cùng hạ tầng `ClientConnection`/push-channel vừa dựng, chưa code.
- RabbitMQ/HTTP qua server — không cần push, có thể làm độc lập bất cứ lúc nào.
- Giới hạn số subscription đồng thời (như `MAX_CURSORS`) — bản thiết kế đã ghi là chưa quyết, vẫn chưa phải vấn đề ở quy mô dùng thật hiện tại.
