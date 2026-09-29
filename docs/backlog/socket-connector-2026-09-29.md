# Connector Socket — xong (2026-09-29)

Mục thứ hai trong nhóm "Connector mới" (sau ClickHouse cùng ngày). Người dùng chọn Socket cụ thể trong số Socket/MySQL-MariaDB/gRPC/CLI import-export qua `AskUserQuestion`, khớp đúng thứ tự kỹ thuật roadmap đã gợi ý từ trước ("Socket trước — đơn giản kỹ thuật nhất... → gRPC sau"). Thiết kế đã có sẵn, đầy đủ chi tiết, trong `docs/architecture.md`'s "Thiết kế UI: HTTP, gRPC, Socket" (2026-08-16) — không cần vòng `AskUserQuestion` chốt phạm vi mới, chỉ đọc lại thiết kế cũ rồi code. Vài chỗ thiết kế gốc không nói rõ (hoặc giả định sai hình dạng UI) buộc phải tự quyết khi code thật — ghi lại đầy đủ bên dưới, đúng khuôn "Sai khác khi triển khai thật" HTTP đã đặt ra.

## Đúng theo thiết kế gốc

- `target` = `host:port` trần (giống Cassandra/Kafka).
- `SocketSession` sở hữu `tokio::net::TcpStream` (tách `OwnedReadHalf`/`OwnedWriteHalf` qua `into_split()`), một task nền đọc liên tục đẩy chunk nhận được qua channel nội bộ; `tick()` rút có giới hạn (`MAX_DRAIN_PER_TICK = 64`, y hệt hằng số Kafka dùng) — đúng pattern firehose-safe đã đặc tả.
- Buffer cap 500 entry gần nhất (`MAX_BUFFERED_ENTRIES`, cùng ý tưởng `MAX_BUFFERED_MESSAGES` của Kafka) để không leak trên kết nối sống lâu.
- Không sidebar — một panel cuộn hiện transcript, một dòng input ở đáy, `Enter` gửi.
- Toggle tự thêm `\n` cuối dòng khi gửi, mặc định bật.
- Không UDP, không TLS, không parse/frame theo giao thức cụ thể nào (non-goal v1 giữ nguyên).
- Không probe liveness riêng lúc connect — bản thân TCP handshake (`TcpStream::connect` qua `with_connect_timeout`) đã là bằng chứng "sống", không cần round-trip thêm như Kafka/Postgres phải làm (chúng không có gì để "handshake" ở tầng ứng dụng ngoài chính kết nối).

## Sai khác khi triển khai thật

1. **`r` reconnect → `ctrl-r`, `j`/`k`/`gg`/`G` cuộn → mũi tên/`ctrl-d`/`ctrl-u`.** Thiết kế gốc viết "`r` ngắt và kết nối lại" mà không nói rõ input line có rời focus được không. Khi code thật, quyết định input line **luôn ở chế độ gõ** (không có sidebar/field khác để `Tab` sang, khác hẳn HTTP có 4 field luân phiên) — nghĩa là **mọi phím chữ cái đơn đều phải là text**, không được là lệnh, đúng lý do `Context::Http` từ đầu "chỉ giữ binding không-in-được". Hệ quả: `r` (reconnect) và `j`/`k`/`gg`/`G` (cuộn kiểu vim, dùng ở mọi nơi khác trong app qua `Context::List`) đều không dùng được nguyên bản ở đây — đổi thành `ctrl-r` (reconnect) và mũi tên lên/xuống + `ctrl-d`/`ctrl-u` (cuộn nửa trang) cho `Context::Socket` riêng, **không** kết hợp `Context::List` (khác Kafka/HTTP, xem doc comment `Context::Socket` trong `keymap.rs`). Không có `gg`/`G` (nhảy đầu/cuối) ở v1 — `Home`/`End` bị `TextInput` chiếm cho di chuyển con trỏ trong dòng đang gõ nên không dùng lại được, và không muốn thêm tổ hợp phím mới không có tiền lệ chỉ cho việc này.
2. **Hex dump 1 dòng, không phải nhiều dòng kiểu `xxd` thật.** Thiết kế gốc: "decode UTF-8 khi hợp lệ) còn không thì hiện dạng hex dump (kiểu `xxd`)". `xxd` thật in nhiều dòng (16 byte/dòng, offset + hex + ASCII). Làm đúng vậy sẽ phá vỡ bất biến "1 entry = 1 dòng hiển thị" mà cơ chế cuộn (`scroll_offset` tính theo dòng, giống `HttpScreen::response_scroll`) đang dựa vào — cần một bước "trải phẳng thành nhiều dòng" giống `json_lines` đã làm cho JSON view (2026-09-24), nhưng ở đây dữ liệu tới liên tục (không tĩnh như một kết quả query), làm đúng sẽ phức tạp hơn đáng kể. Chọn bản đơn giản: mỗi entry vẫn đúng 1 dòng, hex dump chỉ hiện tối đa 32 byte đầu (`HEX_PREVIEW_BYTES`) dạng `aa bb cc ...`, phần còn lại chỉ ghi `… (N bytes)`. Đủ để xác nhận "có dữ liệu nhị phân tới", không đủ để đọc từng byte trong app — nếu cần xem đầy đủ, chưa có cách nào khác ngoài log ra ngoài.
3. **Có log cả tin đã gửi, không chỉ tin nhận được.** Thiết kế gốc chỉ nói "một panel cuộn hiện dữ liệu **nhận được**". Thêm log tin đã gửi (đánh dấu `›` khác `‹` của tin nhận) vì không có gì khác xác nhận "đã gửi cái gì" — gõ xong `Enter`, dòng input xoá trắng, nếu không echo lại thì không có cách nào biết vừa gửi đúng nội dung mong muốn (kể cả có tự thêm `\n` như đã bật hay không). Rủi ro thấp, dễ bỏ nếu thừa.
4. **Timestamp là elapsed-since-connect, không phải giờ tường (wall-clock).** Thiết kế gốc chỉ nói "mỗi entry có timestamp", không chỉ rõ dạng nào. Chọn elapsed (`[+12.34s]`, tính từ lúc session connect/reconnect gần nhất) để không phải thêm dependency `chrono` chỉ cho việc format giờ-phút-giây — `std::time::Instant`/`Duration` có sẵn đã đủ, và với một phiên xem log ngắn thì "đã trôi bao lâu" hữu ích không kém giờ tường.

