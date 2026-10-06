# Neovim: navigator panel thành cây thật (2026-10-06)

## Bối cảnh

`docs/roadmap.md` (chốt 2026-10-04) xếp "Navigator dạng cây (`connection → schema → bảng → cột`) trong Neovim" vào mục "Mục nhỏ" của "Thứ tự tiếp theo" — một trong 4 việc code cụ thể không phụ thuộc bước #1 ("dùng thật vài ngày", việc của người dùng). Người dùng hỏi "phân tích tiếp theo nên làm gì"; agent rà roadmap, đề xuất bắt đầu từ mục này vì nó thuần UI phía plugin (không đụng `tradar-server`/protocol, khác 3 việc còn lại trong "Mục nhỏ" đều cần đổi server hoặc `QueryResult`), người dùng đồng ý làm thử.

## Vấn đề cụ thể

`nvim/lua/tradar/render.lua`'s `M.schema` (cũ) + `init.lua`'s `M.schema()` (panel `<leader>rs`) render phẳng: mọi bảng của connection, **mỗi bảng luôn hiện hết toàn bộ cột ngay lập tức**, không có khái niệm gấp/mở. Hai vấn đề:

1. Schema/keyspace/database (Postgres schema, Cassandra keyspace, MongoDB database — field `entry.schema` đã có sẵn từ RPC `schema`, dùng duy nhất để nối tiền tố tên bảng qua `M.qualified`) không có dòng nhóm riêng — bảng của schema khác nhau trộn lẫn, không phân biệt được bằng mắt.
2. Một schema nhiều bảng × nhiều cột là một khối text khổng lồ ngay khi mở panel — đúng kiểu vấn đề mà cây thật (gấp được) giải quyết, TUI's navigator (`crates/tradar-app/src/components/navigator.rs` + `flatten_outline`) đã có từ 2026-08-19.

## Thay đổi

### `render.lua`: `M.schema` (cũ) → `M.schema_tree(entries, expanded)`

Thay hẳn hàm render phẳng cũ bằng một hàm xây cây thật, `expanded` là state gấp/mở do caller giữ (set đơn giản, key → bool):

- **Nhóm theo `entry.schema`**: bucket theo **thứ tự xuất hiện đầu tiên**, không sort lại — copy chính xác thuật toán `flatten_outline` (`crates/tradar-query-workbench/src/components/query_screen.rs`) đã dùng cho TUI ("Bucketed by first-seen order, not sorted — a schema list sorted here would silently disagree with whatever order the driver's own query already returned in"), quan trọng vì Postgres's `list_schema` gộp hai query (bảng/view + function/procedure) nên cùng một schema có thể xuất hiện **không liền nhau** trong `entries` — một scan tuyến tính đơn giản ("đổi schema thì mở nhóm mới") sẽ tạo nhầm hai nhóm trùng tên; bucket bằng map theo key tránh được lỗi này (verify bằng test riêng, xem "Test").
- **Entry không có `schema` (SQLite, Elasticsearch, Redis)**: không có dòng nhóm, bảng hiện trực tiếp — giữ nguyên hành vi phẳng cũ cho các driver này, đúng quy tắc "driver nào không set field thì cây không đổi gì" mà TUI's `flatten_outline` cũng tuân theo.
- **Mặc định gấp/mở khác nhau theo cấp**: nhóm schema mặc định **mở** (chỉ là danh sách tên, rẻ); bảng mặc định **gấp** (cột mới là phần tốn chỗ thật — đúng thứ gây ra "khối text khổng lồ" ở trên). `expanded[key]` lưu *ngoại lệ* so với mặc định đó, không phải giá trị tuyệt đối — đọc lại qua `node_is_open` (`init.lua`) theo đúng quy tắc `expanded[key] == nil` → dùng mặc định theo `node.kind`.
- **Label bảng trong một nhóm dùng `entry.name` thô, không qua `M.qualified`** — giống `push_table`'s `label: table.name.clone()` của TUI. Khác biệt quan trọng: Cassandra's `entry.name` **đã sẵn** `"demo.events"` (keyspace nối thẳng vào tên, khác Postgres để `schema`/`name` riêng) — `M.qualified` chỉ thêm tiền tố khi tên chưa có, nên dùng `entry.name` trực tiếp ở đây không tạo lại đúng bug `demo.demo.users` vừa sửa ở đợt 2026-10-04 (verify bằng test riêng). `<CR>`'s insert-text vẫn luôn qua `M.qualified(entry)` (không đổi) — bảng trùng tên giữa hai schema vẫn chèn đúng tên đã phân biệt.
- Trả `lines` + `nodes` (song song từng dòng): `{kind = "schema"|"table"|"column", key, insert}` — `key` là thứ gấp/mở tra vào `expanded`, `nil` cho dòng cột (lá, không gấp được); `insert` là text `<CR>` chèn, `nil` cho dòng nhóm schema (không có gì để chèn cho một thư mục gom nhóm).

