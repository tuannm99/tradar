# Roadmap

Việc **đang mở/chưa scope xong** sống ở đây — ngắn, dễ quét, không lẫn vào lịch sử. Việc **đã xong** nằm trong `docs/backlog/` (một file mỗi sub-project, tách ra từ `docs/backlog.md` cũ khi file đó dài quá 450 dòng — xem `docs/backlog/README.md` để có mục lục). Thiết kế hệ thống nằm ở `docs/architecture.md`. Cập nhật file này mỗi khi một mục ở đây bắt đầu/kết thúc hoặc có mục mới, đúng tinh thần "roadmap tracks everything" đã theo xuyên suốt dự án.

## Checklist tổng quan

Nhìn nhanh trạng thái — chi tiết/lý do đầy đủ vẫn nằm ở các mục văn xuôi bên dưới, checklist này chỉ để quét nhanh, **không thay thế**. Tick khi một mục chuyển sang "xong" ở phần chi tiết.

**So sánh DataGrip/DBeaver/Studio3T — thứ tự Tier**

- [x] Tier 1 — #9 Sort theo cột
- [x] Tier 2 — #1 Navigator schema/database + nhóm object
- [x] Tier 3 — #6 Autocomplete ngữ cảnh sâu
- [x] Tier 3 — #5 ERD
- [x] Tier 4 — #7 Generate SQL từ UI (column picker)
- [x] Tier 4 — #10 Multi-filter kết hợp
- [x] Tier 5 — #3 Schema diff/compare
- [x] Tier 5 — #2 Table designer
- [x] Tier 5 — #4 Migration/version-control
- [ ] Tier 6 — #11 Group-by trong grid — bỏ qua 2026-09-23
- [x] Tier 6 — #12 Mở rộng edit-cell/delete-row ngoài single-table-with-PK

**Connector mới**

- [x] MySQL / MariaDB — xong (2026-10-01), xem `docs/backlog/mysql-connector-2026-10-01.md`
- [x] ClickHouse — xong (2026-09-29), xem `docs/backlog/clickhouse-connector-2026-09-29.md`
- [x] Kafka: mode Groups (lag theo consumer group) — xong (2026-10-01), xem `docs/backlog/kafka-groups-mode-2026-10-01.md`
- [x] Socket — xong (2026-09-29), xem `docs/backlog/socket-connector-2026-09-29.md`
- [ ] gRPC (cần chốt phạm vi v1 trước)

**`tradar` CLI: import/export** — chưa scope, tier thấp

- [ ] Chốt các điểm mở (subcommand vs binary riêng, streaming, import vs export...)

**Gap nhỏ, chưa scope**

- [x] Nhiều theme preset dựng sẵn — xong (2026-09-23), xem `docs/backlog/theme-presets.md`
- [x] Remap phím vim bên trong editor — xong (2026-09-24), xem `docs/backlog/vim-remap-2026-09-24.md`
- [x] Resize cột bằng tay trong results grid — xong (2026-09-24), xem `docs/backlog/column-resize-2026-09-24.md`
- [x] Visual mode search-as-motion trong query editor — xong (2026-09-24), xem `docs/backlog/visual-mode-search-motion-2026-09-24.md`
- [x] `:s/pat/repl/` trong query editor — xong (2026-09-24), xem `docs/backlog/command-line-substitute-2026-09-24.md`
- [x] `Component: Send` — re-verify lý do gốc còn đúng không (xong 2026-09-03, kết luận: giữ nguyên)

## Connector mới, đã lên kế hoạch nhưng chưa code

