# Connector ClickHouse — xong (2026-09-29)

Mục "Connector mới" đầu tiên được làm trong roadmap (sau khi hoàn tất toàn bộ "Gap nhỏ, chưa scope" 2026-09-24). Chốt phạm vi qua `AskUserQuestion` trước khi code: người dùng chọn **chỉ ClickHouse** trong nhóm "MySQL / MariaDB / ClickHouse" ở roadmap (MySQL/MariaDB vẫn để mở, tách khỏi mục này).

## Chốt phạm vi (2 câu hỏi trước khi code)

1. **Kết nối/transport**: chọn **HTTP interface của ClickHouse qua `reqwest` + `FORMAT JSON`**, không phải crate `clickhouse` chính thức. Lý do: `sqlx` không hỗ trợ ClickHouse (khác Postgres/SQLite), và crate `clickhouse` (client Rust chính thức) thiết kế cho row kiểu tĩnh (`#[derive(Row)]`) — hợp với code biết trước schema, không hợp với query bất kỳ từ user mà driver không biết trước cột gì. HTTP interface + `FORMAT JSON` thì ClickHouse tự trả cả tên cột lẫn kiểu trong `meta`, khớp thẳng vào `QueryResult::Table` động đang có — đúng hệt cách connector Elasticsearch đã làm (`reqwest` + JSON), không thêm dependency mới (workspace đã có `reqwest`/`serde_json`).
2. **Row-edit ở v1**: **không làm**. ClickHouse không có `UPDATE`/`DELETE` chuẩn ANSI, chỉ có mutation `ALTER TABLE t UPDATE col=val WHERE ...` / `ALTER TABLE t DELETE WHERE ...` — chạy **bất đồng bộ** ở nền, không có hiệu lực ngay. Luồng "sửa cell → xác nhận `y` → chạy → tự đọc lại thấy ngay" đang có cho Postgres/SQLite/Mongo/Elasticsearch sẽ cho trải nghiệm sai kỳ vọng (đọc lại thấy dữ liệu cũ) nếu áp cho ClickHouse, nên bỏ hẳn thay vì làm nửa vời.

## Kiến trúc

