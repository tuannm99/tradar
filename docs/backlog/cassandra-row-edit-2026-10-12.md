# Cassandra: sửa/xoá dòng ngay trong grid (2026-10-12)

## Bối cảnh

Mục kế tiếp sau "Completion sau `keyspace.`/`schema.`" trong checklist "Server headless + plugin Neovim" — người dùng chọn làm trước gRPC (cần chốt scope) và kênh push cho Kafka/RabbitMQ/HTTP/Socket (kiến trúc lớn hơn). Ghi chú gốc trong roadmap: "driver cần nhớ kiểu cột từ `list_schema` để sinh literal CQL đúng kiểu: uuid/số không nháy".

## Vấn đề

`QueryDriver::edit_sql`/`edit_source` (sửa ô/xoá dòng trong grid kết quả) chưa override cho Cassandra — xem ghi chú cũ trong `docs/architecture.md`. `build_sql_edit` dùng chung (Postgres/MySQL/SQLite) luôn quote giá trị thành chuỗi SQL (`'1'`), dựa vào Postgres/SQLite **tự coerce** `'1'` thành số khi cột là số — một shape viết đúng cho mọi kiểu cột mà không cần biết kiểu nào. CQL **không coerce** như vậy: `WHERE id = '123e4567-...'` cho cột `uuid` hay `WHERE count = '7'` cho cột `int` bị CQL từ chối thẳng với lỗi kiểu (`Invalid STRING constant ... for int`), không phải chạy sai lặng lẽ — nhưng vẫn là sửa/xoá không chạy được.

## Khảo sát: tại sao không tra kiểu cột từ schema

Hướng đầu tiên nghĩ tới: cache `type_name` của từng cột (đã có sẵn trong `ColumnInfo` từ `list_schema`) rồi `edit_sql` tra cache đó để quyết định nháy hay không. Vấn đề: `QueryDriver::edit_sql(&self, edit: &RowEdit) -> Option<String>` là method **sync** — không gọi lại `list_schema()` (async, một round-trip CQL thật tới `system_schema.columns`) được từ trong nó. Cứ dựa vào một cache hy vọng đã được điền từ một lần gọi `list_schema()` trước đó (lúc mở schema panel, connect, ...) thì có nguy cơ cache trống (chưa ai gọi `list_schema()` lần nào) đúng lúc cần sửa dòng đầu tiên — không đáng tin.

Hoá ra không cần: đọc lại `tradar-connector-mongo`'s `edit_sql`/`mongo_value_literal` và Elasticsearch's `edit_sql` riêng mới thấy cả hai đã giải đúng vấn đề này trước rồi, theo cách khác hẳn — suy luận kiểu **từ chính text giá trị** (số/bool/null thì để trần, còn lại quote) thay vì tra schema. Không cần cache, không cần round-trip, không có nguy cơ "chưa load được".

## Thay đổi

### `cql_value_literal` + `looks_like_cql_uuid` (mới, riêng crate Cassandra)

Suy luận trực tiếp từ text:
- `true`/`false` (không phân biệt hoa thường) → để trần, chuẩn hoá chữ thường (CQL boolean literal là `true`/`false` thường).
- Parse được bằng `str::parse::<f64>` → để trần nguyên văn (không re-format qua f64, chỉ dùng để *kiểm tra* có hợp lệ không — số nguyên, số thực, số âm, scientific notation đều hợp lệ CQL).
- Hình dạng UUID (5 nhóm hex ngăn bởi `-`, đúng độ dài `8-4-4-4-12`) → để trần (`uuid`/`timeuuid` dùng chung cú pháp literal không nháy, không cần phân biệt hai kiểu này ở bước này).
- Còn lại → quote làm chuỗi CQL (`'...'`, escape `'` thành `''`).

Biết trước, chấp nhận: gõ đúng chữ `NaN`/`inf`/`infinity` vào ô sửa một cột `text` sẽ bị để trần nhầm (vì `f64::from_str` chấp nhận các chữ đó) — hợp lệ cho `float`/`double`, lỗi kiểu cho `text`. Cùng dạng rủi ro hẹp mà `mongo_value_literal` đã có sẵn cho chữ `null`.