### `init.lua`: `M.schema()` dùng state mới, thêm `<Tab>` gấp/mở

- State mới: `schema_entries` (cache để gấp/mở không gọi lại RPC `schema`), `schema_nodes` (song song với dòng đang hiện), `schema_expanded` (set gấp/mở, **không khoá theo connection** — panel dùng chung một buffer `tradar://schema` cho mọi connection nên đổi connection có thể vô tình hiện sẵn-mở một bảng trùng tên từ connection trước; chấp nhận vì chỉ ảnh hưởng hiển thị, không sai dữ liệu, và khoá theo connection thêm độ phức tạp cho một lợi ích nhỏ).
- `node_is_open(node)`: đọc lại đúng quy tắc mặc định `schema_tree` dùng để render (`current ~= nil` thì dùng nguyên, không thì mặc định theo `node.kind == 'schema'`) — hai nơi phải khớp nhau tuyệt đối, nên viết thành một hàm chung logic (không lặp lại điều kiện ở hai chỗ).
- `<Tab>` gấp/mở dòng dưới con trỏ (nhóm schema hoặc bảng — không làm gì trên dòng cột, vì `node.key == nil`). `<CR>` giữ nguyên hành vi chèn tên cũ cho bảng/cột (`node.insert`), **thêm** gấp/mở khi đứng ở dòng nhóm schema (`node.insert == nil` nhưng `node.key` có) — vì không có gì để chèn cho một dòng nhóm, nhân tiện cho `<CR>` cũng gấp/mở được thì tiện hơn ép người dùng nhớ chỉ `<Tab>` làm việc đó cho nhóm. Bảng luôn có `insert` nên `<CR>` trên bảng **không** gấp/mở (chỉ chèn) — tránh vừa chèn tên vừa đổi trạng thái gấp cùng lúc, không rõ ràng; `<Tab>` là cách duy nhất gấp/mở một bảng.
- Giữ nguyên cursor (theo dòng, kẹp trong khoảng hợp lệ) qua mỗi lần vẽ lại — gấp/mở không nhảy con trỏ về đầu buffer.

## Test

Không có `nvim` hay interpreter Lua sẵn trong sandbox này — cài `lua5.3` qua `apt` (không ảnh hưởng gì khác) để chạy được `render.lua`'s `schema_tree` như một hàm Lua thuần (`require`, không đụng API Neovim nào ngoài `vim.fn.strdisplaywidth`/`vim.islist`/`vim.json.encode`/`vim.NIL`, mock lại bằng vài dòng), viết một harness độc lập (`lua5.3 test_schema_tree.lua`, không commit vào repo — chỉ chạy tay trong sandbox) kiểm 6 kịch bản:

- Postgres hai schema, chưa gấp/mở gì: đúng 4 dòng (2 header + 2 bảng), không cột nào hiện.
- Mở một bảng: cột của đúng bảng đó hiện, bảng khác/nhóm khác không đổi.
- Gấp một nhóm schema: bảng của nhóm đó biến mất hoàn toàn, nhóm khác không ảnh hưởng.
- Cassandra (`entry.name` đã có tiền tố keyspace sẵn): label + insert đều là `demo.events`, không lặp thành `demo.demo.events`.
- SQLite (không `schema`): không có dòng nhóm, cây vẫn đúng 2 cấp bảng→cột như hành vi cũ.
- `schema` xuất hiện không liền nhau trong `entries` (mô phỏng Postgres gộp bảng+function): vẫn đúng **một** nhóm, không tạo nhóm trùng tên thứ hai.

Tất cả 6 kịch bản (20 assertion) pass. `luac5.3 -p` xác nhận `render.lua`/`init.lua` không lỗi cú pháp. **Chưa làm được**: chạy tay thật trong Neovim có giao diện (như các đợt trước "chạy tay" làm) — không có `nvim` trong sandbox này; cần người dùng tự xác nhận khi dùng thật (đúng bước #1 của roadmap).

## Chưa làm

- Không có cấp "Tables/Views/Functions/Procedures" dưới schema như TUI's navigator có cho Postgres — roadmap chỉ xin đúng `connection → schema → bảng → cột`, không xin cấp đó; để sau nếu thật sự cần.
- Không có phím "mở tất cả"/"gấp tất cất" — `<Tab>` chỉ gấp/mở đúng 1 dòng dưới con trỏ. Chưa thấy nhu cầu cụ thể, thêm sau nếu người dùng thấy bất tiện khi dùng thật.
- `schema_expanded` không khoá theo connection (xem phần "Thay đổi" ở trên) — quyết định có chủ đích, chi phí thấp.
