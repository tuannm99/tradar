# Roadmap

Việc **đang mở/chưa scope xong** sống ở đây — ngắn, dễ quét, không lẫn vào lịch sử. Việc **đã xong** nằm trong `docs/backlog/` (một file mỗi sub-project, tách ra từ `docs/backlog.md` cũ khi file đó dài quá 450 dòng — xem `docs/backlog/README.md` để có mục lục). Thiết kế hệ thống nằm ở `docs/architecture.md`. Cập nhật file này mỗi khi một mục ở đây bắt đầu/kết thúc hoặc có mục mới, đúng tinh thần "roadmap tracks everything" đã theo xuyên suốt dự án.

## Connector mới, đã lên kế hoạch nhưng chưa code

- **MySQL / MariaDB / ClickHouse.** `README.md` liệt ở mục "Dự kiến". Rẻ nhờ kiến trúc pluggable: thêm crate mới + 1 dòng trong `registry()`, không đụng core. MySQL qua `sqlx` gần như giống hệt connector Postgres đang có.
- **Kafka: mode Groups (lag theo consumer group).** Hoãn khỏi v1 (`docs/backlog/mockup-ui-2026-08-15.md` mục 5, Topics mode + publish đã xong 2026-08-16) — mockup Screen 7 có phần consumer group nhưng lấy lag đúng nghĩa (current offset của 1 group cụ thể theo từng partition, so với high-water mark) với `rdkafka` cần dựng 1 consumer tạm gán `group.id` được chọn rồi gọi `committed()`, phức tạp hơn đáng kể so với phần Topics đã làm. Đọc-only, không seek/reset offset (giữ nguyên non-goal đã ghi trong `docs/architecture.md`).
- **gRPC, Socket** (cùng đợt yêu cầu 2026-08-16 với HTTP — HTTP đã xong, xem `docs/backlog/http-connector.md`). Thiết kế trong `docs/architecture.md` (mục "Thiết kế UI: HTTP, gRPC, Socket") chưa đổi, chưa code dòng nào. gRPC vẫn cần user xác nhận cắt phạm vi v1 xuống unary + server-streaming (bỏ client-streaming/bidi) trước khi bắt tay — connector rủi ro cao nhất trong ba cái, nên spike/prototype phần reflection + `prost-reflect::DynamicMessage` trước khi cam kết chi tiết UI. Socket đơn giản hơn, thiết kế đã đủ rõ để code thẳng khi tới lượt. Gợi ý thứ tự kỹ thuật: Socket trước (đơn giản nhất) → gRPC sau (rủi ro cao nhất, giờ đã có kinh nghiệm build UI phi-query mới từ HTTP — kể cả bài học "kiểm tra Cargo.toml của connector tương tự trước khi tin plan tái dùng crate nào").

## Gap nhỏ, chưa được scope

- Nhiều theme preset dựng sẵn (hiện: một theme dark + override từng màu).
- Cho phép remap cả phím vim *bên trong* editor (hiện cố định theo vim chuẩn — xem ghi chú phạm vi ở đầu `crates/tradar-core/src/keymap.rs`).
- Cột trong bảng kết quả resize được bằng tay (hiện tự tính theo giá trị rộng nhất, cap 40 ký tự).
- **Visual mode search-as-motion** trong query editor — thật vim hỗ trợ `/pattern` làm motion trong Visual mode, editor tự viết ở đây cố tình chưa làm (`open_buffer_search` chỉ hoạt động ở Normal mode).
- **`:s/pat/repl/` (replace) trong query editor** — phần còn thiếu của "Visual mode, copy/paste nội bộ, search trong buffer" (`docs/backlog/features-batch-2026-08-14.md`), tách ra làm sau vì chưa có tiền lệ UI dạng dòng lệnh `:` nào trong app, cần scope riêng.
- **`Component: Send`** — xem "Đã cân nhắc và gác lại" trong `docs/architecture.md`; lý do gốc (`edtui` giữ `Rc`) đã biến mất nhưng chưa ai verify lại.

## So sánh DataGrip/DBeaver/Studio3T — gap còn lại (user hỏi 2026-08-19)

