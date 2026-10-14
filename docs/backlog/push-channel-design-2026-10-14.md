# Push-channel: thiết kế (2026-10-14, chưa code)

## Bối cảnh

Sau khi gRPC v1 xong (PR #34), mục mở lớn nhất còn lại là "Kafka/RabbitMQ/HTTP/Socket qua server" — đứng yên từ lúc `tradar-server` ra đời (2026-10-04) vì "cần kênh notification đẩy từ server (tail realtime), không khớp mô hình request/response" hiện có. Đây cũng chính là hạ tầng gRPC's server-streaming (để fast-follow, PR #32) đang chờ. Phiên này chỉ **thiết kế**, không code — theo đúng yêu cầu, để tránh code hai lần nếu thiết kế sai đâu đó lúc bắt tay Kafka/Socket thật.

## Khảo sát: không phải cả 4 connector đều cần push

Đọc lại thiết kế cũ của cả bốn connector phi-query (`docs/architecture.md`) trước khi thiết kế gì:

- **Kafka, mode Topics**: tail real-time thật — `KafkaSession` có task consumer nền đẩy message liên tục, không phải request-response một lần. **Cần push.**
- **Kafka, mode Groups**: tính lag là một round-trip một lần mỗi `Enter`/`r` — không tail. **Không cần push**, khớp thẳng vào method thường.
- **RabbitMQ**: browse queues/exchanges + publish, đã ghi non-goal rõ ràng từ lúc thiết kế "tail real-time cho RabbitMQ qua AMQP". **Không cần push.**
- **HTTP**: Postman-style, mỗi request một response, không có khái niệm stream. **Không cần push.**
- **Socket**: netcat-style, một task nền đọc liên tục từ `TcpStream`, dữ liệu tới bất cứ lúc nào không theo nhịp client hỏi. **Cần push.**

Kết luận: chỉ 2/4 (Kafka tail, Socket) thực sự chặn bởi thiếu push-channel. RabbitMQ và HTTP có thể đưa vào `tradar-server` độc lập, bất cứ lúc nào, bằng method thường giống `execute`/`schema` — không cần chờ thiết kế này. Roadmap trước đây gộp chung cả 4 vì cùng là "connector phi-`QueryDriver` chưa qua server", không phải vì cùng cần cùng một hạ tầng.

## Quyết định kiến trúc

### 1. Không cần transport mới — tái dùng channel per-connection đã có

Đọc `crates/tradar-server/src/transport.rs`: `serve_connection` đã tách request-handling (một `JoinSet`, mỗi request một task) khỏi việc ghi ra socket (`mpsc::unbounded_channel<String>`, gọi là `replies`/`outbox`, một task writer riêng bơm từng dòng ra). Hai task request-handling và writer đã độc lập với nhau — writer chỉ biết "có dòng String nào gửi tới thì ghi ra socket", không quan tâm dòng đó là reply của request nào hay không phải reply gì cả.

Hệ quả: **bất kỳ code nào giữ một bản sao của `replies: mpsc::UnboundedSender<String>` có thể đẩy một dòng JSON vào socket đó bất cứ lúc nào**, không cần đợi một request tới. Không cần socket thứ hai, không cần SSE/WebSocket, không cần đổi framing (vẫn newline-delimited JSON).

Hình dạng dòng đẩy: một **JSON-RPC notification** đúng chuẩn — object có `jsonrpc`/`method`/`params`, **không có** trường `id` (khác response, luôn có `id`, kể cả lỗi). Phía client (`rpc.lua`'s `on_line`) hiện tại parse mọi dòng thành `msg` rồi tra `pending[msg.id]` — khi `msg.id` là `nil` (notification không có `id`), `pending[nil]` trong Lua trả về `nil` an toàn (đọc bảng bằng khoá `nil` không lỗi, chỉ *ghi* mới lỗi) — nghĩa là một notification hôm nay đã *không làm sập plugin*, chỉ đơn giản bị bỏ qua vì chưa có ai đăng ký nhận nó.

### 2. Mỗi subscription một task nền riêng, không share/dedup giữa các client

Hai buffer Neovim (hoặc hai session Neovim khác nhau cùng dùng một `tradar-server`) cùng tail một topic Kafka → mỗi cái có `rdkafka` consumer riêng của chính nó, độc lập hoàn toàn. Đây **không phải** một giới hạn tạm thời cần tối ưu sau — nó đúng ngữ nghĩa sản phẩm: mỗi người xem tail muốn thấy toàn bộ stream từ offset họ chọn (`Enter` = latest, `b` = earliest), không phải một phần theo kiểu chia tải giữa các consumer trong cùng consumer group. TUI's `KafkaSession` đã luôn làm vậy (một session một consumer); giữ y nguyên khi chuyển sang server tránh phải giải bài toán fan-out/multi-subscriber-một-consumer phức tạp hơn hẳn mà không connector nào thực sự cần.

### 3. Bounded + drop-oldest, không phải unbounded

`replies` hiện là `unbounded_channel` — đúng và đủ cho reply của request (số lượng tự nhiên bị chặn bởi số request client gửi), nhưng một task nền đẩy liên tục (Kafka tail, Socket) mà client đọc chậm sẽ làm `outbox` phình vô hạn. Theo đúng cách TUI đã làm (cả Kafka tail lẫn Socket transcript đều cap ở **500 entry gần nhất**, bỏ cái cũ hơn khi đầy, không phải block nguồn dữ liệu): task nền tự giữ giới hạn 500 trước khi đẩy dòng vào `replies`, không trông cậy vào `replies` tự giới hạn.

### 4. Dọn theo connection khi client đóng kết nối

Một subscription (task nền) phải "thuộc về" đúng một connection socket. Khi `serve_connection` kết thúc (client đóng, ví dụ đóng Neovim đột ngột không gọi `*.stop`), mọi task nền riêng của connection đó phải bị abort — không rò rỉ một `rdkafka` consumer chạy mãi cho một client không còn ai lắng nghe. Cần một cơ chế mới (chưa có hôm nay vì không cần: method nào cũng xong gọn trong một request) — gợi ý: một "connection id" sinh ra trong `serve_connection`, truyền xuống `Server::dispatch` cùng `params`, và `Server` giữ `HashMap<ConnectionId, Vec<JoinHandle>>` để abort hàng loạt lúc dọn.

### Hình dạng method minh hoạ (chưa code, chỉ để thấy rõ thiết kế khớp)

```
kafka.tail.start {connection, topic, from: "latest"|"earliest"} -> {subscription}
                                                     (notification) kafka.tail.message {subscription, partition, offset, key, value}
kafka.tail.stop {subscription} -> {stopped: bool}
```

Socket theo đúng hình dạng tương tự (`socket.tail.message` mỗi chunk nhận được, UTF-8 khi hợp lệ).

## Chưa làm

- Code thật: method RPC cụ thể cho Kafka tail và Socket, cơ chế "connection id" + dọn task nền theo connection trong `server.rs`/`transport.rs`, `rpc.lua`'s bảng đăng ký theo `method` (`rpc.on(method, cb)`), và phần plugin Lua tiêu thụ notification (hiện tail trong một buffer, giống TUI's bảng message/transcript cuộn).
- RabbitMQ/HTTP qua server — không liên quan thiết kế này, có thể làm bất kỳ lúc nào bằng method thường.
- gRPC server-streaming (để fast-follow từ PR #32) — sẽ dùng lại đúng cơ chế push-channel này khi tới lượt, không thiết kế riêng.
- Chưa quyết việc có cần giới hạn số subscription đồng thời (giống `MAX_CURSORS = 16` đã có cho cursor kết quả) hay không — có khả năng cần một hằng số tương tự, nhưng chưa phải vấn đề cho tới khi có số liệu dùng thật.
