# Generate SQL từ UI — mở rộng CRUD snippet bằng column picker (xong, 2026-08-23)

Gap #7 trong đợt so sánh DataGrip/DBeaver/Studio3T (`docs/roadmap.md`), Tier 4. Chốt phạm vi qua `AskUserQuestion` trước khi code: **mở rộng CRUD snippet đã có** (point-and-click chọn cột thay vì gõ tay điền placeholder `<column>`), không phải một query builder trực quan riêng (chọn bảng/JOIN/WHERE) — cái đó vẫn để dành, gần độ phức tạp #2 Table designer.

## Cơ chế chung

`c`/`r`/`u`/`d` trên navigator giờ **luôn** mở một overlay chọn cột (checkbox multi-select) trước khi insert, thay vì insert ngay tất cả cột như trước — quyết định chốt qua `AskUserQuestion`, đổi hẳn hành vi cũ (không thêm phím riêng). Nếu bảng không có cột nào để chọn (Redis: `SchemaInfo::columns` luôn rỗng) thì bỏ qua picker, giữ nguyên insert-ngay một phím như cũ.

- Component mới `crates/tradar-app/src/components/column_picker.rs` (`ColumnPickerComponent`), mô phỏng theo `SnippetPickerComponent` (`tradar-query-workbench/src/components/snippet_picker.rs`) — list + cursor, nhưng thêm state toggle-theo-dòng (`HashSet<usize>` các index đã check, cùng idiom `NavigatorComponent` đã dùng cho `expanded`/`open`).
- Sở hữu bởi `NavigatorComponent` qua field `pending_snippet: Option<PendingSnippet>` (`PendingSnippet { tab, name, op, picker }`) — chiếm input trước filter bar, cùng idiom "modal steals input" như `filter_input`/`is_filtering()`.
- Dữ liệu cột lấy thẳng từ `NavConnection::outline` đã có sẵn (không thêm trait method fetch riêng) — `choose_snippet` duyệt các `OutlineEntry` con ngay sau bảng (`depth == table_depth + 1`, dừng khi gặp entry `depth <= table_depth`).
- `OutlineEntry` thêm field `primary_key: bool` (`tradar-core/src/action.rs`) — type-safe, thay vì parse lại chuỗi `detail` (`"{type} pk"`, vẫn giữ nguyên cho hiển thị, giờ là "hai góc nhìn của cùng một fact", idiom đã có tiền lệ với `kind`/`detail`). `push_table` trong `query_screen.rs`'s `flatten_outline` set field này trực tiếp từ `ColumnInfo::primary_key`.
- `Context::ColumnPicker` mới trong `keymap.rs` (`enter`→Confirm, `esc`→Cancel, `space`→`Command::ToggleColumn` mới, `a`→`Command::ToggleAllColumns` mới) — tự động lên `?` help overlay qua `context_title()`.
- `NavOutcome::Snippet` thêm field `columns: Vec<String>`.
- Đổi signature xuyên suốt (rỗng = "dùng default của op", giữ nguyên hành vi cũ cho mọi caller có sẵn): `Component::crud_snippet` (`action.rs`) → `QueryScreenComponent::crud_snippet` → `QueryEngine::crud_snippet` → `QueryDriver::crud_snippet` (trait, `query_driver.rs`) → 6 connector impl.

## Từng CRUD op — Postgres/SQLite/Cassandra (dùng chung `build_crud_snippet`)

Hàm `pick_columns(columns, candidates, default)` (mới, `pub fn` trong `query_driver.rs` — free function chứ không phải closure, vì closure vừa capture `columns` vừa nhận 2 slice khác lifetime nguồn không tự suy lifetime được, cần chữ ký `fn` có lifetime tường minh) dùng chung cho cả `build_crud_snippet` lẫn Mongo/Elasticsearch (xem dưới).

- **Read**: mặc định không chọn gì → `SELECT *` (giữ nguyên); chọn cột → liệt kê đúng cột đã chọn, đúng thứ tự cột gốc trong bảng (không theo thứ tự click). Đây là op duy nhất mà "chọn rỗng" là kết quả hợp lệ, không phải "dùng default".
- **Create**: mặc định check tất cả cột (giống hệt hành vi cũ khi Enter ngay, không đổi gì).
- **Update**: mặc định check cột non-PK (giống cũ, fallback về tất cả cột nếu bảng không có PK — giữ nguyên fallback cũ); có thể chủ động check cả cột PK nếu muốn. **WHERE vẫn PK-only, không đổi** — ngoài phạm vi đã chốt.
- **Delete**: mặc định check PK; **mở rộng cho phép chọn thêm cột khác cho WHERE** (đúng yêu cầu "mở rộng WHERE" từ user) — nhưng **bắt buộc ≥1 cột được chọn**, không thể tạo `DELETE` thiếu WHERE qua UI này (safety property tường minh, không phải nice-to-have).

