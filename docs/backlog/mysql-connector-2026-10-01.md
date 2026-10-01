# MySQL/MariaDB connector (2026-10-01)

## Bối cảnh

Roadmap liệt "MySQL / MariaDB" ở mục "Connector mới, đã lên kế hoạch nhưng chưa code" — rẻ nhờ kiến trúc pluggable có sẵn, qua `sqlx` gần giống hệt connector Postgres đang có. Chốt qua `AskUserQuestion` sau khi merge PR feature-flags (2026-10-01): làm tiếp "MySQL/MariaDB connector" trong số các hạng mục còn mở (Kafka Groups mode, gRPC spike, CLI import/export) vì rẻ nhất và kiến trúc đã sẵn sàng.

Một driver cho cả hai server: MariaDB nói cùng wire protocol mà sqlx's `mysql` feature nhắm tới, và toàn bộ truy vấn `information_schema` trong driver là SQL chuẩn cả hai server hỗ trợ giống hệt nhau — không cần nhánh riêng theo server nào trong suốt crate.

## Thiết kế

`crates/tradar-connector-mysql` mới, mô phỏng sát `tradar-connector-postgres` (connect/transaction/error-format/stringify_column/Connector impl có cùng shape), khác ở ba chỗ:

- **`list_schema`**: đơn giản hơn Postgres nhờ MySQL's `information_schema` phẳng hơn chuẩn SQL — `information_schema.columns.column_key = 'PRI'` cho primary key trực tiếp (Postgres cần join `table_constraints`/`key_column_usage`), và `information_schema.key_column_usage.referenced_table_name IS NOT NULL` cho foreign key trực tiếp (Postgres cần join thêm `constraint_column_usage`). Lọc 4 schema hệ thống (`information_schema`, `mysql`, `performance_schema`, `sys` — `sys` chỉ có ở MySQL, không có ở MariaDB nhưng liệt vô hại).
- **Lỗi cú pháp**: MySQL/MariaDB không trả vị trí ký tự qua wire protocol như Postgres. Thông báo lỗi cú pháp của cả hai quy ước trích token gây lỗi dạng `... right syntax to use near 'X' at line N` — `format_mysql_error`/`near_token_marker` dò token đó trong câu gốc rồi gọi `query_driver::line_and_caret` dùng chung với Postgres/SQLite, đúng kiểu thủ thuật SQLite's `near "X": syntax error` đã dùng (khác dấu nháy: đơn so với kép).
- **Giải mã cột (`stringify_column`)**: `sqlx::query()` (API động, không macro) chạy qua binary protocol nên `String::try_get` fail âm thầm thành `"NULL"` cho cột không phải kiểu text — giống vấn đề Postgres đã gặp. Khác Postgres (mỗi độ rộng int cần đúng kiểu Rust tương ứng), sqlx-mysql's `int`/`uint` decode chấp nhận **bất kỳ** kiểu int nào bất kể độ rộng khai báo (xác nhận qua đọc source `sqlx-mysql-0.8.6/src/types/int.rs`/`uint.rs` — `int_compatible`/`uint_compatible` chỉ check `ColumnType`, không check exact width), nên chỉ cần `i64`/`u64` phủ hết `TINYINT`..`BIGINT`; tương tự `f64` phủ cả `FLOAT`/`DOUBLE` (sqlx-mysql's `f64::decode` tự xử lý cả buffer 4 lẫn 8 byte). `DECIMAL` cố tình bỏ qua — sqlx-mysql's `f64::decode` tự chối decode `DECIMAL` ("differing semantics"), decode đúng cần feature `bigdecimal`/`rust_decimal` chưa bật.
- **Capabilities**: `[Query, Schema, Export]`, giống Postgres trừ không có `Explain` (chưa driver nào thật sự dùng field này).
- **Chưa làm ở v1** (nhất quán với cách Postgres's table designer/migration tracking từng là sub-project tách riêng sau connector gốc): `table_ddl`/`supports_migrations` không override, rơi về default (`None`/`false`) của `QueryDriver` — MySQL chưa có table designer hay migration tracking riêng.

`docker-compose.yml` thêm service `mysql` (image `mysql:9`, user/password/db giống pattern Postgres) cho dev thủ công; `Makefile` thêm `test-mysql` + `mysql` vào `DOCKER_SERVICES`/`test-unit`'s exclude list/`test-docker`'s dependency list.

Wiring vào `tradar-app`: dependency `tradar-connector-mysql` optional + feature `mysql` trong `Cargo.toml` (đúng pattern feature-flags vừa làm ở PR trước), `#[cfg(feature = "mysql")]` trong `registry()`, entry `target_hint("mysql")` trong `connection_form.rs`. Driver id `"mysql"` dùng chung cho cả MySQL lẫn MariaDB — không có id riêng `"mariadb"`.

`tradar-query-workbench`'s `query_screen.rs` thêm `"mysql"` vào match `Dialect::Sql` (cùng chỗ với `"postgres" | "sqlite" | "clickhouse"`) — tree-sitter SQL highlighting và `za` statement-folding tự áp dụng cho MySQL vì cả hai tính năng đều key theo `Dialect::Sql`, không cần gate riêng.

## Test

Không có Docker trong sandbox hiện tại (xác nhận qua `docker info`), nên 10/16 test (mọi test dùng `testcontainers-modules`'s `mysql` module) không chạy được ở đây — cùng tình trạng chín trong mười connector khác của dự án đã ghi trong `CLAUDE.md`. Đã verify được:

- `cargo check -p tradar-connector-mysql --all-targets`: sạch.
- `cargo clippy -p tradar-connector-mysql --all-targets -- -D warnings`: sạch.
- `cargo test -p tradar-connector-mysql`: 6/16 pass (không cần Docker — `near_token_marker` × 3, `crud_snippet_delegates...`, `table_ddl_and_migrations_are_not_supported_at_v1`, `connect_fails_quickly_against_an_unreachable_host`), 10 fail vì thiếu Docker daemon (đúng dự kiến, test container-based).
- `make test-unit`: chạy sạch toàn bộ (163 test `tradar-app` + mọi test non-Docker khác), không cần sửa file thủ công.
- `cargo clippy --all-targets --workspace --exclude tradar-connector-kafka --exclude tradar-app -- -D warnings`: sạch (bao gồm `tradar-connector-mysql`).
- `cargo clippy -p tradar-app --all-targets --no-default-features --features postgres,mysql,sqlite,mongo,elasticsearch,redis,cassandra,clickhouse,rabbitmq,http,socket -- -D warnings`: sạch.
- `cargo check -p tradar-app --no-default-features --features postgres,mysql,sqlite,mongo,elasticsearch,redis,cassandra,clickhouse,rabbitmq,http,socket` (full trừ kafka, bao gồm mysql): thành công.
- `make build-slim FEATURES=mysql` (chỉ MySQL, không connector nào khác): thành công, log compile xác nhận không kéo `scylla`/`rdkafka`/postgres-sqlite-...-khác.
- `cargo test -p tradar-app --no-default-features --features postgres,mysql,sqlite,mongo,elasticsearch,redis,cassandra,clickhouse,rabbitmq,http,socket target_hint`: cả 4 test `target_hint` pass, bao gồm `target_hint_covers_every_compiled_in_connector` (xác nhận `"mysql"` đã có hint).
- `cargo fmt --all -- --check`: sạch.

Type-decode logic (`stringify_column`) được xác minh đúng qua đọc trực tiếp source `sqlx-mysql-0.8.6` đã cache local (`int.rs`/`uint.rs`/`float.rs`/`bool.rs`/`type_info.rs`/`protocol/text/column.rs`), không chỉ suy đoán từ tài liệu — xác nhận tên kiểu wire (`"INT UNSIGNED"`, `"BOOLEAN"` cho `TINYINT(1)`, ...) và phạm vi tương thích decode (`int_compatible`/`uint_compatible` chấp nhận mọi độ rộng, không riêng từng kiểu Rust) trước khi viết match arm, để tránh lặp lại kiểu lỗi "known gap" mà Postgres connector từng gặp (silent NULL cho kiểu không match).

## Chưa làm

- Table designer (DDL qua UI) và migration tracking cho MySQL — cả hai hiện chỉ Postgres; mở rộng sang MySQL là sub-project riêng nếu có nhu cầu cụ thể (DDL MySQL khác Postgres ở backtick identifier, `AUTO_INCREMENT` thay vì `SERIAL`, không có `IF NOT EXISTS` đồng nhất cho mọi loại constraint).
- Verify thật với Docker (sandbox hiện tại không có daemon) — 10 test testcontainers-based cần chạy trên máy/CI có Docker trước khi coi là đã verify end-to-end.
- `DECIMAL` vẫn hiện `NULL` trong results grid (known gap, xem doc comment `stringify_column`) — cần thêm sqlx feature `bigdecimal`/`rust_decimal` + dependency tương ứng nếu muốn decode đúng.
