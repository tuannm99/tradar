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

### 5. MongoDB: completion không còn giấu tên method đằng sau field/collection trùng tiền tố (2026-10-01)

Gap bỏ qua ở lượt đầu, quay lại làm sau khi người dùng tiếp tục chọn "rà UX sâu hơn". `CompletionSource::matches()` (`crates/tradar-query-workbench/src/components/completion.rs`) rank cố định Table(0) > Column(1) > Keyword(2) — hợp lý cho SQL (tên bảng/cột không đoán được, keyword SQL thì ít và quen thuộc), nhưng sai cho Mongo: "keyword" của Mongo chính là tên method (`find`, `aggregate`, ...), nên gõ `db.orders.f` mà có field/collection nào trùng tiền tố `f` thì nó che mất `find`/`findOne` lên trên.

Thêm biến thể `CompletionContext::MongoMethod` mới: `completion_context()` nhận diện cú pháp `db.<collection>.<method-đang-gõ>` (tái dùng `mongo_call_collection` đã có từ gap #1, chỉ cần kiểm tra token khớp dạng đó — không cần biết tên collection thật sự là gì ở bước này) và trả về context này thay vì rơi vào `None` như trước. `matches_in_context()` xử lý context mới bằng cách **lọc hẳn** chỉ còn `CandidateKind::Keyword` — không chỉ rank lại, vì field/collection không bao giờ hợp lệ ở đúng vị trí đó (khác `TableColumns`/`JoinTarget`, vốn chỉ rank lại trên cùng một flat list).

Cập nhật test cũ `typing_the_mongo_method_name_itself_is_not_scoped_to_a_collection` (trước assert `None`, giờ assert `MongoMethod`), thêm 2 test context-detection mới (`db.orders.` trống cũng là method context; `db.ord` — mới 1 dấu chấm, chưa đủ để phân biệt — vẫn `None` như cũ vì collection name tự nó đã rank đúng là Table) và 2 test ranking trong `completion.rs` (field/collection trùng tiền tố với method bị lọc hẳn, không chỉ xếp sau; prefix không khớp gì thì trả rỗng).

## Test

- `cargo test -p tradar-connector-sqlite`: 27/27 pass (không cần Docker — SQLite file-based).
- `cargo test -p tradar-query-workbench`: 552/552 pass.
- `cargo test -p tradar-connector-elasticsearch`: 26/33 pass, 7 fail là test Docker-integration sẵn có (không có Docker daemon trong sandbox, không liên quan thay đổi này).
- `cargo test -p tradar-connector-mongo`: 41/65 pass (40 cũ + 1 mới `list_schema_marks_a_field_covered_by_a_real_index`), 24 fail đều là Docker-integration sẵn có.
- `cargo test -p tradar-connector-postgres`: 5/18 pass, 13 fail đều Docker-integration sẵn có — xác nhận việc chuyển `line_and_caret` sang `query_driver.rs` không đổi hành vi (test `a_syntax_error_reports_the_offending_line_with_a_caret` vẫn ở nguyên vị trí cũ, chỉ không tự chạy được vì thiếu Docker, không phải do thay đổi).
- `cargo build --workspace --exclude tradar-connector-kafka`, `cargo clippy --all-targets --workspace --exclude tradar-connector-kafka -- -D warnings`, `make test-unit` (kafka-disable trick, khôi phục qua `cp` không dùng `git checkout`): sạch, 552+159+27+... pass toàn bộ không lỗi.
- `cargo fmt --check`: sạch.

Cộng test cho gap #5: `cargo test -p tradar-query-workbench` sau khi thêm → 556/556 pass. `cargo build`/`clippy --all-targets -D warnings`/`make test-unit`/`cargo fmt --check` toàn workspace (kafka-disable trick) lại một lần nữa: sạch.

### 6. Elasticsearch: response không phải 2xx giờ là lỗi thật, không phải "kết quả" (2026-10-01)

Rà sâu hơn một lượt nữa (agent audit riêng), phát hiện bug nghiêm trọng hơn cả UX: `execute()` chưa bao giờ kiểm tra `response.status()` — một request sai (index không tồn tại, mapping lỗi, parse exception, sai quyền...) vẫn trả `Ok(QueryResult::Documents(...))` với chính body lỗi của Elasticsearch, hiện trong bảng kết quả y hệt một kết quả thật. Chỉ lỗi tầng network thật (connection refused, timeout) mới từng hiện đúng như lỗi. Ngược hẳn kỳ vọng bình thường: trường hợp hay gặp (server từ chối query) trông như thành công, trường hợp hiếm (mất mạng) mới trông như lỗi.

Tách phần xử lý response (từ sau khi có status + body text) thành hàm thuần `handle_response()` — test được mà không cần cluster thật, cùng tinh thần `parse_query`/`unwrap_search_hits` đã là free function từ trước. `!status.is_success()` giờ trả `Err` kèm status code + body lỗi pretty-print (phần `error.reason` của Elasticsearch là thông tin hữu ích nhất, không nên bỏ qua chỉ lấy status code).

4 test mới, toàn bộ không cần Docker: status OK vẫn wrap bình thường, status lỗi với body JSON (`index_not_found_exception`) → `Err` chứa đủ status/type/reason, status lỗi với body không phải JSON (vd lỗi reverse-proxy) vẫn báo được, response `_cat`-family (plain text, status OK) vẫn hoạt động như cũ.

### 7. Results grid: sort không còn "âm thầm" áp dụng trễ trong JSON view (2026-10-01)

Cùng đợt audit: `ResultsComponent::sort_by_column()` (`crates/tradar-query-workbench/src/components/results.rs`) luôn ghi `self.sort` bất kể đang xem gì, nhưng `compute_visible_items()`'s nhánh JSON (view mặc định cho kết quả `Documents` của Mongo/Elasticsearch) không bao giờ đọc `self.sort` — bấm `s` trông như không làm gì, rồi khi chuyển sang xem dạng bảng (`Command::ToggleResultView`) mới thấy nó đã tự sort theo cột nào đó từ trước mà không hề chủ ý.

Thêm guard đầu hàm: `if self.columns().is_empty() { return; }` — tái dùng đúng điều kiện `selected_cell()`/`columns()` đã dùng để biết "đang ở view không có cột" (JSON view của Documents). Table và Documents-xem-dạng-bảng không đổi hành vi gì (cả hai đều có cột thật).

2 test mới: sort trong JSON view là no-op (`self.sort` vẫn `None` sau khi bấm), sort hoạt động bình thường ngay khi vừa toggle sang table view.

### 8. Điều tra nghi vấn auto-close phá escape `'O''Brien'` — không phải bug thật (2026-10-01)

Agent audit nghi: gõ literal SQL có escape kiểu `'O''Brien'` sẽ bị auto-close/skip-over làm rối, vì dấu `'` thứ hai của cặp escape không nằm cạnh closer đã auto-insert (giống dấu đầu) mà lại tự mở một cặp `''` mới — nghe hợp lý trên lý thuyết. Người dùng chọn "làm luôn, cẩn thận test kỹ".

Viết test thực nghiệm (`type_str` mô phỏng gõ từng ký tự) thay vì chỉ suy luận tay — kết quả: **không có bug**. Dấu nháy đơn tự ghép cặp với chính nó (`auto_close_for('\'') == Some('\'')`), nên cặp `''` "thừa" tự mở ra ở bước đó chỉ là closer bị đẩy dần về sau khi gõ tiếp các ký tự còn lại (`Brien`), rồi chính nó bị tiêu thụ bởi dấu `'` đóng thật của literal qua skip-over y hệt mọi trường hợp khác — chuỗi ký tự cuối cùng luôn khớp chính xác với những gì gõ vào, không thiếu không thừa. Verify bằng 3 test thực tế: `'O''Brien'`, hai cặp escape trong cùng literal (`'O''Brien''s house'`), và escape ngay đầu literal (`'''a'`) — cả ba đều cho kết quả đúng.

**Không sửa code gì** (không có gì để sửa) — chỉ thêm 3 test khoá lại hành vi đã đúng này làm regression test, phòng khi có ai đó thay đổi logic `type_char`/`auto_close_for` sau này vô tình phá nó. Bài học: một nghi vấn nghe hợp lý về cấu trúc code chưa chắc là bug thật nếu không chạy thử — không nên "sửa" một thứ chưa được xác minh là hỏng.

## Test (gap #6, #7, #8)

- `cargo test -p tradar-connector-elasticsearch`: 34 test, 30 pass (26 cũ + 4 mới), 7 fail Docker-integration sẵn có (không đổi so với trước).
- `cargo test -p tradar-query-workbench`: 561/561 pass (558 + 3 test điều tra gap #8).
- `cargo clippy -p tradar-connector-elasticsearch -p tradar-query-workbench --all-targets -- -D warnings`: sạch.

### 9. REGRESSION tự gây ra hôm nay: `_bulk`/`_msearch` của Elasticsearch bị validate JSON chặn nhầm (2026-10-01)

Đợt audit thứ 3 phát hiện: fix #2 (validate JSON cục bộ, thêm sáng cùng ngày) dùng `serde_json::from_str::<Value>(body)` — yêu cầu TOÀN BỘ body là đúng một JSON value. Nhưng `_bulk` (đã có sẵn trong `keywords()` như một endpoint được hỗ trợ) và `_msearch` dùng NDJSON thật — nhiều JSON object nối nhau bằng dòng mới, không dấu phẩy, không phải một object duy nhất. Mọi request `_bulk` thật đều bị validate cục bộ chặn với lỗi "trailing characters" trước khi chạm network — gõ đúng format NDJSON chuẩn (giống hệt Kibana Dev Tools) vẫn bị báo sai.

Sửa: `validate_json_body()` thử parse cả body như MỘT JSON value trước (giữ nguyên lỗi rõ ràng cho trường hợp phổ biến — object đơn gõ sai); nếu fail mới thử parse TỪNG DÒNG riêng (bỏ qua dòng trống) — chỉ báo lỗi khi cả hai cách đều fail. Không hardcode tên endpoint `_bulk`/`_msearch` — tổng quát cho mọi NDJSON body.

6 test mới: single-object hợp lệ, NDJSON 2 dòng hợp lệ, NDJSON có dòng trống xen giữa vẫn hợp lệ, single-object sai vẫn báo lỗi như cũ, NDJSON có 1 dòng sai vẫn báo lỗi, và test tích hợp gửi `_bulk` thật qua `execute()` xác nhận không bị chặn cục bộ (dùng cùng trick host-không-route-được để phân biệt "chặn cục bộ" với "lỗi network").

### 10. MongoDB: câu lệnh gõ trải nhiều dòng không còn bị tách sai (2026-10-01)

Cùng đợt audit: `MongoDriver::split_statements()` tách CÂU theo DÒNG — mỗi dòng không rỗng là một câu riêng, không như `tradar-connector-elasticsearch`'s `split_statements`/`starts_request` (đã merge dòng tiếp nối vào câu phía trên từ trước). Hệ quả: gõ một call Mongo trải nhiều dòng theo đúng phong cách mongosh/Compass hay dùng (ví dụ `db.orders.find({` xuống dòng, `status: "open"`, rồi `})`) bị tách thành 3 câu riêng biệt, mỗi câu tự nó không parse được — `Ctrl+Enter`/`Ctrl+A` báo lỗi cú pháp khó hiểu mà không gợi ý gì về giới hạn "một dòng một câu" này.

Thêm `starts_mongo_statement()` (mirror `starts_request` của Elasticsearch): một dòng bắt đầu câu mới nếu bắt đầu bằng `db.`, `use `, hoặc đúng `show dbs`/`show databases`; dòng khác luôn nối vào câu phía trên. `split_statements()` đổi logic y hệt Elasticsearch — mở rộng `current.end`/`current.text` (re-slice từ text gốc, giữ nguyên `\n` nhúng bên trong) thay vì luôn push câu mới. Statement đã merge xuống dòng vẫn parse đúng vì `serde_json`/`find_matching_close_paren` vốn không giả định text một dòng.

6 test mới: call trải nhiều dòng gộp thành một câu, hai call một-dòng vẫn tách đúng như cũ, call nhiều dòng tách đúng khỏi câu kế tiếp, `use`/`show dbs` vẫn nhận diện đúng là câu riêng, và một test tích hợp (cần Docker) chạy thật một `find()` viết trải 3 dòng, verify kết quả đúng.

## Test (gap #9, #10)

- `cargo test -p tradar-connector-elasticsearch`: 42 test, 36 pass (30 cũ + 6 mới), 6 fail Docker-integration sẵn có (không đổi).
- `cargo test -p tradar-connector-mongo`: 70 test, 44 pass (40 cũ + 4 mới pure), 26 fail Docker-integration sẵn có (25 cũ + 1 test mới cần Docker).
- `cargo clippy -p tradar-connector-elasticsearch -p tradar-connector-mongo --all-targets -- -D warnings`: sạch.
- `cargo build`/`clippy --all-targets -D warnings`/`make test-unit`/`cargo fmt --check` toàn workspace (kafka-disable trick): sạch.

## Chưa làm

- **Gap #5 đã làm (2026-10-01)** — xem mục 5 ở trên. Ranking của Elasticsearch (endpoint/method cũng là "keyword") chưa đụng tới: ES không có cú pháp `db.x.y` để nhận diện như Mongo, và console của nó gõ `METHOD /path` ở đầu dòng chứ không lẫn vào giữa field/index — rủi ro bị field che mất thấp hơn hẳn, không có bằng chứng cụ thể cần sửa.
- **Gap #8 điều tra, không phải bug** — xem mục 8 ở trên.
- **Gap #9, #10 đã làm (2026-10-01)** — xem mục 9, 10 ở trên.
- **Chưa làm, đang chờ quyết định phạm vi**: CRUD snippet `insertOne` của Mongo quote field lồng nhau (vd `address.city`) thành key phẳng có dấu chấm thay vì dựng lại object lồng nhau thật — đúng cho `updateOne`/`deleteOne` (Mongo coi dấu chấm trong filter/`$set` là path expression) nhưng sai cho `insertOne` (tạo field phẳng tên `"address.city"` thay vì `{address: {city: ...}}`). Sửa đúng cần dựng cây lồng nhau từ danh sách field dotted rồi render lại thành object literal nhiều cấp — phức tạp hơn 4 fix vừa rồi, cần `AskUserQuestion` riêng.
- **Chưa làm, mang tính chủ quan hơn**: connection form không có gợi ý format `target` theo từng driver (Postgres connection string vs Mongo URI vs ES base URL) và lỗi connect thất bại hiện message gốc từ thư viện (`sqlx`/`mongodb`) không có thêm ngữ cảnh — là cải thiện UX rộng hơn (thêm placeholder/help text cho N driver), không phải bug fix hẹp, cần quyết định phạm vi riêng.
- Postgres/SQLite/Cassandra/ClickHouse không có `ColumnInfo::indexed` thật — field mới chỉ Mongo dùng, các driver khác luôn `false` (không phải bug, chỉ là scope hẹp theo đúng gap đã chọn).
- `near_token_marker` không xử lý được trường hợp token trích dẫn xuất hiện nhiều lần trong câu lệnh mà lỗi thật nằm ở lần xuất hiện sau — lấy lần đầu tiên, chấp nhận như một giới hạn đã biết (ghi rõ trong doc comment).
