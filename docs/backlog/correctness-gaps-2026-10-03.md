# 6 gap correctness/UX rải khắp app (2026-10-03)

## Bối cảnh

Sau `app-wide-ux-gaps-2026-10-02.md`, người dùng gõ "tiếp tục" không chỉ định vùng audit tiếp. Agent tự tìm một gap trước khi audit mở rộng: `tradar-connector-redis`'s `list_schema` SCAN toàn bộ keyspace vào RAM trước khi hiện browse sidebar — đúng y rủi ro `MongoDB find/aggregate` đã sửa ở lượt trước (`app-wide-ux-gaps-2026-10-02.md`), chỉ khác driver. Dùng agent Explore audit mở rộng (không giới hạn theo connector) tìm thêm 5 gap: connection form, table designer, Cassandra, migrations, export JSON. Người dùng chọn qua `AskUserQuestion`: "Làm tất cả 6, rẻ→đắt".

## Thay đổi

### 1. Connection form: HTTP bắt buộc gõ Target dù hint nói "optional"

`ConnectionFormComponent::confirm()` (`crates/tradar-app/src/components/connection_form.rs`) chạy check "target must not be empty" cho **mọi** driver — nhưng `target_hint("http")` ngay phía trên đã ghi rõ `"optional -- e.g. https://api.example.com"`, và `tradar-connector-http`'s `connect()` thực sự chấp nhận target rỗng (gõ URL đầy đủ mỗi request). Kết quả: chọn driver HTTP với ý định dùng target rỗng (trường hợp được chính app quảng cáo là hỗ trợ) thì không lưu được connection, phải gõ một target giả chỉ để qua validation.

Thêm điều kiện `&& self.driver_id() != "http"` vào check — chỉ driver này được miễn.

1 test mới: `an_empty_target_is_accepted_for_http` xác nhận `confirm()` trả `Saved` với target rỗng khi driver là `"http"`.

### 2. Table designer: tick Primary Key trên nhiều cột cùng lúc, Postgres sẽ từ chối DDL

`Stage::CreateTableColumns`'s `TableDesignerCommitColumn`/`Confirm` handler (`crates/tradar-query-workbench/src/components/table_designer.rs`) không có cross-column check — tick PK trên cột `id` rồi commit, sau đó tick PK tiếp trên cột `email` rồi commit/confirm, statement hiện ra ở bước `y`-confirm trông bình thường nhưng Postgres chạy sẽ báo lỗi "multiple primary keys ... are not allowed". Đây đúng vai trò validate-trước-khi-chạy mà `confirm()` đã làm cho tên/target rỗng ở gap #1, chỉ thiếu cho trường hợp PK trùng.

Thêm `columns_have_a_primary_key(&[NewColumn]) -> bool`, gọi tại cả hai nhánh commit cột (phím commit tường minh `Ctrl+A` và auto-commit cột cuối lúc `Confirm`) — tick PK lần thứ hai bị chặn ngay với lỗi "a table can only have one primary key column", cột không được thêm vào danh sách.

2 test mới: `committing_a_second_primary_key_column_is_rejected`, `confirming_with_a_second_primary_key_still_in_the_form_is_rejected` (cả hai đường commit đều phải chặn, không chỉ một).

### 3. Redis `list_schema`: SCAN toàn bộ keyspace vào RAM trước khi hiện sidebar

`RedisDriver::list_schema` (`crates/tradar-connector-redis/src/lib.rs`) SCAN phân trang 100 key/lần nhưng lặp **tới khi hết** (`cursor == 0`) rồi mới trả về — một Redis thật với hàng triệu key load hết vào memory trước khi browse sidebar/navigator kịp hiện, khác mọi connector khác (chỉ đọc metadata nhỏ, không phải data ở quy mô này). Code đã có comment tự nhận biết rủi ro này từ trước ("Fine for a keyspace of ordinary size...").

Dừng loop khi `keys.len() >= MAX_ROWS` (tái dùng `query_driver::MAX_ROWS`, cùng hằng số mọi driver SQL/Mongo đã dùng), `keys.truncate(MAX_ROWS)` phòng batch cuối vượt mốc. **Cố ý không** thêm cờ `truncated` lan tới sidebar — khác với gap MongoDB ở lượt trước, ở đây sẽ phải đổi signature `list_schema`/`SchemaInfo` cho toàn bộ 12 connector chỉ để phục vụ đúng 1 driver, chi phí vượt xa phạm vi gap này.

1 test mới (cần Docker, không chạy được trong sandbox): `list_schema_stops_scanning_at_max_rows` — seed `MAX_ROWS + 1` key qua một `MSET` ghép sẵn (không phải `MAX_ROWS + 1` round trip qua `execute()`), xác nhận `list_schema()` trả đúng `MAX_ROWS` key.

### 4. Cassandra `execute()`: `query_unpaged` tải hết kết quả về trước rồi mới cắt

