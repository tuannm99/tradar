# UX gaps còn lại ở các connector khác (2026-10-02)

## Bối cảnh

Sau `sql-es-mongo-ux-2026-09-30.md` (SQL/Elasticsearch/MongoDB), người dùng chỉ đạo tiếp "tối ưu UI/UX" — lượt này rà các connector chưa được audit riêng: Cassandra, ClickHouse, Redis, HTTP, Kafka, RabbitMQ, Socket. Agent audit tìm được 6 gap cụ thể, có evidence file/line rõ ràng. Người dùng chọn qua `AskUserQuestion`: "Làm tất cả, theo thứ tự rẻ→đắt".

## Thay đổi

### 1. SQL editor: ClickHouse/Cassandra có syntax highlighting

`QueryScreenComponent::new()` (`crates/tradar-query-workbench/src/components/query_screen.rs`) chỉ bật `Dialect::Sql` (tree-sitter highlight) cho `postgres`/`mysql`/`sqlite` — ClickHouse (SQL thật, dùng chung `SQL_KEYWORDS`/`split_sql_statements`) và Cassandra (CQL, cũng khá gần SQL) gõ trong editor không màu, dù cả hai đều là SQL-like đủ gần với grammar `tree-sitter-sequel` để tận dụng được.

Thêm `"clickhouse"` và `"cassandra"` vào match arm bật `Dialect::Sql`.

### 2. Redis: browse-mode sidebar không filter được

`BrowseSidebarComponent` (`crates/tradar-query-workbench/src/components/browse_sidebar.rs`) là danh sách key Redis — không có filter, trong khi navigator/Kafka/RabbitMQ sidebar đều đã có `/`-filter từ các lượt trước. Redis có thể có hàng nghìn key, không filter được là gap rõ nhất.

Thêm `filter`/`filter_input` theo đúng pattern filter-bar đã chuẩn hoá (navigator/Kafka/RabbitMQ): `visible_entries()` lọc case-insensitive theo tên, `selected_entry()`/`apply_move()`/`click()` đều đổi sang dùng `visible_entries()` thay vì `self.entries` trực tiếp, `open_filter()`/`is_filtering()`/`filter_key_event()` (Esc xoá+đóng, Enter giữ+đóng, phím khác gõ live). Phát hiện phụ trong lúc làm: `Command::Search` (`/`) chưa từng được bind trong `Context::Browse` ở `keymap.rs` — không phải chỉ thiếu UI, mà bàn phím `/` trong Browse mode trước đây literally không resolve ra lệnh gì cả, nên fix thật là thêm cả binding keymap lẫn logic filter. `dispatch_command`'s `Command::Search` arm rẽ nhánh theo `self.focus == Focus::Browse` để mở filter của sidebar thay vì filter kết quả (`self.search`) khi đang focus panel đó.

4 test mới trong `browse_sidebar.rs`, 1 test mới trong `query_screen.rs` xác nhận `/` khi focus Browse mở đúng filter của sidebar, không phải search kết quả.

### 3. HTTP: thư viện request đã lưu không filter được

`RequestPicker` (`crates/tradar-connector-http/src/screen.rs`) — picker cho request đã lưu (`F7`) — không filter, cùng vấn đề với Redis ở trên khi thư viện có nhiều request.

Cùng pattern, nhưng cần thêm bước map ngược index vì danh sách đã lưu không xoá được item khi đang filter nếu chỉ lọc hiển thị: `visible_indices() -> Vec<usize>` (index thật trong `entries`) + `selected_entry_index()` (map vị trí đã filter → index thật) — load/delete đều qua hàm này để không bao giờ load/xoá nhầm entry khi đang filter. `Context::HttpRequests` thêm binding `/` → `Command::Search`.

5 test mới (filter narrows, Esc/Enter, load khi đang filter load đúng entry không phải theo raw index).

### 4. ClickHouse: lỗi cú pháp có marker vị trí

ClickHouse không có row-edit (biết từ trước) nhưng cũng chưa có marker `LINE N: ... ^` như Postgres/MySQL/SQLite/SQLite-pattern đã có — lỗi cú pháp chỉ hiện message thô từ HTTP interface.

ClickHouse's HTTP error body thường có dạng `... failed at position N (<token>) ...` nhưng không có crate Rust để verify offset base (0 hay 1, byte hay char) chống lại server thật (không có Docker, playground public bị proxy chặn 403). Thiết kế cố ý an toàn: **không tin số `N`** — chỉ dùng token trích dẫn trong message, tìm lần xuất hiện đầu của token đó trong SQL gốc (`sql.find(token)`), rồi tái dùng `query_driver::line_and_caret` y như SQLite/MySQL. Nếu message không đúng dạng giả định hoặc token không thật sự có trong câu lệnh, trả `None` — thà không có marker còn hơn marker sai vị trí.

4 test mới (`clickhouse_error_marker`: trỏ đúng, message không có vị trí, token không có thật trong SQL; `format_clickhouse_error` gắn marker khi tìm được).

### 5. Cassandra: lỗi cú pháp CQL có marker vị trí