`crates/tradar-connector-clickhouse` (crate mới, nối vào workspace `Cargo.toml` + `registry()` trong `tradar-app/src/main.rs`, đúng khuôn "Registry" trong `docs/architecture.md" — chỉ 2 chỗ cần đụng). Vẫn implement thẳng `Connector`+`QueryDriver` như Postgres/SQLite/Elasticsearch, không cần `Screen` riêng — `QueryEngine::new(driver, connection, schema)` xử lý hết, y hệt mọi driver SQL/JSON khác.

**`ClickHouseDriver`**: `base_url`/`database`/`user`/`password` (`Option`, điền lúc `connect()`, chưa có lúc construct — giống `PostgresDriver` để `pool: Option<PgPool>` trống tới `connect()`) + `reqwest::Client` dùng chung cho mọi request thay vì tạo mới mỗi lần.

**Target string**: `http://[user[:password]@]host[:port][/database]` — parse bằng `reqwest::Url` (đã có sẵn qua `reqwest`, không thêm crate `url` riêng) thay vì tự viết parser. `ParsedTarget` là struct đặt tên (không phải tuple 4 phần tử — bản đầu dùng tuple bị `clippy::type_complexity` chặn, đổi sang struct cho rõ nghĩa hơn luôn). `docker-compose.yml` đã có sẵn service `clickhouse` (`CLICKHOUSE_DB=mydb`, `CLICKHOUSE_USER=user`, `CLICKHOUSE_PASSWORD=password`) khớp đúng format này: `http://user:password@localhost:8123/mydb`.

**`connect()`**: chạy `SELECT 1` qua request có auth đầy đủ (không chỉ `GET /ping`, vì `/ping` của ClickHouse trả lời `Ok.` **không cần auth** — dùng nó để xác nhận "còn sống" sẽ bỏ sót sai user/password/database) — cùng tinh thần Postgres/Elasticsearch: `connect()` phải chứng minh **credential thật** chạy được, không chỉ "có gì đó trả lời".

**`ping()`** (chạy nền mỗi 15s qua `QueryEngine::tick`): dùng `GET /ping` — endpoint rẻ nhất ClickHouse có, không parse query, không cần auth. Đánh đổi đã biết: nếu credential/database bị thu hồi *sau* khi đã connect, `ping()` vẫn báo "còn sống" cho tới khi một query thật chạy và lộ ra — chấp nhận được vì mọi driver khác cũng chỉ coi ping là "chứng minh tới được", không phải "mọi quyền vẫn còn".

**`execute()`**: statement không trả dòng (`!query_driver::returns_rows`, heuristic dùng chung có sẵn — đã phủ cả `SHOW`/`DESCRIBE`/`EXPLAIN` của ClickHouse) chạy thẳng, trả `Affected { rows: 0 }` (ClickHouse không báo số dòng bị ảnh hưởng qua HTTP interface cho `INSERT`/`ALTER`/... — `0` là placeholder giống Postgres đã dùng cho `BEGIN`/`COMMIT`, không phải số thật). Statement trả dòng thì tự thêm `FORMAT JSON` vào cuối (trừ khi câu đã tự khai `FORMAT` riêng — dò bằng cách tìm từ "format" đứng riêng, case-insensitive, cùng kiểu heuristic-không-phải-parser với `returns_rows`/`transaction_control` sẵn có), parse response `{"meta": [...], "data": [...]}` thành `QueryResult::Table` — cột lấy thứ tự từ `meta` (không sort theo key JSON của từng row, tránh lệch thứ tự nếu client nào đó sắp lại key). Cắt về `MAX_ROWS` (10.000) phía client sau khi tải về — **chưa** ép `LIMIT` phía server (xem "Chưa làm").

**Dịch 1 cell**: `NULL` → chuỗi `"NULL"` (khớp quy ước mọi driver khác), `String` → nguyên văn không escape, còn lại (số/bool/`Array`/`Tuple`/`Map`/kiểu lồng nhau ClickHouse render ra JSON array/object) → JSON compact qua `serde_json::Value::to_string()` — không có kiểu decode riêng từng loại như `sqlx` cho Postgres vì response đã là JSON động sẵn, một hàm là đủ cho mọi kiểu.

**`list_schema()`**: một query `system.columns` (join tới `system.tables` không cần thiết, `system.columns` đã có `database`/`table`/`name`/`type`/`is_in_primary_key` cho từng cột), loại `system`/`information_schema`/`INFORMATION_SCHEMA`. Nhóm theo `(database, table)` giống hệt vòng lặp Postgres dùng cho `(schema, table)` — ClickHouse "database" đóng đúng vai trò `SchemaInfo::schema` (giống Postgres schema/Cassandra keyspace/MongoDB database), nên navigator tự có thêm cấp database → table → column mà không cần sửa `flatten_outline`. `is_in_primary_key` (cột `UInt8`, "0"/"1" sau khi qua JSON) map vào `ColumnInfo::primary_key` — có ghi rõ trong comment đây là sorting/index key của ClickHouse, không phải ràng buộc duy nhất kiểu PK quan hệ, và hiện **không** có gì dùng nó để hành động (vì row-edit không làm ở v1) nên chỉ là metadata hiển thị đúng thuật ngữ ClickHouse. `foreign_key` luôn `None` (ClickHouse không có FK khai báo, giống Cassandra). `qualify_colliding_names` áp dụng như Postgres, phòng hai database trùng tên bảng.

**`keywords()`**: trả thẳng `query_driver::SQL_KEYWORDS`, không tự thêm từ khoá riêng của ClickHouse (`ENGINE`, `ARRAY JOIN`, ...) — theo đúng tiền lệ Cassandra đã đặt (CQL cũng khác SQL nhưng vẫn chỉ trả `SQL_KEYWORDS` trơn, không tự mở rộng riêng cho từng dialect).

**Highlight cú pháp**: `query_screen.rs`'s danh sách driver dùng `Dialect::Sql` (tree-sitter) thêm `"clickhouse"` bên cạnh `"postgres"`/`"sqlite"` — ClickHouse SQL đủ giống ANSI để `tree-sitter-sequel` tô màu hợp lý.

**Icon**: `📊` (chưa driver nào dùng, hợp với vai trò OLAP/phân tích của ClickHouse).

## Test

`crates/tradar-connector-clickhouse/src/lib.rs` — 6 unit test thuần (không cần Docker, chạy được trong sandbox): `parse_target` (tách đúng userinfo/path/base URL 3 trường hợp), `has_format_clause` (case-insensitive, whole-word), `clickhouse_cell_to_string` (4 dạng JSON), `table_from_json` (thứ tự cột theo `meta`, không theo key JSON). 7 test tích hợp qua `testcontainers-modules` (feature `clickhouse`, đã có sẵn ở đúng version `0.11` workspace đang pin, không cần bump): `connect` thành công/thất bại rõ lý do (database không tồn tại), `execute` chạy DDL/DML rồi đọc lại đúng dữ liệu, `execute` báo lỗi cú pháp từ chính ClickHouse, `list_schema` liệt kê đúng bảng vừa tạo + nhóm theo database + không rò `system`, `ping` thành công. **Chưa chạy được trong sandbox này** (không có Docker daemon — cùng giới hạn đã ghi trong CLAUDE.md cho 8 connector kia), nhưng theo đúng khuôn test của Postgres/Elasticsearch nên tin cậy được qua review.

`Makefile`: thêm `tradar-connector-clickhouse` vào exclude list của `test-unit`, thêm vào `test-docker` và target `test-clickhouse` riêng — đúng khuôn 8 connector Docker-dependent đã có.

## Chưa làm (để lại, không tự chốt trước)

- Row-edit (sửa cell/xoá dòng) — cố tình bỏ ở v1, xem phần chốt phạm vi.
- CRUD snippet (`c`/`r`/`u`/`d` trong navigator) — cùng lý do row-edit: `Update`/`Delete` cần cú pháp `ALTER TABLE` khác hẳn `UPDATE`/`DELETE` chuẩn mà `build_crud_snippet` dùng chung sinh ra, làm nửa vời (chỉ `Create`/`Read` đúng) dễ gây hiểu lầm hơn là không có.
- Ép `LIMIT`/`FORMAT JSONCompact` phía server để tránh tải cả kết quả khổng lồ về trước khi cắt `MAX_ROWS` phía client — hiện chỉ cắt sau khi tải, chấp nhận được cho v1 vì mọi driver SQL khác trong app cũng không tự thêm `LIMIT`.
- Phân biệt Table/View/MaterializedView trong navigator qua `object_kind` (Postgres đã có) — `system.tables.engine` có đủ thông tin nhưng chưa làm, để navigator ClickHouse phẳng một loại object như Cassandra/MongoDB.
- `TLS`/`https://` — target hiện giả định `http://`; một target `https://...` vẫn parse được qua `reqwest::Url` (không chặn) nhưng chưa test riêng.