`CassandraDriver::execute` (`crates/tradar-connector-cassandra/src/lib.rs`) dùng `session.query_unpaged(query, &[])` — tải **toàn bộ** response chưa phân trang về rồi mới lặp cắt ở `MAX_ROWS`, khác Postgres/MySQL/SQLite (đều `fetch()`/stream và dừng đọc khỏi wire ngay khi đủ cap). `SELECT * FROM events` quên `LIMIT` trên bảng chục triệu dòng sẽ kéo hết bảng về client trước khi Tradar có cơ hội cắt — cùng lỗi "pulls unbounded result into memory" như gap Redis/Mongo, chỉ khác bề mặt kích hoạt.

Đổi sang `query_iter` (API phân trang thật của `scylla`, trả về `QueryPager`) cho câu lệnh có trả dòng, dùng `rows_stream::<Row>()` (`futures-util::StreamExt`, dependency mới) để stream+cắt đúng kiểu Postgres/MySQL/SQLite đang làm. Khó nhất của gap này: `query_iter` không còn tự phân biệt được SELECT (trả dòng) với INSERT/UPDATE/DDL (không trả dòng) theo cách `query_unpaged`'s `into_rows_result()`/`IntoRowsResultError::ResultNotRows` cũ làm được — verify trực tiếp trong source code đã cache cục bộ (`~/.cargo/registry/.../scylla-1.8.0/src/client/pager.rs`): một statement không trả dòng được pager mock thành **empty row stream với 0 column spec** (`DeserializedMetadataAndRawRows::mock_empty()`, comment gốc trong driver nhắc rõ "as suggested in #631") — nghĩa là không dùng được heuristic "0 cột = không phải SELECT" một cách an toàn vì nó phụ thuộc hành vi nội bộ không thuộc public API contract. Thay vào đó, tái dùng đúng cơ chế Postgres/MySQL/SQLite đã dùng từ trước: `query_driver::returns_rows(query)` (heuristic cú pháp client-side, SELECT/WITH/VALUES/... ở đầu câu) quyết định rẽ nhánh `query_unpaged` (giữ nguyên, không cap vì không cần) hay `query_iter` (stream+cap) — nhất quán với 3 driver SQL khác, không phải cơ chế riêng cho Cassandra. `format_cassandra_error` đổi tham số từ `ExecutionError` cụ thể sang generic `E: std::error::Error + Send + Sync + 'static` vì `query_iter`/`rows_stream` trả kiểu lỗi khác (`PagerExecutionError`/`TypeCheckError`/`NextRowError`), tái dùng đúng logic tìm token trong message không đổi.

Test mới cho đúng cú pháp cắt ở `MAX_ROWS` **không làm được** trong lượt này — xem "Chưa làm". Test tích hợp có sẵn (`schema_and_execute_round_trip_through_a_real_cluster`, cần Docker) đã phủ cả hai nhánh (CREATE KEYSPACE/TABLE + INSERT qua `Affected`, SELECT qua `Table`) nên xác nhận refactor không đổi hành vi observable cho trường hợp thường, chỉ thiếu phần "thật sự cap ở MAX_ROWS".

### 5. Migrations: thư mục khoá theo tên connection (chuỗi có thể đổi), không enforce unique

`default_migrations_dir(connection_name)` (`crates/tradar-core/src/storage/mod.rs`) chỉ nhận tên connection làm input — hai connection trùng tên (không gì ngăn cả) dùng chung thư mục migration, lẫn file của nhau; đổi tên connection qua `e` (sửa lỗi gõ, thao tác bình thường) làm "mồ côi" thư mục cũ mà không cảnh báo gì — panel đọc thư mục mới (rỗng hoặc khác), báo "up to date" dù file migration thật vẫn còn nguyên ở đường dẫn cũ.

Hai nửa fix, cả hai trong `ConnectionPickerComponent::apply_form_outcome` (`crates/tradar-app/src/components/connection_picker.rs`) vì `ConnectionFormComponent` tự nó không thấy được danh sách connection khác:

- **Chặn trùng tên**: so tên connection sắp lưu với mọi connection khác (bỏ qua chính nó khi đang Edit) — trùng thì giữ form mở, báo lỗi `"a connection named ... already exists"`, không lưu/không đóng form. Cùng vai trò với check tên/target rỗng `confirm()` đã làm, chỉ việc này cần thấy toàn danh sách nên đặt một lớp ngoài.
- **Tự di chuyển thư mục khi đổi tên**: `rename_migrations_dir(old_name, new_name)` mới trong `tradar-core::storage` — tính đường dẫn cũ/mới qua `default_migrations_dir`, `std::fs::rename` nếu thư mục cũ có tồn tại và thư mục mới chưa có ai chiếm (best-effort, im lặng cả hai chiều: không có gì để chuyển thì bỏ qua, đích đã có người ở thì giữ nguyên thư mục cũ thay vì ghi đè/merge — mất lịch sử migration vì một lần đổi tên còn tệ hơn phải dọn tay). Gọi ngay sau khi ghi `*slot = connection` trong `FormMode::Edit`, chỉ khi tên thật sự đổi.

Cân nhắc rồi bỏ qua: thêm `id` ổn định (không đổi khi rename) vào `SavedConnection`, khoá migrations dir theo đó thay vì theo tên — giải pháp "đúng" hơn về kiến trúc nhưng kéo theo đổi format file `connections.toml` (cần migrate file cũ thiếu field, backward-compat), rộng hơn hẳn phạm vi gap này; cách chọn (di chuyển thư mục khi rename + chặn trùng tên) giải quyết đúng 2 failure scenario audit tìm ra mà không đổi serialization format.

4 test mới: `move_dir_if_safe_moves_an_existing_directory_to_a_free_destination`/`_is_a_no_op_when_the_old_directory_never_existed`/`_leaves_the_old_directory_in_place_when_the_new_one_is_taken` (`tradar-core`, dùng tempdir — hàm logic chính, test trực tiếp không qua đường dẫn config thật), `adding_a_connection_with_a_duplicate_name_is_rejected`/`editing_a_connection_to_an_already_used_name_is_rejected`/`editing_a_connection_without_changing_its_name_is_not_a_duplicate_of_itself` (`tradar-app`, qua phím thật).

### 6. Export JSON: mọi cell xuất ra đều thành JSON string, kể cả sentinel `NULL`

`export::to_json`'s `Table` arm (`crates/tradar-query-workbench/src/export.rs`) bọc mọi cell thành `serde_json::Value::String` không phân biệt — `NULL` (sentinel null universal của mọi driver SQL-ish: Postgres/MySQL/SQLite/Cassandra/ClickHouse's `format_value`/`stringify_row` đều dùng đúng chuỗi này) xuất ra thành `"NULL"` chứ không phải `null` thật.

**Không** làm fix đầy đủ (suy luận kiểu số/bool từ chuỗi hiển thị) — đó cần `QueryResult::Table` mang theo kiểu dữ liệu từng cột, thứ không driver nào tính hiện tại (đổi sẽ lan sang mọi driver trả `Table`, ngoài phạm vi gap này) và tệ hơn: đoán kiểu từ chuỗi là **heuristic không an toàn** — một cột text chứa đúng chữ số `"123"` sẽ bị đoán nhầm thành number, biến "mọi thứ luôn là string" (sai nhưng nhất quán) thành "đôi khi sai kiểu một cách im lặng" (tệ hơn). Chỉ sửa đúng phần an toàn: `cell_to_json` nhận diện riêng sentinel `"NULL"` → `serde_json::Value::Null`, còn lại giữ nguyên là string — vì `"NULL"` đã là quy ước hiển thị cho null trên toàn app từ trước (không phải suy đoán mới), xuất ra `null` thật chỉ là tôn trọng đúng quy ước đó, không phải fix rủi ro mới.

1 test mới: `json_from_a_table_turns_the_null_sentinel_into_a_real_json_null`.

## Test

- `cargo build --workspace`: sạch.
- `cargo build --workspace --tests`: sạch (bao gồm mọi crate cần Docker — compile-only, không chạy được test thật trong sandbox này).
- `cargo clippy --all-targets --workspace -- -D warnings`: sạch.
- `cargo fmt --all -- --check`: sạch.
- `make test-unit`: toàn bộ pass — `tradar-core` 128 (125 + 3 test `move_dir_if_safe`), `tradar-query-workbench` 630 (627 + 2 table_designer + 1 export), `tradar-app` 167 (163 + 1 connection_form + 3 connection_picker), `tradar-connector-socket` 19, phần còn lại loại trừ vì cần Docker.
- `tradar-connector-redis`/`tradar-connector-cassandra`: build + clippy sạch; test Docker mới (gap #3, #4) không chạy được trong sandbox này, sẽ chạy ở CI.

## Chưa làm

- Cassandra's cap ở `MAX_ROWS` (gap #4) không có test xác nhận cắt đúng tại mốc — khác SQLite (dùng recursive CTE sinh dữ liệu không cần insert thật), CQL không có cách sinh hàng loạt dòng phía server; nạp `MAX_ROWS + 1` dòng thật qua `BATCH` sẽ vượt `batch_size_fail_threshold_in_kb` mặc định của Cassandra (~50KB, ước tính CQL text cho 10,001 insert ước ~400KB) → batch bị từ chối chắc chắn, không phải flake; qua `execute()` tuần tự 10,001 round trip thì đúng nhưng quá chậm cho một test CI. Logic đã verify kỹ bằng cách đọc trực tiếp source code `scylla` cục bộ (xem gap #4) và qua test tích hợp có sẵn phủ cả hai nhánh rẽ, nhưng nhánh "stream dừng đúng tại cap" cụ thể chưa có test tự động.
- Redis's cap (gap #3) không lan `truncated` tới UI — quyết định có chủ đích (chi phí đổi signature cho 12 connector), xem gap #3.
- Export JSON (gap #6) vẫn xuất số/bool dạng string — quyết định có chủ đích (đoán kiểu từ chuỗi không an toàn), xem gap #6. Fix đầy đủ cần `QueryResult::Table` mang kiểu cột, chưa có driver nào tính.
- Migrations (gap #5) chặn trùng tên/tự di chuyển thư mục khi rename, nhưng không thêm `id` ổn định cho `SavedConnection` — xem gap #5's "cân nhắc rồi bỏ qua".
