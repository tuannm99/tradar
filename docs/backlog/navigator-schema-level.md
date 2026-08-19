# Navigator: cấp schema/database + nhóm theo loại object — xong (2026-08-19)

Gap #1 trong đợt so sánh DataGrip/DBeaver/Studio3T (`docs/roadmap.md`), Tier 2 — nền tảng cho Tier 3 (#6 autocomplete sâu, #5 ERD). Đảo ngược một phần quyết định "dừng ở 2 cấp" đã chốt lúc xây navigator ban đầu (`docs/backlog/roadmap-sub-project.md` mục 6) — lý do dừng khi đó (mỗi connector trả gì khi không có khái niệm schema/view/function) vẫn đúng về mặt kỹ thuật, giờ chỉ là đã trả lời được cho 3 connector: Postgres (schema), Cassandra (keyspace), MongoDB (database). Chọn qua `AskUserQuestion` (2026-08-19): làm cả cấp schema/database **và** nhóm theo loại object trong cùng một lần, cho Postgres/Cassandra/MongoDB.

## Kiến trúc dữ liệu

`SchemaInfo` (`crates/tradar-query-workbench/src/query_driver.rs`) thêm 2 field:
- `schema: Option<String>` — schema/keyspace/database của entry, `None` khi connector không có khái niệm đó (SQLite/Elasticsearch/Redis).
- `object_kind: Option<String>` — kiểu object, chữ thường số ít (`"table"`/`"view"`/`"function"`/`"procedure"`), `None` khi connector chỉ có đúng 1 loại (mọi connector trừ Postgres).

`OutlineEntry` (`crates/tradar-core/src/action.rs`) thêm `is_object: bool` — thay cho quy ước cũ "depth 0 = đối tượng thật, dùng được CRUD snippet". Quy ước cũ vỡ ngay khi cây có thể sâu hơn 2 cấp (folder schema/kind cũng nằm ở depth 0/1 tuỳ connector) — `choose_snippet` trong `navigator.rs` trước đây check `depth != 0`, giờ check `!is_object`; bug thật, không phải giả định — nếu không sửa, `c`/`r`/`u`/`d` đứng trên một folder schema Postgres sẽ generate CRUD snippet sai (folder không có "tên bảng" thật).

## `flatten_outline` — thuật toán chung, không đặc biệt hoá theo connector

`flatten_outline` (`crates/tradar-query-workbench/src/components/query_screen.rs`) gom `Vec<SchemaInfo>` thành cây bằng 2 lượt group-by-first-seen-order (không sort — giữ đúng thứ tự driver trả về):
1. Group theo `schema`. `Some(name)` → chèn folder depth 0, con xuống depth 1. `None` → không chèn gì, giữ depth 0 (đúng hành vi cũ).
2. Trong mỗi group, group tiếp theo `object_kind`. `Some(kind)` → chèn folder depth+1 (label hoá qua `kind_label`: "table"→"Tables", "view"→"Views", ...). `None` → không chèn, bảng nằm thẳng ở depth hiện tại.

Kết quả: connector không set field nào (SQLite/Elasticsearch/Redis, và mọi test hiện có) có cây **y hệt trước khi tính năng này tồn tại** — cùng depth 0/1, cùng thứ tự — verify bằng test `flatten_outline_with_no_schema_or_kind_is_unchanged_from_the_flat_tree`. Cassandra/MongoDB có `schema` nhưng không có `object_kind` (chỉ 1 loại object) nên chỉ thêm đúng 1 cấp, không có folder "Tables" thừa bọc quanh loại duy nhất đó. Postgres có cả hai nên cây sâu tới 4 cấp: schema → kind → table/view/function/procedure → cột.

## Mỗi connector implement những gì

- **Postgres**: `list_schema` bỏ filter `table_schema = 'public'`, thêm cột `table_schema`/`table_type` vào cùng join hiện có (không cần query views riêng — `information_schema.tables.table_type` đã phân biệt `BASE TABLE`/`VIEW`); thêm 1 round trip riêng cho `information_schema.routines` (functions/procedures, không có cột nên không tái dùng được join bảng). Loại trừ `pg_catalog`/`information_schema` cùng cách Cassandra loại trừ `system*`. **Indexes/Triggers cố tình bỏ khỏi scope** — quyết định lúc code, không phải lúc chốt phạm vi ban đầu: DataGrip/DBeaver thật đặt chúng làm con của từng bảng, không phải folder ngang hàng Tables/Views ở cấp schema; đúng chỗ của chúng là #2 Table designer khi có UI xem chi tiết 1 bảng.
- **Cassandra**: `schema: Some(keyspace)` thêm vào entry đã có sẵn (chỉ 1 dòng); `object_kind` để `None` vì Cassandra không có view/function/procedure đúng nghĩa (đã ghi nhận từ lúc scope ban đầu). **`name` giữ nguyên dạng `keyspace.table` như trước** — không rút gọn xuống bare name dù giờ đã đứng trong folder keyspace (hơi thừa về hiển thị) vì CQL cần tên đủ để chạy đúng khi không có `USE` trước, và đây là hành vi cũ đã test/chạy thật — đổi sẽ là thay đổi hành vi suy đoán, không phải yêu cầu của tính năng này.
- **MongoDB**: `list_schema` viết lại hoàn toàn — trước chỉ liệt kê collection của 1 database (database hiện tại lúc connect), giờ `client.list_databases()` rồi với mỗi database (trừ `admin`/`local`/`config`) liệt kê collection của nó, cả hai bước đều chạy song song qua `join_all` (cùng idiom với việc song song hoá sample-1-document-mỗi-collection đã có sẵn) — round trip tăng nhưng vẫn bounded bởi request chậm nhất, không phải tổng N request nối tiếp. Vẫn là snapshot 1 lần lúc connect (đúng model "navigator không có live-refresh hook" đã ghi từ trước) — chỉ là giờ chụp mọi database thay vì 1 cái, không phải kiến trúc mới.