Tương tự #4. `scylla`'s `ExecutionError` được đọc trực tiếp từ source crate cached local (`~/.cargo/registry/.../scylla-1.8.0`, `scylla-cql-core-1.8.0`) để xác nhận CHẮC CHẮN format cuối: `ExecutionError::LastAttemptError` (`#[error(transparent)]`) → `RequestAttemptError::DbError(DbError, String)` (`#[error("Database returned an error: {0}, Error message: {1}")]`) — `reason: String` ở vị trí `{1}` mang nguyên văn message ANTLR parser gốc của Cassandra server, dạng `mismatched input 'FRO' expecting K_FROM` (token trong dấu nháy đơn).

`cassandra_error_marker(message, query)`: trích token trong dấu nháy đơn đầu tiên, tìm trong query gốc, tái dùng `line_and_caret`. Cùng nguyên tắc an toàn như ClickHouse — không có token quote được thì `None`.

3 test mới (trỏ đúng vị trí, message không có token quote, token không có thật trong query).

### 6. Kafka/RabbitMQ/Socket: hỗ trợ chuột

Ba connector tự vẽ `Screen` riêng (không qua `QueryScreenComponent`) chưa implement `handle_mouse_event` — toàn bộ chỉ dùng được bàn phím, khác với phần còn lại của app (connection picker, navigator... đều click/double-click/scroll được từ `mouse-ux-polish.md`).

Kafka (`crates/tradar-connector-kafka/src/screen.rs`) và RabbitMQ (`crates/tradar-connector-rabbitmq/src/screen.rs`, cùng cấu trúc sidebar-rồi-detail): thêm field `sidebar_area: Rect` (ghi lại trong `draw_sidebar`), `list_state: ListState` persisted (không dựng mới mỗi `draw()` — cần giữ `.offset()` đúng để hit-test chính xác vị trí đã cuộn tới), `double_click: DoubleClickTracker`. `handle_mouse_event` mới: click trong `sidebar_area` chọn item theo `ui::index_at`, double-click mở luôn (Kafka: tail topic hoặc xem lag group theo mode hiện tại; RabbitMQ: peek queue hoặc xem binding exchange), click ngoài sidebar không làm gì, scroll lên/xuống di chuyển lựa chọn qua `vim_list::apply`.

Socket (`crates/tradar-connector-socket/src/screen.rs`) không có sidebar (chỉ transcript + input line luôn gõ được) nên chỉ cần scroll — tái dùng `scroll()` đã có sẵn cho `ctrl-d`/`ctrl-u`/mũi tên, `handle_mouse_event` map `ScrollUp`/`ScrollDown` vào đó, cùng hướng với phím mũi tên.

Kafka: 4 test mới (click chọn, double-click mở, click ngoài sidebar no-op, scroll di chuyển). RabbitMQ: 4 test mới (cùng bộ). Socket: 1 test mới (scroll wheel giống `j`/`k`).

## Test

- `cargo test -p tradar-query-workbench --lib`: 566/566 pass (561 trước đó + 5 test mới cho gap #2/#1 verify).
- `cargo test -p tradar-core --lib`: 124/124 pass (binding `/` mới cho `Context::Browse`/`Context::HttpRequests`).
- `cargo test -p tradar-connector-http --lib`: 32/33 pass, 1 fail Docker-integration sẵn có (không đổi).
- `cargo test -p tradar-connector-clickhouse --lib`: 10/17 pass, 7 fail Docker-integration sẵn có (không đổi).
- `cargo test -p tradar-connector-cassandra --lib`: 10/12 pass, 2 fail Docker-integration sẵn có (không đổi).
- `cargo test -p tradar-connector-kafka --lib`: 16/16 pass (cần `libcurl4-openssl-dev` cài sẵn trong sandbox từ lượt Kafka Groups mode trước — xem `kafka-groups-mode-2026-10-01.md`).
- `cargo test -p tradar-connector-rabbitmq --lib`: 19/20 pass, 1 fail Docker-integration sẵn có (không đổi).
- `cargo test -p tradar-connector-socket --lib`: 18/18 pass (connector này không cần Docker cho test nào).
- `cargo clippy --all-targets --workspace -- -D warnings`: sạch.
- `cargo build --workspace`: sạch.
- `cargo fmt --all -- --check`: sạch (một vài lần auto-fix dòng dài trong lúc làm, đã chạy lại `cargo fmt --all` trước khi check).

## Chưa làm

- ClickHouse/Cassandra's error marker không verify được chống server thật (không Docker, không network ra ngoài tới service public) — thiết kế an toàn (chỉ tin token quote, không tin số vị trí) giảm rủi ro nhưng không loại trừ hoàn toàn trường hợp message server thật khác hẳn giả định; nếu sai hình dạng, marker chỉ im lặng trả `None`, không bao giờ sai vị trí.
- HTTP/Kafka/RabbitMQ/Socket chưa có test tích hợp thật với Docker trong lượt này (không có daemon) — chỉ verify qua unit test thuần cho các hàm/handler mới, theo đúng pattern "Docker test fail vì thiếu daemon là kỳ vọng, không phải regression" đã thống nhất từ trước trong project này.
- Kafka/RabbitMQ's mouse support chỉ phủ sidebar (chọn/mở) — panel chi tiết (tail/lag/peek/binding) vẫn thuần bàn phím, chưa click được vào message/partition cụ thể trong đó.
