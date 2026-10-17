# Test end-to-end thật cho Kafka tail qua server (2026-10-20)

Việc còn mở từ `kafka-tail-server-2026-10-15.md` ("chưa làm: test end-to-end thật, cần Kafka broker sống, không có Docker lúc code") — Socket/RabbitMQ/HTTP đã có test e2e không cần Docker, riêng Kafka thì không thể giả lập broker.

`crates/tradar-server/tests/rpc_kafka.rs` (feature `kafka`): dựng `apache/kafka:4.0.1` (KRaft một node, cùng cấu hình với test Docker của `tradar-connector-kafka`) bằng `testcontainers::GenericImage`, rồi lái qua `Server::handle` với một `new_client` có outbox thật — nên notification `kafka.tail.message` đến đúng như trên socket. Một test tuần tự (broker quảng bá cố định `localhost:9092` nên hai test song song sẽ đụng cổng), kiểm tra:

- `kafka.connect` (thử lại tới khi metadata sẵn sàng), gọi lại báo `already`, `connections.list` báo `connected` + `supported`.
- `kafka.topics` thấy topic vừa tạo và **không** thấy topic nội bộ `__*`.
- `kafka.tail.start` từ `earliest` nhận đúng message đã có sẵn (key/value/partition), rồi nhận message sinh ra *trong lúc đang tail* với offset liền kề.
- `kafka.tail.stop` báo `true` rồi `false`, và sau đó broker nhận thêm message thì **không** còn push gì.
- **Dọn khi client ngắt kết nối**: tail không được dừng tay, `client_disconnected` rồi sinh message mới → không có push nào. Đây là phần mà backlog Kafka tail ghi là chưa kiểm chứng được.
- `kafka.disconnect` quên connection; `kafka.topics` sau đó trả lỗi sạch.

Không test lỗi "unknown topic": `librdkafka` có thể tự tạo topic khi hỏi metadata nên kết quả không ổn định.

Chạy: `make test-kafka-server` (cũng nằm trong `make test-docker`). Cần Docker; trên Docker không có bridge network (một số sandbox) đặt `TRADAR_DOCKER_HOST_NETWORK=1` để container dùng host network thay vì publish cổng. `testcontainers` được thêm làm dev-dependency của `tradar-server` (chỉ test này dùng); test biên dịch ra khỏi build khi không bật feature `kafka`.

Xác nhận: 1 passed (~17 giây trên broker thật, chạy hai lần kể cả sau `cargo fmt`), container được dọn sạch sau test, `clippy -D warnings` sạch ở cấu hình mặc định và `--features kafka`, `make test-unit` xanh.