## `qualify_colliding_names` — vấn đề tên trùng giữa 2 schema/database

`SchemaInfo.name` đóng vai trò kép: vừa là label hiển thị, vừa là định danh `build_crud_snippet`/`column_types` khớp với tên bảng đọc thẳng từ `FROM` clause của câu query thật. Ban đầu định luôn qualify tên bằng `schema.table` (giống cách Cassandra vẫn làm) cho nhất quán — **sai**: phần lớn câu Postgres viết `FROM users` (bare, dựa vào `search_path` mặc định), nếu `SchemaInfo.name` luôn là `"public.users"` thì match `entry.name == "users"` vỡ ngay cho **mọi** cài đặt Postgres chỉ có 1 schema — tức là trường hợp phổ biến nhất, không phải edge case.

Giải pháp: `qualify_colliding_names(&mut [SchemaInfo])` (`query_driver.rs`, dùng chung cho Postgres và MongoDB) chỉ qualify tên khi nó **thật sự trùng** với một entry khác ở schema/database khác trong cùng lần fetch — đếm tần suất bare name, chỉ những cái xuất hiện >1 lần mới bị đổi thành `schema.name`. Một cài đặt 1-schema (tuyệt đại đa số) không đổi gì so với trước tính năng này tồn tại; chỉ khi thật sự có 2 schema cùng tên bảng thì mới phân biệt — đúng tinh thần "khi nghi ngờ, đừng đoán sai" đã dùng cho `single_table_source`.

## `choose_snippet` — sửa đúng chỗ giả định `depth == 0` từng đứng

`crates/tradar-app/src/components/navigator.rs`'s `choose_snippet` (phím `c`/`r`/`u`/`d`) trước đây check `outline_entry.depth != 0` để loại cột/connection ra. Giờ check `!outline_entry.is_object` — loại đúng cả cột lẫn mọi folder nhóm (schema/keyspace/database, Tables/Views/...) bất kể folder đó nằm ở depth nào. Test mới `choosing_a_snippet_on_a_grouping_folder_does_nothing_even_at_depth_zero` dựng thẳng một folder ở depth 0 để xác nhận không còn dựa vào depth nữa.

## Verify thật (không chỉ unit test)

Dựng container Postgres tạm qua `docker run` (không phải testcontainers — smoke test thủ công), tạo schema `public`/`reporting`, bảng, 1 view, 1 function trong `public`. Chạy `tradar` thật trong tmux, mở navigator, xác nhận: `public`/`reporting` hiện đúng làm folder cấp 1; `public` mở ra đúng 3 folder "Views"/"Tables"/"Functions" (không có "Procedures" — không tạo cái nào, đúng "ẩn khi rỗng"); `reporting` chỉ có "Tables" (không "Views"/"Functions" — đúng "ẩn khi chỉ có 1 loại"); cột hiện đúng dưới bảng; phím `r` trên bảng `users` sinh đúng `SELECT * FROM "users" LIMIT 100;`, cùng phím đó trên folder "Views" không làm gì. Backup/restore `~/.config/tradar/` nguyên vẹn sau khi test xong (diff xác nhận khớp 100% bản gốc).

## Test

- `crates/tradar-query-workbench/src/components/query_screen.rs`: `flatten_outline_*` (4 test) — folder schema khi có, folder kind khi có, cột luôn sâu hơn bảng đúng 1 cấp bất kể độ sâu, và cây không đổi khi không connector nào set field mới.
- `crates/tradar-app/src/components/navigator.rs`: `choosing_a_snippet_on_a_grouping_folder_does_nothing_even_at_depth_zero`.
- `crates/tradar-connector-postgres/src/lib.rs`: group nhiều schema + ẩn `pg_catalog`/`information_schema`, object_kind đúng cho table/view/function/procedure, qualify chỉ tên trùng.
- `crates/tradar-connector-mongo/src/lib.rs`: cover mọi database + ẩn `admin`/`local`/`config`, qualify chỉ tên trùng giữa 2 database.
- Toàn bộ suite Docker-dependent (Postgres/Cassandra/MongoDB/Elasticsearch/Redis) chạy lại pass; 2 lần fail gặp phải (Cassandra port-race, Elasticsearch container flake khi chạy song song nhiều container) đều biến mất khi chạy `--test-threads=1`, xác nhận là nhiễu môi trường Docker chứ không phải regression.