## Mongo/Elasticsearch — "làm thật" thay vì bỏ qua

Phát hiện lúc research: Mongo/Elasticsearch **có** populate `entry.columns` (Mongo sample field từ 1 document, ES đọc mapping) nhưng `crud_snippet` cũ của cả hai **chưa từng dùng field nào** — luôn sinh body rỗng (`insertOne({})`, `POST index/_doc\n{\n}`...). Mở picker cho 2 driver này mà không đổi gì thì thành "chọn cột xong không có tác dụng" — hỏi lại user, chốt **làm thật** thay vì bỏ qua picker cho cả hai (Redis vẫn tự động bỏ qua picker vì `columns` luôn rỗng, không cần quyết định riêng).

- **Mongo Read**: chọn field → thêm projection `find({}, {field: 1})`; rỗng → giữ `find({})`.
- **Mongo Create**: mặc định check tất cả field (kể cả `_id`) → `insertOne({field: <value>, ...})`. **Thay đổi hành vi có chủ đích** so với `insertOne({})` cũ — đúng lỗ hổng mà "làm thật" muốn vá, không phải regression.
- **Mongo Update**: `$set` theo field đã chọn (mặc định = non-`_id`, fallback tất cả nếu không có field nào khác `_id`); filter vẫn `{_id: ObjectId("<id>")}`, không đổi.
- **Mongo Delete**: filter theo field đã chọn, mặc định = `_id` → Enter ngay ra đúng `deleteOne({_id: ObjectId("<id>")})` y hệt cũ (một trong số ít trường hợp default mới trùng khít default cũ). Cùng luật an toàn ≥1 field như SQL Delete.
- Field name không phải bare JS identifier (dotted path như `address.city`, từ document lồng — xem `flatten_document`) được quote (`mongo_field_key`) — bug thật sẽ gặp nếu bỏ qua, vì `db.x.find(...)` được `mongosh` parse thật, không chỉ hiển thị.
- **Elasticsearch Read**: chọn field → thêm `"_source": [...]`; rỗng → giữ nguyên `match_all` cũ.
- **Elasticsearch Create/Update**: body `{"field": <value>, ...}` theo field đã chọn (mặc định = tất cả field, vì ES không có khái niệm PK cho field mapping nên "non-PK, fallback tất cả" tự nhiên suy biến thành "tất cả"). Cùng kiểu "thay đổi có chủ đích" như Mongo Create.
- **Elasticsearch Delete: giữ nguyên `DELETE index/_doc/<id>`, không qua picker, `columns` bị bỏ qua.** Hỏi lại user riêng cho ca này: mở rộng đúng tinh thần SQL sẽ cần đổi hẳn sang API `_delete_by_query` (POST, xoá hàng loạt theo điều kiện match) — verb/endpoint khác hẳn, rủi ro xoá nhầm hàng loạt đúng loại mà roadmap #12 đã cảnh báo là nguy hiểm nhất trong toàn danh sách. **Chốt: không làm**, giữ ES Delete đơn giản/an toàn như cũ.

## Test

- `query_driver.rs`: toàn bộ test cũ thêm `&[]` (giữ nguyên assertion, xác nhận backward-compat), cộng test mới cho từng op với selection tường minh + case "selection không khớp cột thật nào → fallback về default" (Read/Delete).
- 6 connector crate: mỗi crate 1-2 test forward-selection qua đúng driver thật (không lặp lại logic `build_crud_snippet` đã test ở `query_driver.rs`); Mongo/Elasticsearch có bộ test riêng đầy đủ cho field thật + `mongo_field_key` quote dotted path.
- `column_picker.rs`: default-checked đúng theo từng op, toggle/toggle-all, **case an toàn "Delete rỗng → không cho confirm"** (test riêng, safety-critical), thứ tự selection luôn theo thứ tự bảng gốc dù toggle không theo thứ tự.
- `navigator.rs`: `choose_snippet` mở picker thay vì insert ngay khi có cột; bảng không cột vẫn insert ngay một bước (regression guard); Esc huỷ không insert gì.
- `mod.rs` (`RootComponent`): test cũ (`pressing_r_on_a_table_...`, `pressing_c_on_a_connection_row_...`) không đổi gì (fixture `OutlineScreen` có 0 cột, tự động bỏ qua picker).

Không đụng `SchemaInfo`/`ColumnInfo`/`list_schema` của bất kỳ driver nào — dữ liệu cột cần thiết đã có sẵn từ trước.
