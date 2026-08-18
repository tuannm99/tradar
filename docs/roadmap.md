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

1. **Navigator thêm cấp schema/database + nhóm theo loại object** (Tables/Views/Functions/Procedures/Triggers/Indexes dưới mỗi schema). **Đảo ngược quyết định "dừng ở 2 cấp"** đã chốt lúc xây navigator ban đầu (`docs/backlog/roadmap-sub-project.md` mục 6 "Bỏ cấp `database` giữa") và giữ nguyên lúc rà mockup lần 2 (`docs/backlog/mockup-ui-2026-08-15.md`, ghi chú "Navigator phẳng hơn mockup") — lý do dừng khi đó vẫn còn nguyên giá trị kỹ thuật: cần thêm method "liệt kê schema/database" + "liệt kê view/function/procedure/trigger" trên `QueryDriver`, phải nghĩ rõ từng connector trả gì khi không có khái niệm đó (Elasticsearch không có schema, Mongo/Redis không có view/function/trigger, Cassandra có keyspace nhưng không có view/procedure). Cần quyết: mỗi connector implement tối thiểu những gì, cấp nào ẩn khi rỗng thay vì hiện trống.
2. **Table designer qua UI** — thêm/sửa/xoá cột, constraint, index, FK; tạo bảng mới; đổi tên bảng. Rủi ro cao nhất trong 5 mục Schema & DDL: sinh DDL đúng cú pháp cho từng dialect (`ALTER TABLE` Postgres khác SQLite khác Cassandra), UI cho một thao tác vốn cần nhiều field (kiểu, null-able, default, cascade...) trong khi app hiện chỉ có `TextInput`/`TextArea` một dòng/nhiều dòng chưa có form nhiều field phức tạp kiểu này (form connection 3 field là ví dụ gần nhất, nhưng đơn giản hơn nhiều). Có thể cân nhắc hướng rẻ hơn: sinh DDL rồi cho user review/sửa tay trước khi chạy (giống cách row-edit làm với UPDATE/DELETE) thay vì UI point-and-click đầy đủ ngay từ v1.
3. **Schema diff/compare** — so 2 connection hoặc 2 schema, liệt kê khác biệt (cột thiếu/thừa, kiểu khác, index khác). Cần `list_schema` đủ chi tiết ở cả 2 phía (đã có cho Postgres/SQLite qua PK, nhưng chưa có index/constraint/default) trước khi so được gì có ý nghĩa.
4. **Migration/version-control tích hợp** — chưa rõ hình dạng: file migration kiểu Flyway/Alembic đọc từ thư mục, hay chỉ là "lưu lịch sử DDL đã chạy" đơn giản hơn? Cần hỏi lại phạm vi trước khi nghĩ kiến trúc.
5. **ERD (sơ đồ quan hệ bảng)** — TUI nên đây là box-drawing/ASCII render trong terminal, không phải đồ hoạ thật như DataGrip; cần nghĩ shape trước (danh sách bảng + FK vẽ bằng ký tự, hay chỉ liệt kê quan hệ dạng text). Phụ thuộc dữ liệu FK đã có sẵn ở PK/index nếu #2-3 làm trước.

**Query & editor**

