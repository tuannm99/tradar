# Kafka: mode Groups — lag theo consumer group (2026-10-01)

## Bối cảnh

Hoãn khỏi v1 lúc connector Kafka được thêm (2026-08-16, `docs/backlog/mockup-ui-2026-08-15.md` mục 5/8) — mockup Screen 7 có phần consumer group nhưng lấy lag đúng nghĩa (current offset của 1 group cụ thể theo từng partition, so với high-water mark) với `rdkafka` cần dựng 1 consumer tạm gán `group.id` được chọn rồi gọi `committed()`, phức tạp hơn đáng kể so với Topics mode. Chọn làm tiếp qua `AskUserQuestion` trong số các hạng mục roadmap còn mở (so với gRPC spike và CLI import/export, cả hai cần chốt phạm vi trước).

## Phát hiện phụ: bug biên dịch có sẵn, chưa ai từng bắt được

Trước khi code được tính năng này, `cargo check -p tradar-connector-kafka` thất bại ngay từ đầu — sandbox này thiếu header `curl/curl.h` (`libcurl-dev`) nên `rdkafka-sys`'s `cmake-build` feature chưa bao giờ build được ở đây (đúng như `CLAUDE.md` đã ghi: "Nine/Ten of the ten/twelve connectors... needs Docker"; riêng kafka còn cần thêm toolchain C). Cài `libcurl4-openssl-dev` qua `apt-get` (sau `apt-get update`, lần đầu bị 404 vì cache cũ) giải quyết việc build — nhưng build xong lộ ra một lỗi borrow-checker **có sẵn từ trước** ở `KafkaScreen::draw_sidebar` (thêm lúc filter sidebar được code 2026-09-30, `docs/backlog/kafka-rabbitmq-sidebar-filter-2026-09-30.md`): `let visible = self.visible_topics(); let len = visible.len(); self.sidebar_selected = ...;` giữ borrow của `visible_topics()` (elide buộc vào toàn bộ `&self`) qua lượt gán `sidebar_selected` — thất bại dù hai field vốn tách biệt. Lỗi này tồn tại trên `master` từ ngày đó, chỉ chưa ai bắt được vì không có sandbox/CI nào từng thực sự build được crate này (thiếu `libcurl-dev` chặn ngay từ bước compile). Sửa bằng cách tách `len` ra tính trước và drop trước khi gán (pattern y hệt `RabbitScreen::draw_sidebar` vốn đã dùng helper `sidebar_len()` riêng từ đầu, tránh đúng vấn đề này) — đồng thời Groups mode cũng cần một `sidebar_len()` dispatch theo mode nên refactor này làm luôn trong cùng lúc.

## Thiết kế

**Lấy lag bằng cách nào** (xác nhận qua đọc trực tiếp source `rdkafka-0.36.2`/`rdkafka-sys` đã cache local, không đoán từ tài liệu): `Consumer::committed_offsets(tpl, timeout)` hỏi broker's group coordinator committed offset của group mà *chính consumer instance đang gọi* được cấu hình `group.id` — đây là request `OffsetFetch`, **không** cần consumer đã `subscribe()`/`assign()`/join group thật (khác `Consumer::poll()`, cái mới cần join). Vì vậy `fetch_group_lag(group)` dựng một `BaseConsumer` tạm chỉ với `bootstrap.servers` + `group.id = group`, không gọi `subscribe`/`assign` gì cả — không bao giờ ảnh hưởng tới rebalance/membership của group thật, đúng kỹ thuật đã phác thảo trong `docs/architecture.md` từ lúc hoãn.

Build một `TopicPartitionList` phủ **mọi partition của mọi topic đã biết** (`add_partition_range(topic, 0, partitions - 1)`, không phải chỉ topic group đang "có vẻ" consume — không có cách rẻ nào để biết trước topic nào một group consume mà không decode byte thô của `member_assignment`, phụ thuộc partition-assignor nào group đó dùng). Gọi `committed_offsets(tpl, timeout)` một lần cho toàn bộ danh sách; partition group chưa từng commit trả về `Offset::Invalid` (hằng số `-1001`), lọc bỏ — phần còn lại chính xác là tập partition group đó thực sự consume, không cần parse protocol nào khác. Với mỗi partition còn lại, `fetch_watermarks(topic, partition, timeout)` lấy `(low, high)`; lag = `high - committed`.

Liệt kê group (sidebar): `Consumer::fetch_group_list(None, timeout)` trả `GroupList` — lấy tên/state/số member mỗi group (`GroupInfo::name()/state()/members().len()`). Không decode `member_assignment`/`member_metadata` (bytes thô, định dạng phụ thuộc protocol/assignor đàm phán) vì `fetch_group_lag` lấy được câu trả lời chính xác hơn trực tiếp từ broker, không cần suy luận từ đó.

