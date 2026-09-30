# Filter cho sidebar Kafka/RabbitMQ (2026-09-30)

## Bối cảnh

Sau khi merge PR #13 (cache `visible_items()`/`json_lines()` trong results grid), rà thêm vài component khác (`navigator.rs`, `erd.rs`, `outline()`, `completion.rs`, `table_designer.rs`, `migrations.rs`) không tìm thêm được vấn đề hiệu năng đáng sửa. Chuyển hướng sang rà UI/UX: so những gì đã có với hành vi người dùng bình thường sẽ mong đợi.

Ba nghi vấn ban đầu — filter trong navigator, loading indicator khi query chạy lâu, export kết quả ra file — hoá ra **cả ba đều đã có sẵn** (navigator có `/`-filter từ `mouse-ux-polish.md`, results grid có spinner qua `QueryEngine::running`/`elapsed_running()`, và `Ctrl+E` export CSV/JSON đã có từ `features-batch-2026-08-14.md`). Phát hiện phụ: `CLAUDE.md` còn câu "General export... is not built yet" đã lỗi thời — sửa luôn trong cùng lần này.

Rà tiếp thì thấy sidebar Kafka (danh sách topic) và RabbitMQ (danh sách queue/exchange) — hai màn hình bespoke không dùng `QueryScreenComponent` — **không có filter nào cả**, chỉ `j`/`k`/`gg`/`G` duyệt tuần tự toàn danh sách. Với cluster Kafka/RabbitMQ thật có hàng trăm topic/queue, đây là điểm ma sát rõ rệt so với navigator (vốn đã có filter) và đúng theo tinh thần "hành vi người dùng bình thường sẽ muốn gõ để lọc nhanh".

## Thay đổi

Thêm filter bar kiểu `/` cho cả hai sidebar, tái dùng nguyên `Command::Search` (đã bind `/` sẵn ở `Context::Navigator`/`Context::Picker`/`Context::QueryScreen`/`Context::Results`) thay vì tạo `Command` mới — chỉ cần thêm `("/", Command::Search)` vào `Context::Kafka`/`Context::Rabbit` trong `keymap.rs`.

Logic filter đúng nguyên xi `NavigatorComponent`: `filter: String` + `filter_input: Option<TextInput>`, `open_filter()` prefill filter cũ, `filter_key_event()` (`Esc` xoá sạch + reset cursor, `Enter` giữ và đóng bar, phím khác gõ live), case-insensitive substring match trên tên.

- **`KafkaScreen`** (`crates/tradar-connector-kafka/src/screen.rs`): `visible_topics()` lọc `session.topics` theo tên; `selected_topic()`, vim-move (`j`/`k`/`gg`/`G`), và `draw_sidebar` đều chuyển sang dùng danh sách đã lọc thay vì `session.topics` trực tiếp. Title đổi thành `"Topics — filter: <chuỗi>"` khi có filter, giống navigator.
- **`RabbitScreen`** (`crates/tradar-connector-rabbitmq/src/screen.rs`): hai hàm lọc riêng `visible_queues()`/`visible_exchanges()` vì `F2` đổi qua lại 2 mode với hai danh sách khác hẳn nhau — `toggle_mode()` xoá luôn filter đang áp dụng (một filter gõ cho tên queue không có nghĩa gì với tên exchange), giống cách nó đã reset `sidebar_selected` từ trước.
- Cả hai đều render filter bar qua `ui::split_bottom_bar` (helper có sẵn, navigator đã dùng), và thêm hint `"filter"` vào `status_hints()`.

Không đổi `Context::Socket`/`Context::Http` — Socket không có sidebar (đã note lý do trong CLAUDE.md/README), HTTP là form 4 field không phải danh sách nên "lọc" không áp dụng.

## Test

`crates/tradar-connector-kafka/src/screen.rs`: 3 test mới (`filter_narrows_visible_topics_case_insensitively`, `esc_clears_the_filter_and_resets_the_cursor`, `enter_keeps_the_filter_applied_and_closes_the_bar`) — không chạy được trong sandbox này (không có `libcurl-dev` cho `rdkafka-sys`/CMake, vấn đề build-time từ trước, không liên quan thay đổi này), verify bằng compile/test riêng `tradar-connector-rabbitmq` (cùng pattern) cộng đọc code kỹ so với bản đã pass.

`crates/tradar-connector-rabbitmq/src/screen.rs`: 5 test mới (`toggle_mode_also_clears_a_filter_left_over_from_the_other_mode`, `filter_narrows_the_queue_list_case_insensitively`, `filter_narrows_the_exchange_list_independently_of_queues`, `esc_clears_the_filter_and_resets_the_cursor`) — `cargo test -p tradar-connector-rabbitmq`: 15/16 pass (test còn lại là Docker-integration test sẵn có, fail vì không có Docker daemon trong sandbox, không liên quan).

`cargo build --workspace --exclude tradar-connector-kafka`, `cargo clippy --all-targets --workspace --exclude tradar-connector-kafka -- -D warnings`, `make test-unit` (loại kafka bằng "kafka-disable trick" — sao lưu/khôi phục `tradar-app/Cargo.toml` + `main.rs` qua `cp`, không dùng `git checkout`): sạch, 543 test pass.

## Chưa làm

- Không thêm filter cho HTTP (không phải danh sách) hay Socket (không có sidebar).
- Không đổi hành vi filter navigator đang có (chỉ tái dùng logic/pattern).