6. **Autocomplete theo ngữ cảnh sâu** (gợi ý theo join/FK — gõ sau `JOIN` gợi ý bảng có FK liên quan, gõ sau `.` trên alias gợi ý đúng cột của bảng đó). Nguồn dữ liệu cần thêm: FK giữa các bảng (chưa có trong `SchemaInfo`/`ColumnInfo` hiện tại, chỉ có `primary_key: bool` chứ không có "cột này FK tới bảng/cột nào") — có thể dùng chung dữ liệu với ERD (#5) và table designer (#2) nếu làm theo đúng thứ tự phụ thuộc.
7. **"Generate SQL" từ UI** — tạo câu SELECT/INSERT/UPDATE tự động từ việc chọn bảng/cột qua giao diện, không gõ tay. Cần làm rõ khác gì với CRUD snippet đã có (navigator `c`/`r`/`u`/`d` sinh khung Create/Read/Update/Delete với placeholder `<tên_cột>`) — nếu ý là "point-and-click chọn cột thay vì gõ tay điền placeholder", đây là mở rộng của CRUD snippet chứ không phải tính năng mới từ đầu; nếu ý là "query builder trực quan" (chọn bảng, kéo điều kiện WHERE, chọn JOIN) thì là một sub-project lớn riêng, gần với #2 về độ phức tạp UI.

**Data grid**

8. **Import CSV/Excel/JSON vào bảng** — hướng ngược lại `export.rs` đã có (`to_csv`/`to_json`). Excel (`.xlsx`) cần thêm dependency mới (không có sẵn trong workspace, CSV/JSON thì không cần). Cần quyết cách map cột file vào cột bảng (theo tên header, theo thứ tự, hay UI cho chọn) và cách xử lý lỗi giữa chừng (dừng ở dòng lỗi đầu tiên, giống `Ctrl+A` chạy nhiều câu — có tiền lệ sẵn để theo).
9. **Sort theo cột (click header)** — client-side trên `QueryResult` đã tải (giống filter `/` đã có), không phải `ORDER BY` gửi lại DB. Rẻ hơn các mục khác trong nhóm này, đứng cạnh filter/column-types đã có sẵn hạ tầng để mở rộng. **Plan đã chốt (2026-08-19), chưa code** — xem "Plan: #9 Sort theo cột" ngay bên dưới.
10. **Multi-filter kết hợp** — hiện chỉ 1 ô filter text đơn khớp bất kỳ cột nào (`/`). Cần nghĩ UI cho nhiều điều kiện cùng lúc (theo cột cụ thể, AND/OR) mà không đụng vỡ ý nghĩa của filter đơn hiện có.
11. **Group-by trong grid** — client-side, cần nghĩ trước cả UI (group theo cột nào, hiện aggregate gì) lẫn có đáng làm trong một results grid vốn thiết kế cho xem/sửa row-by-row hay không (khác hẳn mục đích của group-by).
12. **Mở rộng edit-cell/delete-row ngoài single-table-with-PK** — **rủi ro cao nhất trong toàn bộ danh sách**, cân nhắc kỹ trước khi nhận làm: `single_table_source`/`build_sql_edit` cố tình bảo thủ (từ chối join/view/no-PK) đúng vì đoán sai bảng nguồn nghĩa là sinh `UPDATE`/`DELETE` nhắm sai chỗ — hậu quả là mất/sửa nhầm dữ liệu thật, không phải một tính năng thiếu vô hại. Nếu làm, cần một cơ chế xác định nguồn đáng tin hơn heuristic hiện tại (có thể là hỏi DB trực tiếp qua `EXPLAIN`/system catalog thay vì tự parse SQL), và có lẽ vẫn nên giữ nguyên tắc "từ chối rồi nói rõ lý do" cho các trường hợp còn mơ hồ thay vì cố đoán bừa.

**Thứ tự đã chốt (2026-08-19)**, user duyệt đề xuất theo rủi ro/phụ thuộc kỹ thuật, không tự chọn thứ tự khác:

- **Tier 1 (làm trước, rẻ/độc lập)**: #9 Sort theo cột → #8 Import CSV/JSON.
- **Tier 2 (nền tảng)**: #1 Navigator schema/database + nhóm object.
- **Tier 3 (dùng chung dữ liệu FK vừa thêm ở #1)**: #6 Autocomplete ngữ cảnh sâu → #5 ERD.
- **Tier 4 (cần chốt phạm vi trước khi code)**: #7 Generate SQL từ UI → #10 Multi-filter kết hợp.
- **Tier 5 (lớn, tách nhiều bước nhỏ)**: #2 Table designer → #3 Schema diff/compare → #4 Migration/version-control.
- **Tier 6 (để cuối, #12 cần bàn lại có đáng làm không)**: #11 Group-by trong grid → #12 Mở rộng edit-cell/delete-row ngoài single-table-with-PK.

## Plan: #9 Sort theo cột

Chốt qua `AskUserQuestion` (2026-08-19): kích hoạt bằng **phím tắt + click header**; chu kỳ **asc → desc → không sort**; sort **theo kiểu dữ liệu khi biết được**, còn lại so chuỗi. Chưa code — plan dưới đây để bắt tay khi tới lượt (Tier 1).

**Kiến trúc dữ liệu**
- `ResultsComponent` thêm `sort: Option<(usize, SortDirection)>` (index cột + `SortDirection::{Asc, Desc}`), reset về `None` trong `set_result()` (giống `filter`), **giữ nguyên** trong `set_result_keeping_cursor()` (giống `filter` — đây là refresh cùng shape, không phải kết quả mới).
- `sort_by_column(index: usize)`: nếu `index` khác cột đang sort → set `Asc`; cùng cột → cycle `Asc → Desc → None`.

**Điểm cắm — phải sửa đúng 2 chỗ đang trùng logic**
Code hiện tại có **hai nơi tính filtered-indices độc lập**: `ResultsComponent::visible_items()` (dùng cho selection/edit/yank) và `draw_table_body()` tự gọi lại `filter_table_rows()` bên trong nó (dùng để vẽ). Cả hai đều phải cộng thêm bước sort để không lệch nhau — cách sạch nhất là gộp thành 1 hàm dùng chung `visible_and_sorted_rows(rows, filter, sort, column_types) -> Vec<usize>`, gọi từ cả `visible_items()` lẫn `draw_table_body()`, thay vì thêm sort riêng ở từng chỗ.

**So sánh giá trị khi sort**
- Có `column_types[index]` biết được (đã có sẵn từ mục "Bảng kết quả không hiện kiểu cột" — `docs/backlog/mockup-ui-2026-08-15.md`) → parse số (`f64`) rồi so số nếu là kiểu số; parse lỗi thì rơi về so chuỗi cho đúng hàng đó (không crash, không rớt cả cột về chuỗi vì một giá trị lỗi).
- Không biết type (Documents-table-view luôn vậy, `column_types` rỗng theo thiết kế) → so chuỗi.
- `None`/rỗng luôn xếp cuối bất kể `Asc`/`Desc` (tránh giá trị thiếu nhảy lên đầu khi sort giảm dần).

**Bất biến phải giữ**: gutter số thứ tự **vẫn đọc theo vị trí gốc trong result**, không đánh số lại theo vị trí trên màn hình — miễn phí vì sort chỉ tráo thứ tự trong `Vec<usize>`, không đổi nội dung `rows[]`. Edit-cell/delete-row không bị ảnh hưởng (đi qua đúng index gốc để build `UPDATE`/`DELETE`).

**Kích hoạt**
- Phím: `Command::SortColumn` mới, bind `s` trong `Context::Results` (còn trống — đã kiểm tra không đụng `y/h/l/enter/d/space/t/r/e/c`), tác động lên `selected_col` hiện tại.
- Chuột: thêm `ResultsComponent::click_header(column, row) -> bool` — hit-test hàng `rows_area.y - 1` (đúng vị trí header, theo comment sẵn có "rows_area excludes... the header row") bằng `column_spans` đã ghi lúc `draw()`; chỉ có tác dụng khi đang ở `Table` hoặc `Documents` table-view (không có header ở JSON view). `QueryScreenComponent::handle_mouse_event` gọi hàm này trước/cạnh `results.click(...)` hiện có.

**Hiển thị**: thêm mũi tên `▲`/`▼` sau tên/icon cột đang sort trong header cell (cùng nhóm ký tự đơn-width đã dùng cho fold gutter `▸`/`▾`, không phải ký tự mới rủi ro lệch cột). Tiêu đề panel Results nối thêm đoạn `— sort: <col> ▲` cạnh đoạn `filter:` đã có khi đang sort.

**Scope rõ**: chỉ `QueryResult::Table` và `Documents` ở table-view (`t`) — không áp dụng JSON view, không áp dụng `Affected`.

**Test dự kiến**: cycle Asc→Desc→None, sort số vs chuỗi, giá trị thiếu xếp cuối, gutter giữ đúng vị trí gốc khi sort, `set_result` xoá sort / `set_result_keeping_cursor` giữ sort, click header kích hoạt đúng cột, phím `s` qua `dispatch_command`.
