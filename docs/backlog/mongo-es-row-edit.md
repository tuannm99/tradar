# Mở row-edit cho MongoDB và Elasticsearch — xong (2026-09-07)

Trước đó kết quả Mongo/Elasticsearch trong app luôn read-only (cố tình, ghi trong `docs/backlog/known-issues.md`). User hỏi thẳng "mở row-edit cho mongo với es" — chốt phạm vi qua `AskUserQuestion` (2 vòng, vì phát hiện giữa chừng làm sai một giả định ban đầu, xem mục "Phát hiện giữa chừng" bên dưới) trước khi code, đúng pattern mọi sub-project khác:

- **Phạm vi**: cả sửa cell (`Enter`) lẫn xoá row (`d`), giống hệt Postgres/SQLite — không tách nhỏ.
- **Kiểu field khi sửa cell**: cho sửa mọi field, tự suy luận kiểu JSON từ chuỗi gõ vào (không giới hạn chỉ string) — chấp nhận rủi ro đoán sai kiểu như một giới hạn đã biết, thay vì giới hạn phạm vi sửa.
- **Query nào coi là "editable"**: bảo thủ như `single_table_source` của SQL — Mongo chỉ `find()` đơn giản trên 1 collection, Elasticsearch chỉ `_search` đơn giản trên đúng 1 index (không multi-index/wildcard/`_all`).

## Vì sao không thể ghi giá trị như SQL

`build_sql_edit` (dùng chung cho Postgres/SQLite) luôn ghi giá trị mới thành string literal, vì Postgres/SQLite tự coerce `'1'` thành số khi cột cần số — một hình dạng ghi đè phủ mọi kiểu cột. MongoDB/Elasticsearch **không** coerce kiểu như vậy: `{_id: "ObjectId(...)"}` sẽ lọc theo đúng chuỗi đó, không gọi constructor; `{"doc": {"age": "31"}}` sẽ đổi hẳn field `age` từ số sang string. Cả hai driver mới đều tự suy luận kiểu JSON từ text gõ vào thay vì luôn bọc string:

- **Mongo** (`mongo_value_literal`, `crates/tradar-connector-mongo/src/lib.rs`): `ObjectId("...")`/`ISODate("...")` (đúng định dạng `simplify_extjson` đã hiện sẵn, nên giá trị `_id` hiển thị round-trip thẳng vào filter không cần parse ngược) giữ nguyên; chuỗi parse được JSON (số/bool/`null`/mảng/object gõ tay) giữ nguyên; còn lại bọc JSON string. **Giới hạn đã biết**: một kiểu BSON không có literal text nào (`Decimal128`, `Binary`, ...) sẽ bị gửi nhầm thành string thường — chấp nhận được vì suy luận từ text gõ tay vốn không thể phân biệt hết, đúng như phương án user chọn.
- **Elasticsearch** (`es_infer_value`): đơn giản hơn Mongo vì ES không có cú pháp constructor-call kiểu `ObjectId(...)` — chuỗi parse được JSON giữ nguyên, còn lại bọc JSON string.

## Elasticsearch: phát hiện giữa chừng — `_search` chưa từng tách hit thành row

Trước khi code phần ES, rà lại `execute()` phát hiện: **mọi response ES luôn bọc thành đúng 1 `QueryResult::Documents` entry** — kể cả `_search`, cả response (`{hits: {hits: [...]}, took, ...}`) là một "document" duy nhất, không tách theo từng hit. Tức là chưa từng có khái niệm "1 row = 1 ES document" trong app — quyết định lúc chốt phạm vi ("chỉ `_search` 1 index mới editable") dựa trên giả định sai. Quay lại hỏi user bằng `AskUserQuestion` thứ hai, chốt: sửa `execute()` để tách `hits.hits` thành nhiều `Documents` (ảnh hưởng cả cách browse `_search` bình thường, không chỉ edit) thay vì thu hẹp xuống chỉ `GET index/_doc/<id>` hay bỏ ES.

`unwrap_search_hits` (`crates/tradar-connector-elasticsearch/src/lib.rs`): nhận diện response có `hits.hits` (kể cả mảng rỗng — 0 hit vẫn là dạng `_search`, chỉ là 0 row); mỗi hit gộp `_id` + field của `_source` thành 1 document phẳng, giống cách Mongo document đã có sẵn `_id`. `None` cho mọi response khác (`_count`, `_cat`, `_cluster/health`, lỗi, `GET .../_doc/<id>`) — giữ nguyên hành vi cũ, cả response vẫn là 1 document, không có gì để tách. Không bao giờ bỏ một hit dù thiếu `_source` (`"_source": false` trong query) — hit đó chỉ còn `{"_id": ...}` chứ không biến mất.

## `_id` không nằm trong schema của Elasticsearch

