# Autocomplete ngữ cảnh sâu (JOIN/FK) + ERD — xong (2026-08-20)

Tier 3 trong đợt so sánh DataGrip/DBeaver/Studio3T (`docs/roadmap.md`): gap #6 (autocomplete) rồi #5 (ERD), làm liền nhau vì #5 dùng lại đúng dữ liệu FK #6 thêm vào. Cả hai đều cần `AskUserQuestion` chốt phạm vi trước khi code, theo đúng pattern đã dùng xuyên suốt `docs/backlog/`.

## Dữ liệu FK mới — nền tảng dùng chung cho cả hai việc

`ColumnInfo` (`crates/tradar-query-workbench/src/query_driver.rs`) thêm `foreign_key: Option<ForeignKeyRef>` (`table`/`column`), mirror đúng pattern `primary_key: bool` đã có. `ColumnInfo::new` default `None`, mọi call site literal-construct `ColumnInfo` (Postgres/SQLite/Cassandra) phải update — Mongo/ES/Redis dùng `ColumnInfo::new` nên không đụng gì.

**Phạm vi connector** (chốt qua `AskUserQuestion`, khuyến nghị được chọn): Postgres + SQLite. Cassandra không có khái niệm FK trong CQL, giữ `None` luôn.

- **Postgres**: một round trip nữa kiểu `information_schema.table_constraints` join `key_column_usage`/`constraint_column_usage`, lọc `constraint_type = 'FOREIGN KEY'`. Key bằng bare table name (không schema-qualify) — `qualify_colliding_names` chỉ prefix tên khi đụng độ, nên đây là chỗ "biết trước sẽ miss" hiếm gặp, chấp nhận được.
- **SQLite**: `PRAGMA foreign_key_list($1)` mỗi bảng, cột `to` (referenced column) `NULL` nghĩa là FK trỏ tới PK ngầm định — trường hợp này bỏ qua, không cố resolve tiếp.

`same_table(query_name, schema_name) -> bool` chuyển từ `completion.rs` (private) lên `query_driver.rs` (public) — so khớp tên bảng chịu được cả hai chiều schema-qualified/bare, dùng chung bởi completion (alias/JOIN) lẫn ERD (đi theo FK reference).

## #6 — Autocomplete ngữ cảnh sâu

Trước đây `CompletionSource` hoàn toàn không nhìn ngữ cảnh: 1 danh sách phẳng (keyword + mọi tên bảng + mọi tên cột gộp toàn cục, dedupe theo tên) prefix-match không phân biệt đang gõ ở đâu. Gõ `.` sau alias không làm gì cả vì `.` không phải word char trong `query_editor.rs`'s `word_before_cursor()`.

**`CompletionContext`** (`query_driver.rs`, hàm `completion_context(text_before_cursor)`) — đọc thuần cú pháp, không cần schema, tách biệt với việc resolve thật (giống cách `single_table_source` đã tách "đọc SQL" khỏi "khớp với schema"):

- `TableColumns { table }` — cursor ngay sau `alias.` (có thể kèm phần cột đang gõ dở). Resolve alias thuần cú pháp qua `from_join_table_refs` (quét mọi `FROM`/`JOIN` + alias tuỳ chọn, tái dùng đúng logic skip-alias của `single_table_source`) — không cần schema, vì `FROM users u` → `u` = `users` chỉ là ngữ pháp SQL.
- `JoinTarget { known_tables }` — cursor đang gõ tên bảng ngay sau `JOIN`. Bug thật gặp lúc code: token `JOIN` đang gõ dở (chưa hoàn thành) tự lọt vào `known_tables` nếu quét cả token cuối — sửa bằng cách chỉ đọc `from_join_table_refs` trên phần token *trước* từ khoá `JOIN` đang kích hoạt context, không phải toàn bộ buffer.

`CompletionSource::matches_in_context(prefix, context)` (`completion.rs`) — giữ nguyên `matches()` cũ cho `CompletionContext::None`, thêm:

