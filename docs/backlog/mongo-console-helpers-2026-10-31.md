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

## Bổ sung: `show profile`, `getSiblingDB`

`show profile` đọc 20 dòng mới nhất của `system.profile` (sort `ts` giảm dần). `db.getSiblingDB("x").<lệnh>` được tách ở `split_sibling_db` rồi chạy lệnh còn lại trên database `x` cho đúng câu đó (không đổi `current_db`); `edit_source` từ chối câu kiểu này. Chỉ unit-test phần parse; chưa chạy trên Mongo thật.

## Bổ sung: `.explain()`, `.hint()`, `.count()` phía server

Phản hồi "`.sort` chưa ăn index": không có cách nào kiểm tra, và `find(...).count()` trước đây đếm bằng cách đi hết cursor ở client (kéo mọi document về, không dùng index). Giờ `.count()` gọi `count_documents` (server đếm, qua index nếu có; tính `skip`/`limit`/`hint`), thêm `.hint(<tên|key doc>)` và `.explain()` (lệnh `explain` với `executionStats` trả plan thật để xem `IXSCAN`/`COLLSCAN`). Thứ tự key của `sort` được giữ nguyên (test). Test thật cần Docker (`explain_hint_and_count_go_through_the_server_and_its_index`), chưa chạy được trong sandbox; phần kiểm tra chain chạy được và đạt.

## Bổ sung: Elasticsearch aggregation không hiện kết quả

`_search` luôn bị bóc thành từng hit (`unwrap_search_hits`), nên một response có `aggregations`/`suggest` (thường `"size": 0`, không hit) ra lưới rỗng. Giờ response như vậy được giữ nguyên làm một document (test `a_search_with_aggregations_keeps_the_whole_response`), và plugin mở một document lồng nhau ở dạng JSON thay vì bảng một dòng rất rộng (`default_view`). Chưa chạy trên cluster ES thật (Docker 429).

## Bổ sung: Mongo/ES mặc định hiện JSON

Kết quả `documents` (Mongo, Elasticsearch, Redis) giờ mở ở dạng JSON; `<leader>rv` sang bảng để sửa ô/xoá dòng; `setup{ documents_view = 'table' }` đổi mặc định. Thay cho luật tự đoán trước đó (một document lồng nhau → JSON).

## Bổ sung: `_index` cho search nhiều index

`unwrap_search_hits` giữ `_index` khi các hit đến từ nhiều hơn một index (`logs-*`, `a,b`); search một index vẫn như cũ (không lặp `_index` ở mọi dòng, lưới sửa được không đổi). Test `hits_from_several_indices_keep_their_index`.