## Kiến trúc

`crates/tradar-connector-socket` — 2 file, đúng khuôn Kafka/RabbitMQ: `lib.rs` (`SocketSession` implement `Session`, `SocketConnector` implement `Connector`) + `screen.rs` (`SocketScreen` implement `Component`, không tái dùng `QueryScreenComponent`/`ResultsComponent`, không phụ thuộc `tradar-query-workbench` — đúng quy tắc cách ly connector).

**Gửi** (`SocketSession::send`): ghi log ngay (optimistic, coi như đã gửi) rồi `tokio::spawn` một task cầm `Arc<Mutex<Option<OwnedWriteHalf>>>` để `write_all` thật — không await trực tiếp trong hàm gọi từ phím `Enter` (đúng luật "Screen không bao giờ làm IO" chung của app). Dùng `Arc<Mutex<...>>` (không phải field `OwnedWriteHalf` trần) vì `reconnect()` cần thay thế write-half đang dùng trong khi có thể vẫn còn task gửi cũ chưa xong, và hai lần `Enter` liên tiếp không được phép ghi đè byte lẫn nhau.

**Reconnect** (`SocketSession::reconnect`): abort task đọc cũ, `connected = false` ngay, dial lại từ đầu trong một task mới (không auto-retry — bấm rồi mới thử, giống mọi kết nối khác trong app không tự retry). Task đọc mới dùng đúng vòng lặp `spawn_reader` đã factor ra dùng chung với lần connect đầu tiên.

**Test — không cần Docker** (khác 9/10 connector khác trong workspace): dựng thẳng `tokio::net::TcpListener` cục bộ ngay trong test (`echo_server()` helper, echo lại nguyên văn những gì nhận được, chấp nhận nhiều kết nối tuần tự để `reconnect()` có cái để dial vào). 17 test: 9 ở `lib.rs` (connect thành công/thất bại, gửi-nhận-echo cả 2 hướng đều log đúng, toggle `\n`, phát hiện peer đóng kết nối, reconnect phục hồi và gửi lại được, buffer cap đúng), 8 ở `screen.rs` (chữ cái gõ vào input chứ không bị nuốt làm lệnh — kể cả `r`, `Enter` gửi và xoá input, `F2` toggle, `ctrl-r` reconnect mà không gõ nhầm vào input, `Esc` back, mũi tên cuộn mà không đụng input, hex preview cắt đúng + báo số byte, hiển thị nhị phân không mangle thành text). `Makefile` không cần đổi exclude list — `tradar-connector-socket` đã tự nhiên nằm trong `test-unit` vì không dùng `testcontainers`; thêm target `test-socket` tiện gọi riêng, theo khuôn `test-sqlite`.

## Chưa làm (để lại, không tự chốt trước)

- `gg`/`G` (nhảy đầu/cuối transcript) — xem "Sai khác #1".
- Hex dump nhiều dòng kiểu `xxd` thật — xem "Sai khác #2".
- Pause/freeze view khi cuộn ngược xem log cũ trong lúc dữ liệu mới vẫn tới (Kafka có `paused_at_len` cho đúng vấn đề này) — Socket hiện chỉ có `scroll_offset` tính từ đáy, dữ liệu mới vẫn dồn view xuống nếu đang xem gần đáy; chấp nhận được cho v1 vì thiết kế gốc không yêu cầu, thêm sau nếu thấy khó dùng thật.
- UDP, TLS (`rustls`/`native-tls`) — non-goal v1 đã ghi từ thiết kế gốc, giữ nguyên.