- `TableColumns`: lọc `entry.columns` của đúng bảng đó, **cho phép prefix rỗng** hiện hết cột (khác `matches()` — vốn trả rỗng khi prefix rỗng, vì gõ `u.` xong chưa gõ gì cũng nên thấy hết cột).
- `JoinTarget`: bảng có FK tới/từ bảng đã có trong `known_tables` (cả hai chiều — nếu A có cột FK tới B, B cũng "liên quan" khi gõ sau A) **xếp lên đầu** (khuyến nghị được chọn qua `AskUserQuestion`, không lọc ẩn bảng khác — vẫn gõ tay JOIN bảng bất kỳ được).

`CompletionSource` giờ giữ thêm `schema: Vec<SchemaInfo>` (trước chỉ có `candidates` đã flatten/dedupe, mất thông tin bảng nào chứa cột nào) để tra cứu theo ngữ cảnh.

`query_editor.rs` thêm `text_before_cursor()` (dùng `text()[..cursor_offset()]` sẵn có) cho `query_screen.rs`'s `refresh_completions()` gọi `completion_context()`. `replace_word_before_cursor()` không đổi gì — vẫn dừng ở `.` (không phải word char) nên chấp nhận completion đúng phần sau dấu chấm mà không cần sửa logic chèn text.

## #5 — ERD

**Phạm vi** (chốt qua `AskUserQuestion`, cả ba câu đều theo khuyến nghị trừ câu vẽ FK):

- **Lân cận bảng được chọn**, không phải toàn schema cùng lúc — 1 bảng focal + mọi bảng có FK trực tiếp tới/từ nó (1 hop, không transitive closure). Lý do: chưa có dữ liệu thực tế app dùng ở quy mô schema bao nhiêu bảng (`docs/roadmap.md` note "unaddressed territory"), nên tránh cam kết một thuật toán layout cho graph không giới hạn.
- **Vẽ đường nối box-drawing thật sự** (không phải liệt kê text cạnh box — đây là lựa chọn *không* theo khuyến nghị, user chọn phương án rủi ro kỹ thuật cao hơn).
- **Overlay gần toàn màn hình** (`centered_rect(92, 92, area)`), không phải side panel cố định như Navigator.

**Kiến trúc module** (`crates/tradar-query-workbench/src/components/erd.rs`) — tách hai lớp:

1. `neighborhood(schema, focal_name) -> Option<Neighborhood>` + `render(&Neighborhood) -> Vec<String>` — hàm thuần, không đụng ratatui, test được bằng so sánh chuỗi chính xác.
2. `ErdComponent` — overlay ratatui mỏng bọc ngoài, state machine `Picking(HistoryPickerComponent)` → `Viewing { lines, scroll }`, cùng shape với `HistoryPickerComponent`/`SnippetPickerComponent`: không phải `Component`, `QueryScreenComponent` sở hữu trực tiếp, chiếm hết input lúc mở.

**Table picker tái dùng `HistoryPickerComponent`** thay vì viết picker mới — đây vốn chỉ là list phẳng có vim motion + Enter/Esc, đúng thứ ERD cần cho "chọn 1 bảng theo tên". Duy nhất phải sửa: `HistoryPickerComponent` trước đó hardcode tiêu đề `"History — ..."` trong `draw()` — thêm `with_title()` builder để ERD hiện đúng `"ERD — pick a table"` thay vì "History" (bug thật phát hiện lúc test tay qua tmux, không phải lúc code).

**Renderer — kỹ thuật bitmask cho box-drawing**: mỗi ô canvas tích luỹ 4 bit (Bắc/Nam/Đông/Tây) từ mọi đoạn thẳng đi qua nó (`Canvas::add_hline`/`add_vline`), chuyển thành đúng 1 ký tự box-drawing (`─│┌┐└┘├┤┬┴┼`) ở bước cuối (`flush_connections`) — cách này tránh phải tự suy luận từng trường hợp góc/chữ T bằng tay: một trunk dùng chung bởi nhiều bảng tự động ra đúng `┬`/`┴`/`├`/`┤`/`┼` tuỳ chỗ nhánh gặp trunk. Bug thật gặp lúc code: `match bits { NORTH | SOUTH => ... }` là or-pattern của Rust (khớp `bits==NORTH` HOẶC `bits==SOUTH` riêng lẻ), không phải giá trị bitwise-OR — phải định nghĩa hằng số hợp trước (`const NS: u8 = NORTH | SOUTH`) rồi match trên hằng đó.

