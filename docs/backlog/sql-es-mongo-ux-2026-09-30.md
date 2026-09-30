# UX cho SQL/Elasticsearch/MongoDB (2026-09-30)

## Bối cảnh

Sau `kafka-rabbitmq-sidebar-filter-2026-09-30.md`, người dùng chỉ đạo: quan trọng nhất vẫn là SQL, Elasticsearch, MongoDB — không phải Kafka/RabbitMQ. Rà UI/UX riêng cho ba driver này (đọc `query_driver.rs`, `completion.rs`, và từng connector crate), tìm được 5 gap cụ thể. Người dùng chọn làm 4/5 (bỏ qua gap #5 — ranking của completion candidate, confidence thấp nhất trong khảo sát ban đầu).

## Thay đổi

### 1. MongoDB: completion lọc theo collection bên trong call

`completion_context()` (`crates/tradar-query-workbench/src/query_driver.rs`) trước đây chỉ hiểu cú pháp SQL `alias.column` (qua `resolve_alias` quét `FROM`/`JOIN`) — Mongo không có FROM/JOIN nên luôn rơi vào `CompletionContext::None`, kể cả khi gõ field bên trong `find({...})`.

Thêm `mongo_collection_in_scope()`: quét ngược token tìm token dạng `db.<collection>.<method>` ở độ sâu ngoặc **nông hơn** vị trí con trỏ hiện tại — đó là dấu hiệu con trỏ đang ở *bên trong* tham số của call đó, không phải đang gõ dở tên method. `db.orders.find({status: "open", na` → scoped tới field của `orders`; `db.orders.f` (chưa mở ngoặc) → vẫn `None` như cũ, vì đó là đang gõ tên method (`find`/`aggregate`/...), không phải field — scoping nó theo collection sẽ giấu mất các method thật.

6 test mới trong `query_driver.rs` (field trong call, vị trí trống ngay sau `{`, gõ dở method, hai call liên tiếp, và một test đảm bảo `SELECT COUNT(na` — SQL lồng ngoặc — không bị nhầm thành Mongo call).

### 2. Elasticsearch: validate JSON cục bộ trước khi gửi

`ElasticsearchDriver::execute()` (`crates/tradar-connector-elasticsearch/src/lib.rs`) trước đây forward thẳng body gõ vào cho `reqwest`, không kiểm tra gì — JSON sai chỉ lộ ra sau khi Elasticsearch từ chối qua network, với lỗi tự Elasticsearch sinh ra (thường mơ hồ hơn).

Thêm một bước `serde_json::from_str` trước khi gắn `.body()` vào request — sai thì trả lỗi `invalid JSON body: <serde_json error, có line/column>` ngay tại chỗ, không chạm network. 1 test mới không cần Docker (target `127.0.0.1:1` không route được — nếu code lỡ chạm network sẽ treo/lỗi kết nối chứ không phải lỗi JSON đúng như assert).

### 3. MongoDB: navigator liệt kê index

`ColumnInfo` (`crates/tradar-query-workbench/src/query_driver.rs`) có thêm field `indexed: bool` (mặc định `false` qua `ColumnInfo::new`, phải set thủ công ở 13 chỗ dựng struct-literal khác trong test code + Postgres/SQLite/Cassandra/ClickHouse — tất cả set `false`, chỉ Mongo set giá trị thật).

`MongoDriver::list_schema()` gọi thêm `indexed_field_names()` (hàm mới, `collection.list_indexes()` qua driver `mongodb`) song song với sample document đã có sẵn (`join_all`, cùng idiom), đánh dấu field nào có index thật — bỏ qua `_id` vì index mặc định của nó đã được `primary_key` phản ánh rồi, thêm lần nữa chỉ dư thừa. `push_table()` trong `query_screen.rs` đổi format detail từ `format!("{} pk", type)` sang build có điều kiện, thêm hậu tố ` idx` khi `column.indexed` — không đổi output cho các driver khác (vẫn y hệt cũ vì `indexed` luôn `false`).

1 test Docker-based mới (`list_schema_marks_a_field_covered_by_a_real_index`) — tạo index thật qua `mongodb::Collection::create_index` (bypass parser vì `createIndex` không nằm trong shell-subset connector này hỗ trợ), verify `indexed` đúng field, sai field không bị đánh dấu nhầm, `_id` không bị đánh dấu lần hai.

### 4. SQLite: lỗi có vị trí như Postgres

Chuyển `line_and_caret()` (trước đây riêng của `tradar-connector-postgres`) sang `query_driver.rs` thành `pub fn`, vì hai connector crate không được phép phụ thuộc lẫn nhau (nguyên tắc kiến trúc trong `CLAUDE.md`) mà cả hai giờ đều cần dùng chung. Postgres đổi gọi qua `query_driver::line_and_caret`, hành vi không đổi.

SQLite không có khái niệm "vị trí lỗi" trong C API của nó (`sqlx::sqlite::SqliteError` chỉ có `message`/`code`, không có offset — đã xác nhận đọc thẳng source `sqlx-sqlite`). Thay vào đó, `format_sqlite_error`/`near_token_marker` (`crates/tradar-connector-sqlite/src/lib.rs`) parse cú pháp thông báo quen thuộc của chính SQLite — `near "X": syntax error` — lấy token `X`, tìm lần xuất hiện đầu tiên của nó trong câu lệnh gốc, rồi tái dùng `line_and_caret` y hệt Postgres. Đây là suy luận gần đúng (không chính xác tuyệt đối như offset thật SQLite có thể có nội bộ nhưng không lộ ra qua sqlx), không phải bug fix hoàn hảo — thông báo nào không có dạng `near "X":` (constraint violation, "no such table"...) thì rơi thẳng về message gốc như trước, không đổi.

5 test mới: 3 test thuần cho `near_token_marker` (match đúng, message không có token trích dẫn, token không thật sự có trong query), 2 test tích hợp chạy thật (SQLite file tạm, không cần Docker) — một lỗi cú pháp thật có marker, một lỗi "no such table" (không có token trích dẫn) vẫn hiện message gốc.

## Test

- `cargo test -p tradar-connector-sqlite`: 27/27 pass (không cần Docker — SQLite file-based).
- `cargo test -p tradar-query-workbench`: 552/552 pass.
- `cargo test -p tradar-connector-elasticsearch`: 26/33 pass, 7 fail là test Docker-integration sẵn có (không có Docker daemon trong sandbox, không liên quan thay đổi này).
- `cargo test -p tradar-connector-mongo`: 41/65 pass (40 cũ + 1 mới `list_schema_marks_a_field_covered_by_a_real_index`), 24 fail đều là Docker-integration sẵn có.
- `cargo test -p tradar-connector-postgres`: 5/18 pass, 13 fail đều Docker-integration sẵn có — xác nhận việc chuyển `line_and_caret` sang `query_driver.rs` không đổi hành vi (test `a_syntax_error_reports_the_offending_line_with_a_caret` vẫn ở nguyên vị trí cũ, chỉ không tự chạy được vì thiếu Docker, không phải do thay đổi).
- `cargo build --workspace --exclude tradar-connector-kafka`, `cargo clippy --all-targets --workspace --exclude tradar-connector-kafka -- -D warnings`, `make test-unit` (kafka-disable trick, khôi phục qua `cp` không dùng `git checkout`): sạch, 552+159+27+... pass toàn bộ không lỗi.
- `cargo fmt --check`: sạch.

## Chưa làm

- Gap #5 (ranking của completion candidate ưu tiên Table/Column trước Keyword — không hợp lý cho Mongo/ES nơi "keyword" chính là tên method/endpoint) — người dùng không chọn làm trong lượt này, confidence thấp nhất trong 5 phát hiện ban đầu.
- Postgres/SQLite/Cassandra/ClickHouse không có `ColumnInfo::indexed` thật — field mới chỉ Mongo dùng, các driver khác luôn `false` (không phải bug, chỉ là scope hẹp theo đúng gap đã chọn).
- `near_token_marker` không xử lý được trường hợp token trích dẫn xuất hiện nhiều lần trong câu lệnh mà lỗi thật nằm ở lần xuất hiện sau — lấy lần đầu tiên, chấp nhận như một giới hạn đã biết (ghi rõ trong doc comment).