Khác Mongo (`list_schema` tự đánh dấu `_id` là `primary_key: true`, vì `_id` là field thật trong document mẫu), `_id` của ES **không bao giờ** là field trong mapping — nó là metadata, không thuộc `_source`. `editable_row()` (`query_screen.rs`) trước giờ luôn lấy cột khoá từ `SchemaInfo.columns.primary_key`, nên nếu thêm `_id` giả vào `SchemaInfo` của ES sẽ kéo theo rủi ro thật: `crud_snippet`'s Create/Update (dùng chung `all_fields` từ `entry.columns`) sẽ vô tình chèn `"_id": <value>` vào body `POST index/_doc`/`_update`, sai cú pháp ES (id luôn nằm trên URL path, không phải body field) — phải sửa thêm logic lọc `_id` ra khỏi Create/Update giống Mongo's `non_id`, phạm vi lan rộng hơn cần thiết.

Chọn hướng khác: thêm `QueryDriver::edit_key_columns(&self, source: &str) -> Option<Vec<String>>` (mặc định `None`) — driver tự nói cột khoá là gì thay vì bắt buộc phải khai trong schema. `None` (mặc định, Postgres/SQLite/Mongo) → `editable_row()` fallback về tra `SchemaInfo.primary_key` như cũ, không đổi hành vi. ES override trả `Some(vec!["_id"])` thẳng, không đụng gì tới `list_schema`/`crud_snippet`. `QueryEngine::edit_key_columns` thêm delegate mỏng, cùng kiểu `edit_source`/`edit_sql` đã có.

## Field lồng nhau (dotted path)

Cả hai driver flatten field lồng nhau thành tên có dấu chấm (`address.city`/`customer.name` — `flatten_document`/`index_fields`), và cả `documents_as_table` lẫn navigator column picker đều dùng đúng tên đó. Khi sửa cell của field lồng:

- **Mongo**: `$set` của MongoDB tự hiểu dotted path làm path lồng — không cần xử lý thêm, `mongo_field_key` chỉ lo quote tên field không phải bare identifier.
- **Elasticsearch**: `_update`'s `doc` merge **không** tự hiểu dấu chấm là path — `{"doc": {"customer.name": v}}` sẽ tạo nhầm 1 field phẳng tên literal `"customer.name"`, không chạm field lồng thật. `nest_dotted_path` dựng lại object lồng thật (`{"customer": {"name": v}}`) từ tên cột phẳng trước khi đưa vào `doc`.

## Chỉ sửa được khi results đang ở table view

`ResultsComponent::columns()`/`selected_row()` chỉ trả dữ liệu thật cho `QueryResult::Table` và cho `Documents` **đang ở table view** (phím `t`) — view JSON thô của Documents trả rỗng, đúng như đã vậy từ trước khi có row-edit. Không cần sửa gì thêm: Mongo/ES row-edit tự nhiên chỉ khả dụng ở table view, giống hệt mọi tính năng dựa trên `columns()` khác (sort, cell cursor theo cột) — không phải giới hạn mới riêng cho row-edit.

## Test

- `crates/tradar-connector-mongo/src/lib.rs`: unit test cho `edit_source` (chấp nhận `find`, từ chối `aggregate`/write/`use`/`show dbs`), `edit_sql` (suy luận số/bool/null/ObjectId, quote string thường, field lồng, delete) — không cần Docker. Tích hợp thật (`edit_sql_round_trips_a_real_update_and_delete_against_a_found_document`, cần Docker): insert 1 document, `find` lấy `_id` thật hiển thị dạng `ObjectId(...)`, build/chạy `updateOne` rồi `deleteOne` qua đúng API driver, xác nhận cả hai đổi dữ liệu thật trong Mongo.
- `crates/tradar-connector-elasticsearch/src/lib.rs`: unit test cho `unwrap_search_hits` (gộp `_id` vào từng hit, `None` khi không phải dạng `_search`, không bỏ hit thiếu `_source`), `edit_source` (chỉ 1 index, từ chối multi/wildcard/`_all`/không phải `_search`), `edit_sql` (suy luận kiểu, field lồng dựng object thật) — không cần Docker. Tích hợp thật (`execute_unwraps_a_search_into_one_document_per_hit`, `edit_sql_round_trips_a_real_update_and_delete_against_a_search_hit`, cần Docker): index 2 document thật, `_search` trả đúng 2 row có `_id`; build/chạy `_update`/`DELETE _doc` qua đúng API driver rồi xác nhận bằng `GET .../_doc/<id>` trực tiếp (không qua `_search`, vì `_search` cần refresh mới thấy thay đổi ngay).
- `crates/tradar-query-workbench/src/components/query_screen.rs`: sửa lại thông báo lỗi khi kết quả không editable (bỏ chữ "SELECT"/"single table" SQL-riêng, đổi thành trung lập cho cả 3 driver) — cập nhật test `a_result_that_is_not_one_table_refuses_the_edit_and_says_why` theo chữ mới.
- **Chưa verify tay qua tmux**: môi trường sandbox lúc code không có Docker daemon (xem `docs/next-run.md`), nên không dựng được Mongo/ES thật để thao tác UI trực tiếp — độ tin cậy dựa vào unit test + integration test qua testcontainers (sẽ chạy khi có Docker, `make test-docker`), không phải thao tác tay như các sub-project trước.

README.md cập nhật đoạn mô tả row-edit (bỏ giới hạn "chỉ Postgres/SQLite có khoá chính", liệt kê thêm Mongo/ES và giới hạn riêng từng driver).
