# Mongo: lệnh console (`show ...`, `db...`)

Chỉ `use` và `show dbs` chạy được; `show collections`, `db.getCollectionNames()`, `db.stats()`, `db.runCommand(...)`, `db.getCollection("x").find()`, và mọi câu có `;` ở cuối đều báo "expected a query starting with db.<collection>.<method>".

- `parse_shell_command` (thuần, có unit test) phân loại một dòng thành `Use`, `ShowDbs`, `ShowCollections` (`collections`/`tables`), `ShowUsers`, `ShowRoles`, `DbName` (`db`), `DbCall` (các method cấp database trong `DB_METHODS`) hoặc `Query` như cũ. `clean_command` bỏ comment, khoảng trắng và `;` cuối. `show` khác → lỗi nêu các lựa chọn hợp lệ.
- `execute` giữ `use`/`show dbs`; phần còn lại của helper vào `run_db_helper`: `list_collection_names` (sắp xếp), `run_command` cho `usersInfo`/`rolesInfo`/`dbStats`/`serverStatus`/`buildInfo`/lệnh tuỳ ý, `adminCommand` chạy trên database `admin`. Kết quả là một giá trị (danh sách = một mảng, phản hồi lệnh = một document, qua `into_relaxed_extjson` + `simplify_extjson`), riêng `show collections` là mỗi collection một `{name}`.
- `db.getCollection("tên").method(...)` vào được `parse_shell_query` (mongosh dùng cho tên collection không phải định danh). Một collection tên giống method db (`db.stats.find()`) vẫn là collection.
- `starts_mongo_statement` nhận `show ...` và `db` trơn nên nhiều lệnh trên nhiều dòng tách đúng.
- Kiểm chứng: unit test phân tích (alias, `;`, comment, lỗi `show` lạ, db vs collection, `getCollection`, tách câu). **Chưa chạy được trên Mongo thật** (không kéo được image Docker); đã thêm test Docker `execute_runs_the_console_helpers_against_a_real_mongo` (chưa chạy) — chạy `make test-mongo` trước khi tin vào phần thực thi.
- Chưa làm: `show profile`/`show log`, `db.getSiblingDB`, `db.<coll>.getIndexes()/distinct/...` (method cấp collection).

## Bổ sung: `distinct`, `getIndexes`, `estimatedDocumentCount`

Thêm vào `run_method` (không chain, không editable). Chỉ unit-test phần parse; phần thực thi chưa chạy trên Mongo thật (Docker Hub 429 trong sandbox).
