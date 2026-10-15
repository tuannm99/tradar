# Socket tail qua server (2026-10-15)

Cùng ngày với `kafka-tail-server-2026-10-15.md`, cùng hạ tầng (`ClientConnection`, push-channel) — xem file đó cho 5 quyết định kiến trúc chung, không lặp lại. Mục này chỉ ghi lại chỗ Socket khác Kafka và phần code thật.

## Khác Kafka ở đúng một chỗ

Một "connection" Socket **là** chính phiên TCP, không phải một tài nguyên dùng lại được như broker string của Kafka (`KafkaConnection { brokers }` sống qua nhiều lần tail cùng kết nối). Vì vậy:

- **Không có `socket.connect` riêng** — `socket.tail.start` dial TCP và bắt đầu tail trong cùng một lời gọi.
- Gọi `socket.tail.start` hai lần trên cùng saved connection mở **hai phiên TCP độc lập hoàn toàn**, không chia sẻ gì — đúng tinh thần "mỗi subscription một task nền riêng" của thiết kế gốc, và cũng là lựa chọn duy nhất hợp lý: không có cách nào chia byte của một `TcpStream` cho hai reader mà không đua nhau đọc.
- `socket.send` vừa ghi vào TCP stream vừa tự đẩy một notification `direction: "sent"` — khác Kafka (chỉ có một chiều, không có khái niệm "gửi"). Plugin nhờ vậy không cần tự echo lại chữ vừa gõ, giống cách `tradar-connector-socket`'s `push_entry` trong TUI log cả hai chiều.

## Server (`tradar-server`)

- **`socket.rs`** (file mới, song song `kafka.rs`, không dùng lại `tradar-connector-socket`'s `SocketSession` — cùng lý do `kafka.rs` không dùng `KafkaSession`): cùng shape `reader`/`forwarder` + ring buffer cap 500 (`MAX_BUFFERED_MESSAGES`). `chunk_text`/`hex_preview`/`looks_like_text` chuyển nguyên logic "text khi là UTF-8 hợp lệ, hex preview khi không" của `tradar-connector-socket`'s `format_entry` sang phía server, vì JSON không có kiểu byte-string để gửi nguyên byte nhị phân.
- **`TailHandle`** bọc `Arc<Inner>` (write half + 2 task handle) thay vì sở hữu trực tiếp — `Clone` rẻ để `socket_send` lấy một bản sao ghi được trong khi bản gốc vẫn nằm trong `State::socket_tails`; `JoinHandle::abort` nhận `&self` nên `stop(&self)` không cần tiêu thụ bản cuối cùng.
- **3 method mới**: `socket.tail.start {connection}` → `{subscription}`; `socket.send {subscription, text, newline}` → `{sent}`; `socket.tail.stop {subscription}` → `{stopped}`. `State` thêm `socket_tails: HashMap<subscription, (client_id, connection_name, TailHandle)>` (tên connection đi kèm ngay trong map này, khác Kafka — không có state riêng per-name để tra) + `socket_tails_by_client` + `next_socket_subscription`.
- **Feature `socket`** — nằm trong `default` (khác `kafka`): không cần dependency/toolchain gì thêm, `tokio::net::TcpStream` đã là dependency sẵn của crate.
- `ClientConnection`'s `#[cfg_attr(not(feature = "kafka"), ...)]` mở rộng thành `not(any(feature = "kafka", feature = "socket"))` — giờ chỉ build tắt cả hai mới thật sự không dùng tới `notify`/`id`/`outbox`.

## Plugin Neovim

- **`socket.lua`** (mới, cùng hình dạng module với `kafka.lua`): `:TradarSocketTail [tên]` mở ngay một buffer **sửa được** (khác hẳn `kafka.lua`'s buffer chỉ đọc) ở chế độ insert sẵn — gõ một dòng, `<CR>` ở dòng cuối cùng gửi (`M.send`), `<CR>` ở chỗ khác là xuống dòng bình thường (để cuộn/yank transcript không bị cản). `›`/`‹` đánh dấu chiều gửi/nhận, cùng ký hiệu `tradar-connector-socket`'s `format_entry`. `q`/`g?`/`BufWipeout` tự dọn giống hệt `kafka.lua`.

## Xác nhận

- `cargo build`/`clippy --all-targets -- -D warnings`/`fmt --all -- --check` sạch ở cả ba cấu hình: workspace mặc định (có `socket`, không `kafka`), `-p tradar-server --features kafka` (cả hai), và `--no-default-features --features postgres,sqlite` (không cái nào).
- `make test-unit` xanh (167 test TUI) và `cargo test -p tradar-server` xanh — bao gồm:
  - 5 unit test thuần Rust trong `socket.rs` (ring buffer, `chunk_text`, và **2 test dùng một TCP echo server thật** — `start_tail` dial thành công/thất bại, gửi/nhận qua một round-trip mạng thật).
  - **6 bài test tích hợp mới** (`tests/rpc_socket.rs`), đi thẳng qua `Server::handle`/`Server::new_client` với một TCP echo server thật (`tokio::net::TcpListener` trần, không cần Docker hay broker giả lập) — đây là chỗ Socket **hơn hẳn** Kafka: cover được toàn bộ vòng đời thật mà Kafka's backlog entry phải ghi "chưa làm được" (dial thành công/thất bại, gửi+nhận notification đúng cả hai chiều mà không giả định thứ tự giữa chúng — "received" phụ thuộc network round-trip thật, không có gì đảm bảo nó tới sau "sent"), `connections.list`'s cờ `connected`, dừng tail (và dừng hai lần không lỗi), và quan trọng nhất — **dọn khi client disconnect** (`server.client_disconnected`), test trực tiếp phần generic connection-id/disconnect-cleanup plumbing mà round Kafka chưa viết được test cho.
- Headless Neovim thật: build `tradar-server` (`cargo build -p tradar-server --features kafka`, `socket` đã có sẵn trong `default`), chạy thật cùng một Python echo server trần (`socketserver`-style thủ công, 15 dòng), gõ một dòng qua `nvim_feedkeys`, xác nhận cả `›`/`‹` xuất hiện đúng trong buffer transcript, đóng buffer, xác nhận `connections.list` báo `connected: false` ngay sau đó — 8/8 kiểm tra qua.

## Chưa làm

- RabbitMQ/HTTP qua server — không cần push, có thể làm độc lập bất cứ lúc nào.
- Giới hạn số subscription đồng thời (như `MAX_CURSORS`) — vẫn chưa phải vấn đề ở quy mô dùng thật hiện tại, giống ghi chú của Kafka tail.