**UI**: `KafkaScreen` thêm `enum KafkaMode { Topics, Groups }`, toggle bằng `F2` (`Command::KafkaToggleMode`, context `Kafka`) — đúng mẫu `RabbitScreen`'s Queues⇄Exchanges đã có (`ToggleRabbitMode`/`RabbitRefresh`/`RabbitOpen` đổi tên chung chung, dispatch theo mode bên trong). `Command::KafkaTailLatest` (bind `enter`) đổi tên thành `Command::KafkaOpen` — vẫn tail-from-latest ở Topics mode, nhưng ở Groups mode gọi `fetch_group_lag` cho group đang chọn; `KafkaTailEarliest`/`KafkaPauseFollow`/`KafkaPublish` chỉ còn tác dụng ở Topics mode (match guard `if self.mode == KafkaMode::Topics`, không làm gì ở Groups — không có "earliest"/"pause"/"publish" tương đương cho lag). `KafkaRefresh` (`r`) dispatch: Topics → `list_topics()`; Groups → `list_groups()` + `fetch_group_lag()` lại cho group đang mở nếu có, đúng mẫu `RabbitScreen::refresh()`. `KafkaSession::new()` gọi cả `list_topics()` lẫn `list_groups()` ngay từ đầu (giống `RabbitSession::new()` fetch cả queues lẫn exchanges upfront) — không cần fetch-on-toggle.

Panel chính ở Groups mode: bảng `topic | partition | committed | high-watermark | lag` cho group đang chọn (`session.group_lag`, populate bất đồng bộ qua `KafkaEvent::GroupLag`); chưa chọn group nào thì hiện placeholder giống Topics mode's "select a topic...".

## Test

- `cargo check`/`clippy -D warnings -p tradar-connector-kafka --all-targets`: sạch (cần `libcurl4-openssl-dev` trong sandbox — xem mục "Phát hiện phụ" ở trên; không có sẵn trên máy/CI thiếu nó, đúng ghi chú `CLAUDE.md` về `rdkafka`'s build requirement).
- `cargo test -p tradar-connector-kafka --lib`: 15/16 pass không cần Docker (toggle_mode, visible_groups filter, open_selected dispatch theo mode, show_lag_selected, group_lag_row's `lag()`, cộng mọi test cũ) — 1 test còn lại (`tail_publish_list_topics_and_group_lag_round_trip_through_a_real_broker`, mở rộng từ test cũ cùng tên để cover thêm Groups mode trong cùng container, giữ đúng constraint "một container, một test" vì port 9092 cố định) cần Docker, **chưa chạy được trong sandbox này** (không có daemon) — viết đầy đủ, compile sạch, cần verify trên máy/CI có Docker trước khi coi là đã xác nhận end-to-end.
- `cargo test -p tradar-core --lib`: 124 pass — xác nhận đổi `Command::ALL` (134→135 phần tử, thêm `KafkaToggleMode`, đổi tên `KafkaTailLatest`→`KafkaOpen`) không vỡ gì (`every_command_has_a_default_binding_somewhere` và các test khác).
- `cargo clippy --all-targets --workspace -- -D warnings` (toàn bộ workspace, **bao gồm** kafka lần đầu tiên verify được trong phiên này): sạch.
- `cargo build --workspace` (mặc định, cả 12 connector bao gồm kafka): thành công.
- `make test-unit`: chạy sạch toàn bộ (không đổi gì ở Makefile — kafka vẫn exclude khỏi `test-unit` vì máy/CI thường không có `cmake`/`gcc`/`libcurl-dev`, việc cài được ở đây chỉ là chuyện riêng của sandbox này trong phiên này, không phải thay đổi yêu cầu build của dự án).
- `cargo fmt --all -- --check`: sạch.

## Chưa làm

- Decode `member_assignment`/`member_metadata` để hiện trực tiếp "group X đang consume topic Y" trong sidebar mà không cần mở lag trước — không cần thiết vì `fetch_group_lag` đã trả lời câu đó chính xác hơn (qua broker, không qua suy luận protocol).
- Không cảnh báo riêng khi một partition có lag âm (consumer "vượt" high-water mark — hiếm, thường do watermark đọc ngay sau khi message mới ghi vào) — hiện số âm ra nguyên văn, không che giấu.
- Chưa verify thật trên Docker (xem mục Test) — cần làm trên máy/CI có daemon trước khi coi tính năng đã xác nhận đầu-cuối với broker thật.