- ~~**MySQL / MariaDB**~~ — xong (2026-10-01), xem `docs/backlog/mysql-connector-2026-10-01.md`. Một driver cho cả hai (cùng wire protocol qua `sqlx`'s `mysql` feature), gần giống hệt connector Postgres đang có — khác ở cách đọc `information_schema` (đơn giản hơn: MySQL's `column_key`/`key_column_usage.referenced_table_name` mang PK/FK trực tiếp, không cần join như Postgres) và cách suy ra vị trí lỗi cú pháp (không có vị trí từ wire protocol như Postgres, suy ra gần đúng từ token trong thông báo lỗi giống cách SQLite làm). Row-edit và transaction control (`BEGIN`/`COMMIT`/`ROLLBACK`) hoạt động như Postgres/SQLite; table designer và migration tracking **chưa làm** cho MySQL (cả hai vẫn chỉ Postgres).
- ~~**ClickHouse**~~ — xong (2026-09-29), xem `docs/backlog/clickhouse-connector-2026-09-29.md`. Không qua `sqlx` (không hỗ trợ ClickHouse) mà qua chính HTTP interface của ClickHouse (`reqwest` + `FORMAT JSON`, giống cách connector Elasticsearch đã làm) — vẫn là SQL thật nên dùng chung `SQL_KEYWORDS`/`split_sql_statements`/tree-sitter highlight với Postgres/SQLite. Row-edit (sửa cell/xoá dòng) **không** làm ở v1: ClickHouse không có `UPDATE`/`DELETE` chuẩn, chỉ có mutation `ALTER TABLE ... UPDATE/DELETE` chạy nền bất đồng bộ, không khớp trải nghiệm "y rồi thấy liền" hiện có.
- ~~**Kafka: mode Groups (lag theo consumer group)**~~ — xong (2026-10-01), xem `docs/backlog/kafka-groups-mode-2026-10-01.md`. `F2` chuyển Topics⇄Groups (giống RabbitMQ's Queues⇄Exchanges); sidebar liệt kê consumer group (tên/state/số member qua `fetch_group_list`), `Enter` tính lag từng partition qua một consumer tạm gán `group.id` được chọn (không subscribe/assign, không join group thật) rồi gọi `committed_offsets()` + `fetch_watermarks()` — đúng kỹ thuật đã ghi ở đây từ đầu. Đọc-only, không seek/reset offset (giữ nguyên non-goal đã ghi trong `docs/architecture.md`).
- ~~**Socket**~~ — xong (2026-09-29), xem `docs/backlog/socket-connector-2026-09-29.md`. Raw TCP netcat-style, `crates/tradar-connector-socket` — không sidebar, một panel transcript cuộn được + một dòng input luôn ở chế độ gõ (không có khái niệm focus rời khỏi nó), nên `j`/`k`/`gg`/`G` không dùng được để cuộn (sẽ bị gõ thành text) — thay bằng mũi tên/`ctrl-d`/`ctrl-u`; `r` reconnect cũng đổi thành `ctrl-r` cùng lý do. Không cần Docker để test (dựng thẳng `tokio::net::TcpListener` cục bộ trong test).
- **gRPC** (cùng đợt yêu cầu 2026-08-16 với HTTP/Socket — cả hai đã xong). Thiết kế trong `docs/architecture.md` (mục "Thiết kế UI: HTTP, gRPC, Socket") chưa đổi, chưa code dòng nào. Vẫn cần user xác nhận cắt phạm vi v1 xuống unary + server-streaming (bỏ client-streaming/bidi) trước khi bắt tay — connector rủi ro cao nhất trong ba cái (HTTP/Socket/gRPC), nên spike/prototype phần reflection + `prost-reflect::DynamicMessage` trước khi cam kết chi tiết UI.

## Gap nhỏ, chưa được scope

- ~~Nhiều theme preset dựng sẵn~~ — xong (2026-09-23), xem `docs/backlog/theme-presets.md`. Chốt qua `AskUserQuestion`: 2 preset **Dracula + Nord** (dark-only, không làm theme sáng/Gruvbox/Solarized), `preset` trong `[theme]` chọn bảng nền, override từng role vẫn đè lên trên như trước.
- ~~Cho phép remap cả phím vim *bên trong* editor~~ — xong (2026-09-24), xem `docs/backlog/vim-remap-2026-09-24.md`. Chốt qua `AskUserQuestion`: remap **toàn bộ** (kể cả `dd`/`yy`/`i`/`a`/`o`/...), không chỉ motion đơn giản — `QueryEditorComponent` giờ resolve qua `Context::VimNormal`/`VimVisual`/`VimMotion` mới trong `tradar_core::keymap`, y hệt mọi component khác.
- ~~Cột trong bảng kết quả resize được bằng tay~~ — xong (2026-09-24), xem `docs/backlog/column-resize-2026-09-24.md`. Chốt qua `AskUserQuestion`: phím tắt `<`/`>` trên cột đang chọn (không phải kéo chuột), reset khi có kết quả mới (không nhớ theo tên cột xuyên session).
- ~~**Visual mode search-as-motion** trong query editor~~ — xong (2026-09-24), xem `docs/backlog/visual-mode-search-motion-2026-09-24.md`. `find()` chưa từng đụng `visual_anchor` nên chỉ cần nới guard "chỉ Normal" thành "trừ Insert" ở `open_buffer_search`/`repeat_buffer_search`.
- ~~**`:s/pat/repl/` (replace) trong query editor**~~ — xong (2026-09-24), xem `docs/backlog/command-line-substitute-2026-09-24.md`. Chốt qua `AskUserQuestion`: theo đúng vim thật (không tiền tố = dòng hiện tại, `%s` = toàn buffer), pattern literal substring như `/` search (không phải regex, không thêm dependency).
- ~~**`Component: Send`**~~ — đã verify 2026-09-03, **kết luận: giữ nguyên, không bound**. Mọi implementor thật (`RootComponent`, `ConnectionPickerComponent`, `QueryScreenComponent`, `KafkaScreen`, `RabbitScreen`, `HttpScreen`) đã `Send` sẵn; chỉ 2 test double còn giữ `Rc`. Lý do gốc (`edtui` giữ `Rc`) đúng là đã biến mất — nhưng điều kiện kích hoạt mà `docs/architecture.md` đặt ra ("xem lại nếu channel `ConnectOutcome` từng trở thành điểm khó bảo trì thật") thì chưa xảy ra, nên bound vào lúc này là đổi kiến trúc cho một lợi ích chưa ai cần. Chi tiết + cách verify trong `docs/backlog/known-issues.md`.

## So sánh DataGrip/DBeaver/Studio3T — gap còn lại (user hỏi 2026-08-19)

Rà lại toàn bộ tính năng hiện có so với 3 IDE database tham chiếu, liệt kê gap theo nhóm (trả lời trong chat, không phải file). User chọn phần **Schema & DDL** và phần đầu **Query & editor**/**Data grid** để đưa vào roadmap và lên plan. **Chưa scope chi tiết bất kỳ mục nào ngoài #9** — mỗi mục dưới đây là một sub-project riêng, cỡ tương đương một trong các mục đã làm trước đó (Kafka, HTTP, row-edit...), cần `AskUserQuestion` riêng trước khi code, theo đúng pattern đã dùng xuyên suốt `docs/backlog/`.

**Schema & DDL**

1. ~~Navigator thêm cấp schema/database + nhóm theo loại object~~ — xong (2026-08-19) cho Postgres/Cassandra/MongoDB, xem `docs/backlog/navigator-schema-level.md`. **Indexes/Triggers cố tình bỏ khỏi scope** (quyết định lúc code, không phải lúc chốt phạm vi ban đầu) — DataGrip/DBeaver thật tự đặt chúng làm con của từng bảng chứ không phải folder ngang hàng Tables/Views ở cấp schema; đúng chỗ của chúng là #2 Table designer bên dưới, khi có UI xem chi tiết một bảng.
2. ~~Table designer qua UI~~ — xong (2026-09-23), xem `docs/backlog/table-designer.md`. Chốt qua `AskUserQuestion`: form nhiều field đầy đủ (point-and-click, user chọn thay vì hướng "sinh DDL rồi review tay" roadmap từng gợi ý là rẻ hơn), 4 thao tác Thêm/Xoá cột, Đổi tên bảng, Tạo bảng mới (Constraint/Index/FK để round sau), **chỉ Postgres** ở v1 — Mongo/Elasticsearch (ưu tiên trước đó) schemaless nên không áp dụng được, SQLite/Cassandra để sau. `a`/`x`/`R`/`n` trong navigator.
3. ~~Schema diff/compare~~ — xong (2026-09-23), xem `docs/backlog/schema-diff.md`. So 2 connection **đã mở** (giản lược so với phạm vi gốc: chưa tự connect hộ một connection chưa mở), dựa trên `Component::outline()` thay vì `SchemaInfo` trực tiếp (đúng luật phụ thuộc `tradar-core`/`tradar-query-workbench`) — nên diff chỉ so được đúng thứ `outline()` đã mang: tên cột, kiểu, PK, không có index/constraint/default/FK. `D` trong navigator, tab riêng, read-only, chưa sinh DDL đồng bộ.
4. ~~Migration/version-control tích hợp~~ — xong (2026-09-23), xem `docs/backlog/migrations.md`. Chốt qua `AskUserQuestion`: file migration kiểu Flyway/Alembic (`.sql` đánh số trong `~/.config/tradar/migrations/<connection>/`), track qua bảng `_tradar_migrations` trong chính DB, độc lập với table designer, chỉ Postgres. `F1` mở panel, chạy mọi file pending tuần tự (mỗi file 1 transaction riêng, lỗi thì dừng + rollback file đó, giữ nguyên các file trước đã commit).
5. ~~ERD (sơ đồ quan hệ bảng)~~ — xong (2026-08-20), xem `docs/backlog/fk-autocomplete-and-erd.md`. Box-drawing thật (đường nối `─│┌┐└┘├┤┬┴┼`), phím `F4`, phạm vi lân cận 1 bảng (không phải toàn schema). Dùng chung dữ liệu FK mới với #6.

**Query & editor**

6. ~~Autocomplete theo ngữ cảnh sâu~~ — xong (2026-08-20), xem `docs/backlog/fk-autocomplete-and-erd.md`. `.` sau alias gợi ý đúng cột của bảng đó, gõ sau `JOIN` xếp bảng có FK liên quan lên đầu. Dữ liệu FK mới (`ColumnInfo.foreign_key`) chỉ có cho Postgres/SQLite (Cassandra không có khái niệm FK trong CQL).
7. ~~"Generate SQL" từ UI~~ — xong (2026-08-23), xem `docs/backlog/crud-snippet-column-picker.md`. Chốt phạm vi là mở rộng CRUD snippet đã có (navigator `c`/`r`/`u`/`d` giờ mở column picker trước khi insert), không phải query builder trực quan riêng (chọn bảng/JOIN/WHERE qua UI) — cái đó vẫn để dành cho sau, gần độ phức tạp #2 Table designer nếu có nhu cầu cụ thể.

**Data grid**

8. ~~Import CSV/Excel/JSON vào bảng qua UI trong TUI grid~~ — **bỏ, đổi hướng sang CLI (2026-08-19)**, xem mục "`tradar` CLI: import/export" ngay bên dưới thay vì làm trong `Data grid` này.
9. **Sort theo cột (click header)** — xong (2026-08-19), xem `docs/backlog/sort-by-column.md`.
10. ~~Multi-filter kết hợp~~ — xong (2026-09-07), xem `docs/backlog/multi-filter.md`. Chốt qua `AskUserQuestion`: kết hợp cả hai hướng (mở rộng cú pháp ô filter `/` hiện có bằng `cột:giá_trị` + `AND`/`OR`, cộng panel `F3` xem/xoá từng điều kiện), hỗ trợ cả AND lẫn OR.
11. **Group-by trong grid** — **bỏ qua 2026-09-23**, qua `AskUserQuestion`: đồng ý với chính nghi vấn roadmap đặt ra — group-by không hợp mục đích một results grid vốn thiết kế cho xem/sửa row-by-row (edit-cell, delete-row, cell cursor h/l đều giả định "đang nhìn đúng dòng thật trong DB"; một hàng nhóm không còn là một dòng thật để trỏ vào). Không tự chốt cứng "sẽ không bao giờ làm" — chỉ là chưa có lý do cụ thể để ưu tiên trước #12, để lại đây làm ghi chú như CLI import/export.
12. ~~Mở rộng edit-cell/delete-row ngoài single-table-with-PK~~ — xong (2026-09-23), xem `docs/backlog/no-pk-row-edit.md`. **Rủi ro cao nhất trong toàn bộ danh sách**, nên hỏi lại phạm vi qua `AskUserQuestion` trước khi code: vẫn làm nhưng cẩn thận, và giới hạn đúng một trường hợp — bảng không có PK khai báo, fallback dùng toàn bộ cột kết quả làm khoá `WHERE`, luôn cảnh báo tĩnh trong overlay confirm. Kết quả JOIN **không** được mở rộng (bị loại rõ ràng ở vòng scope — "sửa dòng nào, ghi vào bảng nào" không còn rõ ràng, rủi ro cao hơn giá trị mang lại), `single_table_source` giữ nguyên không đổi.

**Thứ tự đã chốt (2026-08-19)**, user duyệt đề xuất theo rủi ro/phụ thuộc kỹ thuật, không tự chọn thứ tự khác. #8 đổi hướng sang CLI (xem mục riêng bên dưới) nên rút khỏi Tier 1:

- **Tier 1 (làm trước, rẻ/độc lập)**: #9 Sort theo cột — xong, `docs/backlog/sort-by-column.md`.
- **Tier 2 (nền tảng)**: #1 Navigator schema/database + nhóm object — xong, `docs/backlog/navigator-schema-level.md`.
- **Tier 3 (dùng chung dữ liệu FK vừa thêm ở #1)**: #6 Autocomplete ngữ cảnh sâu — xong, #5 ERD — xong, cả hai `docs/backlog/fk-autocomplete-and-erd.md`.
- **Tier 4 (cần chốt phạm vi trước khi code)**: #7 Generate SQL từ UI — xong, `docs/backlog/crud-snippet-column-picker.md`. #10 Multi-filter kết hợp — xong, `docs/backlog/multi-filter.md`.
- **Tier 5 (lớn, tách nhiều bước nhỏ)** — **cả 3 mục đã xong**: #3 Schema diff/compare — xong, `docs/backlog/schema-diff.md`. → #2 Table designer — xong, `docs/backlog/table-designer.md`. → #4 Migration/version-control — xong, `docs/backlog/migrations.md`. **Đảo thứ tự 2026-09-23** (ban đầu #2 → #3 → #4): user yêu cầu ưu tiên Postgres/Mongo/Elasticsearch trong 3 mục Tier 5 — #3 là mục duy nhất phục vụ được cả ba (schema info đã có sẵn cho cả ba: Postgres qua PK khai báo, Mongo qua suy luận từ `list_schema`, Elasticsearch qua mapping REST API), trong khi #2 Table designer sinh DDL (`ALTER TABLE`...) chỉ có ý nghĩa cho Postgres/SQLite/Cassandra — Mongo và Elasticsearch schemaless, không có khái niệm DDL tương đương. #4 vẫn xếp cuối vì chưa chốt phạm vi.
- **Tier 6 (để cuối, cả 2 mục đã xử lý xong)**: #11 Group-by trong grid — bỏ qua 2026-09-23 (xem lý do ở mục #11 phía trên). → #12 Mở rộng edit-cell/delete-row ngoài single-table-with-PK — xong 2026-09-23, `docs/backlog/no-pk-row-edit.md`.

## `tradar` CLI: import/export (ý tưởng mới, 2026-08-19) — tier thấp, để sau

User đề xuất thay vì làm import CSV/Excel/JSON qua UI trong TUI grid (#8 cũ ở trên, đã bỏ), đổi thành một chế độ **CLI** của cùng binary `tradar` — kiểu port `mongoimport`/`mongoexport`, `psql`'s `\copy`/`COPY`, `cqlsh`'s `COPY`, hay Elasticsearch bulk API qua `curl`, tuỳ connector.

**Cố tình để tier thấp, chưa scope** (quyết định cùng lúc, 2026-08-19): gần như mỗi connector đã có tool OSS hoặc built-in riêng của nhà cung cấp làm đúng việc này rồi (`mongoimport`/`mongoexport`, `psql \copy`/`pg_dump`, `cqlsh COPY`, `elasticdump`...) — giá trị thật của việc `tradar` tự làm lại là gì (không phải gọi lại đúng những tool đó cho tiện, mà là port/thay thế) chỉ rõ ràng khi thật sự cần, không phải bây giờ. Để lại đây làm ghi chú, quay lại chốt phạm vi khi có lý do cụ thể (một connector nào thiếu tool tốt, hoặc user cần một chỗ duy nhất không phải nhớ N tool khác nhau) thay vì chốt trước cho một nhu cầu chưa xác nhận.

Khi quay lại, những điểm cần làm rõ trước khi lên plan (chưa trả lời):

- Subcommand của `tradar` hiện có (`clap` đã nằm sẵn trong `tradar-app/Cargo.toml` nhưng chưa dùng dòng nào — đây sẽ là lần dùng đầu tiên), hay một binary riêng? Chạy `tradar` không kèm subcommand vẫn phải mở TUI như hiện tại, không được đổi hành vi mặc định.
- Tái dùng connection đã lưu trong `~/.config/tradar/connections.toml` (qua tên) thay vì phải gõ lại connection string, giống cách TUI đang làm — cần connector nào cũng đi qua `Connector`/`QueryDriver` sẵn có, không viết logic kết nối riêng cho CLI.
- Mỗi connector có "ngôn ngữ" import/export khác hẳn nhau (Postgres/SQLite: `COPY`/`INSERT` hàng loạt; Mongo: `bson`/`json` theo document, không có schema cột cố định; Elasticsearch: bulk API theo dòng `_index`/`_source`; Cassandra: `COPY` của `cqlsh`; Redis không có khái niệm "bảng" nên có lẽ không áp dụng) — vẫn là đúng vấn đề mapping cột/field mà #8 gặp phải, chỉ chuyển từ UI form sang CLI flag, không tự nhiên biến mất.
- File lớn: đọc/ghi streaming thay vì load hết vào RAM (khác cách `export.rs` hiện làm, vốn nhận `QueryResult` đã có sẵn trong bộ nhớ) — ảnh hưởng tới `QueryDriver` có cần thêm method streaming hay tái dùng nguyên trạng.
- Có đáng làm cả `export` CLI không, hay chỉ `import` (TUI đã có `Ctrl+E` export CSV/JSON cho kết quả đang xem, dù chỉ theo từng query/kết quả một, không phải "dump nguyên bảng" như `mongoexport`/`pg_dump`).