Rà lại toàn bộ tính năng hiện có so với 3 IDE database tham chiếu, liệt kê gap theo nhóm (trả lời trong chat, không phải file). User chọn phần **Schema & DDL** và phần đầu **Query & editor**/**Data grid** để đưa vào roadmap và lên plan. **Chưa scope chi tiết bất kỳ mục nào ngoài #9** — mỗi mục dưới đây là một sub-project riêng, cỡ tương đương một trong các mục đã làm trước đó (Kafka, HTTP, row-edit...), cần `AskUserQuestion` riêng trước khi code, theo đúng pattern đã dùng xuyên suốt `docs/backlog/`.

**Schema & DDL**

1. ~~Navigator thêm cấp schema/database + nhóm theo loại object~~ — xong (2026-08-19) cho Postgres/Cassandra/MongoDB, xem `docs/backlog/navigator-schema-level.md`. **Indexes/Triggers cố tình bỏ khỏi scope** (quyết định lúc code, không phải lúc chốt phạm vi ban đầu) — DataGrip/DBeaver thật tự đặt chúng làm con của từng bảng chứ không phải folder ngang hàng Tables/Views ở cấp schema; đúng chỗ của chúng là #2 Table designer bên dưới, khi có UI xem chi tiết một bảng.
2. **Table designer qua UI** — thêm/sửa/xoá cột, constraint, index, FK; tạo bảng mới; đổi tên bảng. Rủi ro cao nhất trong 5 mục Schema & DDL: sinh DDL đúng cú pháp cho từng dialect (`ALTER TABLE` Postgres khác SQLite khác Cassandra), UI cho một thao tác vốn cần nhiều field (kiểu, null-able, default, cascade...) trong khi app hiện chỉ có `TextInput`/`TextArea` một dòng/nhiều dòng chưa có form nhiều field phức tạp kiểu này (form connection 3 field là ví dụ gần nhất, nhưng đơn giản hơn nhiều). Có thể cân nhắc hướng rẻ hơn: sinh DDL rồi cho user review/sửa tay trước khi chạy (giống cách row-edit làm với UPDATE/DELETE) thay vì UI point-and-click đầy đủ ngay từ v1.
3. **Schema diff/compare** — so 2 connection hoặc 2 schema, liệt kê khác biệt (cột thiếu/thừa, kiểu khác, index khác). Cần `list_schema` đủ chi tiết ở cả 2 phía (đã có cho Postgres/SQLite qua PK, nhưng chưa có index/constraint/default) trước khi so được gì có ý nghĩa.
4. **Migration/version-control tích hợp** — chưa rõ hình dạng: file migration kiểu Flyway/Alembic đọc từ thư mục, hay chỉ là "lưu lịch sử DDL đã chạy" đơn giản hơn? Cần hỏi lại phạm vi trước khi nghĩ kiến trúc.
5. ~~ERD (sơ đồ quan hệ bảng)~~ — xong (2026-08-20), xem `docs/backlog/fk-autocomplete-and-erd.md`. Box-drawing thật (đường nối `─│┌┐└┘├┤┬┴┼`), phím `F4`, phạm vi lân cận 1 bảng (không phải toàn schema). Dùng chung dữ liệu FK mới với #6.

**Query & editor**

6. ~~Autocomplete theo ngữ cảnh sâu~~ — xong (2026-08-20), xem `docs/backlog/fk-autocomplete-and-erd.md`. `.` sau alias gợi ý đúng cột của bảng đó, gõ sau `JOIN` xếp bảng có FK liên quan lên đầu. Dữ liệu FK mới (`ColumnInfo.foreign_key`) chỉ có cho Postgres/SQLite (Cassandra không có khái niệm FK trong CQL).
7. **"Generate SQL" từ UI** — tạo câu SELECT/INSERT/UPDATE tự động từ việc chọn bảng/cột qua giao diện, không gõ tay. Cần làm rõ khác gì với CRUD snippet đã có (navigator `c`/`r`/`u`/`d` sinh khung Create/Read/Update/Delete với placeholder `<tên_cột>`) — nếu ý là "point-and-click chọn cột thay vì gõ tay điền placeholder", đây là mở rộng của CRUD snippet chứ không phải tính năng mới từ đầu; nếu ý là "query builder trực quan" (chọn bảng, kéo điều kiện WHERE, chọn JOIN) thì là một sub-project lớn riêng, gần với #2 về độ phức tạp UI.

**Data grid**

8. ~~Import CSV/Excel/JSON vào bảng qua UI trong TUI grid~~ — **bỏ, đổi hướng sang CLI (2026-08-19)**, xem mục "`tradar` CLI: import/export" ngay bên dưới thay vì làm trong `Data grid` này.
9. **Sort theo cột (click header)** — xong (2026-08-19), xem `docs/backlog/sort-by-column.md`.
10. **Multi-filter kết hợp** — hiện chỉ 1 ô filter text đơn khớp bất kỳ cột nào (`/`). Cần nghĩ UI cho nhiều điều kiện cùng lúc (theo cột cụ thể, AND/OR) mà không đụng vỡ ý nghĩa của filter đơn hiện có.
11. **Group-by trong grid** — client-side, cần nghĩ trước cả UI (group theo cột nào, hiện aggregate gì) lẫn có đáng làm trong một results grid vốn thiết kế cho xem/sửa row-by-row hay không (khác hẳn mục đích của group-by).
12. **Mở rộng edit-cell/delete-row ngoài single-table-with-PK** — **rủi ro cao nhất trong toàn bộ danh sách**, cân nhắc kỹ trước khi nhận làm: `single_table_source`/`build_sql_edit` cố tình bảo thủ (từ chối join/view/no-PK) đúng vì đoán sai bảng nguồn nghĩa là sinh `UPDATE`/`DELETE` nhắm sai chỗ — hậu quả là mất/sửa nhầm dữ liệu thật, không phải một tính năng thiếu vô hại. Nếu làm, cần một cơ chế xác định nguồn đáng tin hơn heuristic hiện tại (có thể là hỏi DB trực tiếp qua `EXPLAIN`/system catalog thay vì tự parse SQL), và có lẽ vẫn nên giữ nguyên tắc "từ chối rồi nói rõ lý do" cho các trường hợp còn mơ hồ thay vì cố đoán bừa.

**Thứ tự đã chốt (2026-08-19)**, user duyệt đề xuất theo rủi ro/phụ thuộc kỹ thuật, không tự chọn thứ tự khác. #8 đổi hướng sang CLI (xem mục riêng bên dưới) nên rút khỏi Tier 1:

- **Tier 1 (làm trước, rẻ/độc lập)**: #9 Sort theo cột — xong, `docs/backlog/sort-by-column.md`.
- **Tier 2 (nền tảng)**: #1 Navigator schema/database + nhóm object — xong, `docs/backlog/navigator-schema-level.md`.
- **Tier 3 (dùng chung dữ liệu FK vừa thêm ở #1)**: #6 Autocomplete ngữ cảnh sâu — xong, #5 ERD — xong, cả hai `docs/backlog/fk-autocomplete-and-erd.md`.
- **Tier 4 (cần chốt phạm vi trước khi code, làm tiếp theo)**: #7 Generate SQL từ UI → #10 Multi-filter kết hợp.
- **Tier 5 (lớn, tách nhiều bước nhỏ)**: #2 Table designer → #3 Schema diff/compare → #4 Migration/version-control.
- **Tier 6 (để cuối, #12 cần bàn lại có đáng làm không)**: #11 Group-by trong grid → #12 Mở rộng edit-cell/delete-row ngoài single-table-with-PK.

## `tradar` CLI: import/export (ý tưởng mới, 2026-08-19) — tier thấp, để sau

User đề xuất thay vì làm import CSV/Excel/JSON qua UI trong TUI grid (#8 cũ ở trên, đã bỏ), đổi thành một chế độ **CLI** của cùng binary `tradar` — kiểu port `mongoimport`/`mongoexport`, `psql`'s `\copy`/`COPY`, `cqlsh`'s `COPY`, hay Elasticsearch bulk API qua `curl`, tuỳ connector.

**Cố tình để tier thấp, chưa scope** (quyết định cùng lúc, 2026-08-19): gần như mỗi connector đã có tool OSS hoặc built-in riêng của nhà cung cấp làm đúng việc này rồi (`mongoimport`/`mongoexport`, `psql \copy`/`pg_dump`, `cqlsh COPY`, `elasticdump`...) — giá trị thật của việc `tradar` tự làm lại là gì (không phải gọi lại đúng những tool đó cho tiện, mà là port/thay thế) chỉ rõ ràng khi thật sự cần, không phải bây giờ. Để lại đây làm ghi chú, quay lại chốt phạm vi khi có lý do cụ thể (một connector nào thiếu tool tốt, hoặc user cần một chỗ duy nhất không phải nhớ N tool khác nhau) thay vì chốt trước cho một nhu cầu chưa xác nhận.

Khi quay lại, những điểm cần làm rõ trước khi lên plan (chưa trả lời):

- Subcommand của `tradar` hiện có (`clap` đã nằm sẵn trong `tradar-app/Cargo.toml` nhưng chưa dùng dòng nào — đây sẽ là lần dùng đầu tiên), hay một binary riêng? Chạy `tradar` không kèm subcommand vẫn phải mở TUI như hiện tại, không được đổi hành vi mặc định.
- Tái dùng connection đã lưu trong `~/.config/tradar/connections.toml` (qua tên) thay vì phải gõ lại connection string, giống cách TUI đang làm — cần connector nào cũng đi qua `Connector`/`QueryDriver` sẵn có, không viết logic kết nối riêng cho CLI.
- Mỗi connector có "ngôn ngữ" import/export khác hẳn nhau (Postgres/SQLite: `COPY`/`INSERT` hàng loạt; Mongo: `bson`/`json` theo document, không có schema cột cố định; Elasticsearch: bulk API theo dòng `_index`/`_source`; Cassandra: `COPY` của `cqlsh`; Redis không có khái niệm "bảng" nên có lẽ không áp dụng) — vẫn là đúng vấn đề mapping cột/field mà #8 gặp phải, chỉ chuyển từ UI form sang CLI flag, không tự nhiên biến mất.
- File lớn: đọc/ghi streaming thay vì load hết vào RAM (khác cách `export.rs` hiện làm, vốn nhận `QueryResult` đã có sẵn trong bộ nhớ) — ảnh hưởng tới `QueryDriver` có cần thêm method streaming hay tái dùng nguyên trạng.
- Có đáng làm cả `export` CLI không, hay chỉ `import` (TUI đã có `Ctrl+E` export CSV/JSON cho kết quả đang xem, dù chỉ theo từng query/kết quả một, không phải "dump nguyên bảng" như `mongoexport`/`pg_dump`).