**Layout 3 cột**: bảng lân cận "incoming" (tham chiếu tới focal) bên trái, focal ở giữa, "outgoing" (focal tham chiếu tới) bên phải — mũi tên luôn đọc trái→phải, từ bảng tham chiếu tới bảng được tham chiếu, bất kể vẽ bên nào. `draw_side()` từng có bug: vẽ luôn cả đoạn "trunk → focal" cho *mọi* hàng lân cận thay vì chỉ hàng hub — gây `┬` giả ở những chỗ không nên có nhánh; sửa bằng cách tách rõ "nhánh riêng của từng lân cận" (việc của `draw_side`) khỏi "nhánh chung vào focal, vẽ đúng 1 lần" (việc của `render`, sau khi gọi `draw_side`).

Bug thứ hai: `box_width()` tính thiếu 1 cột cho tiêu đề dài hơn nội dung (` {name} ` cần `name.len() + 5`, không phải `+ 4`) — tiêu đề đè lên góc `┐` bên phải, không lỗi biên nhưng vỡ hình. Phát hiện bằng cách in thử ASCII output ra terminal (`eprintln!` trong test tạm `#[ignore]`, xoá sau khi soi bằng mắt xong) trước khi tin bất kỳ test string-so-sánh nào.

**Trigger**: `Command::ShowErd` (`crates/tradar-core/src/keymap.rs`), phím mặc định `F4`, bind ở `Context::QueryScreen` giống `History`/`OpenSnippets`. `open_erd()` build danh sách tên bảng từ `self.engine.schema()` — lưu ý `engine.schema()` chỉ fetch **một lần lúc connect**, không tự refresh khi chạy `CREATE TABLE` mới trong session (phát hiện lúc test tay: tạo bảng xong bấm F4 ngay không thấy gì, phải disconnect/reconnect lại tab mới thấy bảng mới trong picker — hành vi đã có từ trước, không phải bug của ERD, nhưng là cạm bẫy dễ gặp khi tự test tính năng mới cần schema tươi).

**Test**: 13 test trong `erd.rs` — `neighborhood()` cả hai chiều + dedupe + bảng không quen biết, `render()` case 1 cặp bảng (không cần trunk), case nhiều lân cận chia sẻ trunk, case wide table bị cắt ở `MAX_COLUMNS`, case không có lân cận nào, và một test **exact-string** cho layout hỗn hợp (1 incoming + 2 outgoing) pin cứng cả sơ đồ ASCII thật — test này verify bằng mắt một lần qua `eprintln!` trước khi chốt thành assertion, vì hình học connector kiểu này dễ "trông đúng nhưng sai một ô" mà unit test rời rạc (chỉ check `contains("orders")`, `contains('>')`...) không bắt được. Cộng test cho `ErdComponent`'s state machine (picker → viewing, Esc đóng cả hai state — bug thật: `Context::List` không có binding cho `Cancel`, phải resolve qua `&[Context::Prompt, Context::List]` giống `HistoryPickerComponent` mới bắt được phím Esc).

**Test tay qua tmux** (không có sqlite3 CLI trong môi trường, phải tạo bảng bằng chính app): connect SQLite trống → gõ `CREATE TABLE` trực tiếp trong editor (lưu ý: editor non-modal mặc định, phím `i`/`Esc` không phải vim mode-switch — `Esc` khi đang gõ sẽ kích `Command::Back` về connection picker, không phải "thoát insert mode"; và `tmux send-keys` nuốt mất `;` trừ khi escape thành `\;`) → reconnect để schema refresh → F4 → chọn bảng → soi ASCII output khớp y hệt bản render thật trong ứng dụng.