### `build_cql_edit` (mới, riêng crate Cassandra)

Cùng hình dạng `UPDATE ... SET ... WHERE ...`/`DELETE FROM ... WHERE ...` như `build_sql_edit`, chỉ đổi chỗ quote giá trị sang `cql_value_literal`. `WHERE` nối các điều kiện bằng `AND` trên từng cột khoá riêng — hợp lệ CQL cho so sánh bằng (`=`), không cần cú pháp tuple `(a, b) = (...)` (cú pháp đó chỉ cần cho `IN`/range trên clustering column).

### `quote_identifier` trong `query_driver.rs` đổi thành `pub`

`build_cql_edit` cần quote identifier kiểu `"keyspace"."table"` giống `build_sql_edit` đã làm — thay vì chép lại cùng 4 dòng logic trong crate Cassandra, đổi `quote_identifier` (trước đó private) thành `pub` rồi dùng lại thẳng. Không đổi hành vi gì của các driver đang dùng nó gián tiếp qua `build_sql_edit`/`build_crud_snippet`/`build_table_ddl`.

### `CassandraDriver::edit_source`/`edit_sql`

`edit_source` → `query_driver::single_table_source(query)` (dùng chung, không cần sửa gì — cú pháp `SELECT ... FROM keyspace.table` của CQL khớp đủ). `edit_sql` → `build_cql_edit(edit)`. `edit_key_columns` không override — mặc định rơi về cột `primary_key` của schema, mà `list_schema` của Cassandra đã đúng từ trước (partition key + clustering, đúng thứ tự).

## Test

7 unit test mới trong `tradar-connector-cassandra`: `cql_value_literal` để trần số/bool/uuid đúng, quote mọi thứ khác (gồm một chuỗi "giống" uuid nhưng sai độ dài nhóm); `edit_source` delegate đúng; `edit_sql` build đúng cho `UPDATE` (khoá uuid để trần), `DELETE` với khoá composite (một cột số để trần, một cột text quote), và khoá `NULL` (ra `IS NULL`, không biến `NULL` thành literal).

Một test tích hợp (Docker, không chạy được trong sandbox này) mở rộng test cũ `schema_and_execute_round_trip_through_a_real_cluster`: build `edit_sql` thật cho `UPDATE` và `DELETE` trên bảng khoá composite `(user_id int, group_id int)`, **chạy** câu đó qua `execute()` thật, rồi `SELECT` lại xác nhận giá trị đã đổi / dòng đã mất — không chỉ so sánh chuỗi sinh ra, vì nếu quote sai kiểu, CQL sẽ từ chối thẳng câu lệnh (không khớp sai lặng lẽ) nên chạy thật là cách duy nhất biết chắc literal đúng.

`cargo build --workspace`, `cargo test -p tradar-query-workbench` (641 pass), `cargo test -p tradar-connector-cassandra --lib -- --skip docker` (18 pass), `cargo test -p tradar-app` (167 pass), `cargo clippy --all-targets --workspace -- -D warnings`, `cargo fmt --all -- --check` đều sạch.

## Chưa làm

- Kiểu `blob`/collection (`list<...>`/`set<...>`/`map<...>`/`tuple<...>`) không được `cql_value_literal` xử lý đúng (rơi vào nhánh quote-làm-chuỗi, sai cú pháp CQL cho các kiểu đó) — collection không thể là `PRIMARY KEY` (CQL tự cấm) nên không bao giờ là cột khoá, và sửa *giá trị* một collection qua một ô nhập text đơn không phải việc của tính năng này.
- Cạnh hẹp: gõ đúng chữ `NaN`/`inf`/`infinity` vào ô sửa cột `text` bị để trần nhầm — xem ghi chú ở `cql_value_literal`.
- Không verify qua Cassandra thật trong phiên này (không có Docker ở sandbox) — test tích hợp viết sẵn, chạy được ở CI/máy có Docker.
