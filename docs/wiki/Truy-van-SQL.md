# Truy vấn SQL trong Neovim

Áp dụng cho PostgreSQL, MySQL/MariaDB, SQLite, ClickHouse, Cassandra (CQL). Với Mongo/Redis/Elasticsearch xem [trang riêng](Mongo-Redis-Elasticsearch) — cách dùng giống nhau, khác ngôn ngữ.

Bắt đầu: tạo file `.sql`, thêm dòng đầu `-- tradar: tên-connection` (xem [Kết nối](Ket-noi)).

## Chạy câu lệnh

Tất cả phím có tiền tố `<leader>r` (đổi được bằng `setup{ prefix = ... }`).

| Phím | Việc |
|---|---|
| `rr` | chạy statement dưới con trỏ. Visual mode: các dòng đang chọn, **tách theo `;`** và chạy lần lượt (dừng ở lỗi đầu tiên; hỏi một lần nếu có câu nguy hiểm) |
| `ra` | chạy cả file theo thứ tự, dừng ở lỗi đầu tiên |
| `rx` | huỷ query đang chạy |
| `re` | `EXPLAIN` statement dưới con trỏ (không bao giờ `ANALYZE`, vì nó *chạy* câu lệnh) |
| `rt` | picker bảng (telescope): `<CR>` chèn tên, `<C-o>` sinh câu xem dòng vào buffer (không chạy), `<C-g>` mở menu sinh câu lệnh (create/read/update/delete; Mongo có thêm find latest / by _id / one, count, distinct, aggregate) |
| `rg` | sinh câu create/read/update/delete cho bảng dưới con trỏ (hoặc chọn bảng) |
| `rh` | history: `<CR>` dán, `<C-r>` chạy |
| `rs` | panel schema (cây schema → bảng → cột) |
| `rc` | đổi connection |
| `rm` | tải thêm dòng |
| `rv` | đổi bảng ↔ JSON (SQL: mảng object; tài liệu: JSON gốc) |

Ranh giới statement do chính driver quyết định. Chạy hoàn toàn async: có spinner và thời gian trên statusline, Neovim không bị đơ. Lệnh tương ứng: `:TradarRun`, `:TradarRunAll`, `:TradarCancel`, `:TradarMore`, `:TradarSchema`, `:TradarTables`, `:TradarHistory`.

`g?` trong buffer SQL, buffer kết quả hoặc panel schema hiện danh sách phím của buffer đó.

## Buffer kết quả

Dòng cuối cho biết số dòng và **thời gian database chạy câu lệnh** (`20/5000 rows · 123ms`; câu `UPDATE`/`DELETE`… là `N row(s) affected · 8ms`). Đó là thời gian server đo quanh lời gọi database — không gồm đường truyền socket hay việc vẽ lại — nên đáng tin hơn thời gian bạn cảm nhận được; statusline cũng dùng con số này. Là buffer Neovim thường (`j`/`k`/`gg`/`G`, `/`...). Mỗi trang 20 dòng (`setup{ page_size }`); trang kế tải bằng `<leader>rm` chứ không tự tải khi cuộn (`setup{ auto_load_more = true }` để như cũ). Query chạy lâu: thanh trên cùng của cửa sổ kết quả hiện spinner + thời gian + cách huỷ.

| Phím | Việc |
|---|---|
| `gK` | xem cả dòng hiện tại, mỗi trường một dòng |
| (mọi lệnh vim) | sửa thẳng trên lưới: `cw`, `dd`, `o`, `:s`, `:g`, `u`… — xem "Sửa trực tiếp" bên dưới |
| `:w` (hoặc `rw`) | xem trước toàn bộ câu lệnh rồi commit một lượt |
| `ru` | bỏ các sửa chưa commit, vẽ lại lưới |
| `gyc` / `gyr` / `gyC` | copy ô / dòng (visual: nhiều dòng) / cột |
| `gyj` / `gyv` / `gym` | copy cả kết quả dạng JSON / CSV / Markdown |
| `K` | xem trọn giá trị một ô |
| `[r` `]r` `r1`…`r9` `rx` `rp` `[R` `]R` `rF` `ro` `rD` `rR` `rW` | tab kết quả, lịch sử lần chạy, lọc/sắp xếp, so sánh, làm mới — xem các mục bên dưới và `Phim-tat` |
| `gd` | trên ô của cột khoá ngoại: chạy `SELECT` dòng được tham chiếu |
| `q` | đóng |

Xuất file: `:TradarExport csv|json|md|tsv [file]`.

Cột rộng hơn 40 ký tự bị cắt và thêm `…`; dùng `K` hoặc `gyc` để lấy giá trị đầy đủ. Lọc và sắp xếp (`rF`, `ro`) chỉ tác động lên các dòng **đã tải** — xem "Lọc và sắp xếp kết quả" bên dưới; với kết quả lớn, sửa `ORDER BY`/`WHERE` rồi `rr` lại để áp dụng trên cả bảng.

### Sửa trực tiếp

**Căn lề**: cột mà mọi ô (bỏ qua rỗng/`NULL`) là số thì căn phải, kể cả tiêu đề; số có số 0 đứng đầu (`007`, mã bưu chính, id) coi là chữ nên căn trái. Nếu một trang nạp thêm (`<leader>rm`) mang giá trị chữ vào cột đang là số, lưới tự vẽ lại và cột đó căn trái.

**Copy ô dài**: ô rộng hơn cột bị cắt `…` trên màn hình, nhưng yank của vim (`yy`, `y$`, `v…y`, `V…y`) vẫn bỏ **toàn bộ** chữ của ô bị cắt vào register (xuống dòng/tab trong giá trị thành khoảng trắng như trên lưới; yank theo khối `Ctrl-v` không được xử lý). Muốn đúng nguyên giá trị gốc: `gyc` (ô), `gyr` (dòng), `K` (xem trước).

Khi kết quả sửa được, lưới là một buffer sửa được như mọi buffer khác: gõ đè ô (`cw`, `ci"`…), xoá dòng (`dd`, `:g/pat/d`), thêm dòng (`o` / `yyp` rồi gõ giá trị), `:s`, macro, `u`/`Ctrl-r`… Chưa có gì chạy xuống DB cho tới khi bạn `:w`:

1. `:w` so lưới với các dòng đã vẽ và mở **buffer xem trước** `tradar://commit` liệt kê mọi câu `UPDATE` / `DELETE` / `INSERT` (do server sinh, thứ tự: update → delete → insert).
2. Trong buffer đó, `:w` hoặc `<CR>` chạy lần lượt, `q` huỷ. Xong tự chạy lại truy vấn gốc và giữ nguyên dòng đang đứng.

Cách nhận dòng: theo **giá trị cột khoá** hiển thị trên dòng, không theo số dòng — nên `dd`, `yyp`, `:sort`, đổi chỗ dòng đều đúng.

- Dòng biến mất → `DELETE`; ô đổi → `UPDATE` (**một câu cho mỗi dòng**, gom mọi ô đã đổi của dòng đó: `UPDATE t SET a = …, b = … WHERE khoá`; gõ `NULL` để đặt NULL); ô không đụng tới (kể cả ô bị cắt `…` hay có xuống dòng) thì giữ nguyên.
- Dòng có khoá chưa từng có → `INSERT`. Ô để trống = bỏ cột đó (DB dùng `DEFAULT`), gõ `NULL` để đặt NULL tường minh. Sửa chính cột khoá của một dòng cũng hiện thành `DELETE` + `INSERT` — xem kỹ ở bước xem trước.
- Plugin không đoán: dòng trùng khoá với dòng khác, dòng không tách được thành đủ cột (giá trị chứa ` │ `), hay sửa header/footer thì `:w` báo lỗi và dừng.
- **Một transaction**: với Postgres/MySQL/SQLite, commit chạy `BEGIN` → các câu → `COMMIT`. Một câu lỗi thì `ROLLBACK` (không câu nào được áp dụng), lưới vẽ lại từ DB và buffer xem trước mở lại với **toàn bộ** câu lệnh cùng thông báo lỗi để sửa rồi `:w` lại. Nếu bạn đã tự `BEGIN` trước đó, plugin cảnh báo và chạy bên trong transaction của bạn, không `COMMIT` hộ. Connector khác (Mongo/ES…) không có transaction: lỗi giữa chừng thì các câu đã chạy giữ nguyên và xem trước chỉ còn phần chưa chạy.
- Trong lúc có sửa chưa commit, không tải thêm trang, không đổi view JSON, các phím theo ô (`gyc`, `K`, `gd`…) tạm tắt; chạy truy vấn mới sẽ bỏ các sửa đó (có cảnh báo).
- Hộp xem trước thay cho bước hỏi của guard, nên connection `protected` chỉ bị hỏi một lần.

Chỉ sửa được khi kết quả đến từ **một bảng có khoá chính** và cột khoá có trong kết quả (và connector hỗ trợ ghi: Postgres/MySQL/SQLite); nếu không, lưới giữ chế độ chỉ đọc và các phím sửa (`i`, `dd`, `o`…) báo rõ lý do thay vì "E21".

## Điều hướng theo schema

- `K` trên tên bảng/cột: kiểu, khoá chính, khoá ngoại, index; hiểu alias (`o.user_id` lấy từ `FROM`/`JOIN`). Không biết thì rơi về hover của LSP.
- `gd` trên tên bảng: **sinh** câu xem dòng của driver (`SELECT ... LIMIT 100`, Mongo `db.x.find({})`…) vào buffer; bấm `rr` để chạy. `setup{ open_runs = true }` để chạy ngay như trước.
- Panel schema (`rs`): `<Tab>` gấp/mở bảng hoặc nhóm, `<CR>` chèn tên (hoặc gấp/mở nếu đang ở dòng nhóm), `o` sinh câu xem dòng vào buffer (không chạy; `rr` để chạy), `c` sinh câu lệnh CRUD (chọn thao tác và cột).

### Sinh câu lệnh create / read / update / delete

`:TradarCrud [create|read|update|delete]` (hoặc `<leader>rg`, `c` trong panel schema, `<C-g>` trong picker `rt`) sinh câu mẫu của một bảng **bằng ngôn ngữ riêng của connector** — `INSERT`/`SELECT`/`UPDATE`/`DELETE` cho SQL và CQL, `insertOne`/`find`/`updateOne`/`deleteOne` cho Mongo, request cho Elasticsearch, lệnh theo kiểu key cho Redis. Bảng là bảng đang nằm dưới con trỏ (`people` trong `people`); không có thì hỏi chọn. Không gõ tên thao tác thì hỏi tiếp.

Với create/read/update, hỏi có chọn cột không: mặc định là tất cả (hoặc mặc định của thao tác, ví dụ `UPDATE` bỏ cột khoá chính khỏi `SET`); chọn **Pick columns…** để bật/tắt từng cột rồi **Done**. Delete không hỏi cột, điều kiện lấy từ khoá chính. Câu lệnh được **chèn dưới dòng con trỏ** (có dòng trống hai bên), với `<chỗ-trống>` để bạn điền — **không chạy**, vì ba trong bốn thao tác ghi dữ liệu; chạy bằng `<leader>rr` như câu thường (connection protected vẫn hỏi xác nhận).

Schema được cache 1 phút phía plugin và tự làm mới sau khi bạn chạy DDL (`CREATE`/`ALTER`/`DROP`/`RENAME`).

## Completion

Nguồn `blink.cmp` (`tradar.blink`, kích hoạt bằng `.`) hoặc `omnifunc` dự phòng. Hiểu ngữ cảnh: sau `alias.` chỉ gợi ý cột của bảng đó, sau `JOIN` ưu tiên bảng có khoá ngoại liên quan, sau `schema.`/`keyspace.` gợi ý bảng trong đó. Nếu cài `sqlls`, nên bỏ nguồn `lsp` cho `sql` trong cấu hình completion của bạn để khỏi trùng.

## Lỗi

Lỗi cú pháp được gạch chân đúng vị trí bằng `vim.diagnostic` (tự xoá ở lần sửa kế tiếp); nội dung lỗi đầy đủ mở trong buffer kết quả.

## Statusline

`require('tradar').status()` trả chuỗi như `db demo · 120/5000 rows · 83ms`: spinner khi đang chạy, `✗` khi ping nền (mỗi 15 giây) thấy connection rớt, `(idle)` khi server đã đóng nó vì không dùng (lệnh kế tiếp tự nối lại), `tx` khi đang trong transaction (cập nhật ngay sau `BEGIN`/`COMMIT`/`ROLLBACK`, không đợi lần ping kế), `⚠` với connection protected. Trong buffer kết quả, nó hiện connection **của kết quả** đó và thêm `✎ unsaved edits` khi lưới có sửa chưa commit. Rỗng ngoài buffer tradar, nên gắn vào lualine không tốn gì ở file khác.

## An toàn

Plugin hỏi xác nhận (đáp án mặc định là `Cancel`, nên `<CR>` phản xạ là an toàn) trước khi chạy:

- `UPDATE`/`DELETE` **không có `WHERE`**, `DROP`, `TRUNCATE` — trên mọi connection.
- **Mọi lệnh ghi** (`INSERT`/`UPDATE`/`DELETE`/DDL...) trên connection "protected": tên chứa một chuỗi trong `setup{ protected = {...} }` (mặc định `{"prod"}`, không phân biệt hoa thường).
- Mongo: `drop()`, `dropDatabase()`, `deleteMany({})`/`updateMany({}, …)` (bộ lọc rỗng). Redis: `FLUSHALL`/`FLUSHDB`/`SHUTDOWN`. Elasticsearch: `DELETE …`, `_delete_by_query`.
- `ra` (chạy cả file) chỉ hỏi **một lần** cho cả lô.

Đây là phân tích theo token (bỏ qua comment và chuỗi), không phải parser — nó bắt các lỗi gõ tay thường gặp chứ **không phải** rào chắn bảo mật. `setup{ confirm = false }` tắt hết.

## Huỷ query

`rx` huỷ phía client ngay; với **PostgreSQL và SQLite** còn huỷ thật phía database. Các database khác chỉ bỏ phần chờ (câu lệnh có thể vẫn chạy nốt ở phía server).

## Lưu và mở query

Hai chỗ để lưu một query:

- **Thư viện** — một thư mục chung cho mọi query, mở được từ bất kỳ project nào. `:TradarSave [tên]` (hoặc `<leader>rS`; bỏ trống tên thì hỏi, điền sẵn tên file hiện tại). Tên trần hoặc `thư-mục/tên` nằm dưới `queries_dir` và tự thêm `.sql` nếu chưa có đuôi; `/tuyệt-đối`, `~/x`, `./x` được hiểu đúng như gõ. File đã tồn tại thì cần `:TradarSave!` để ghi đè. Mặc định thư mục là `~/.config/tradar/queries` (cùng chỗ TUI lưu); đổi bằng `setup{ queries_dir = "~/sql" }`. Buffer đã `:TradarConnect` thì dòng `-- tradar: tên` được ghi vào đầu file để mở lại đúng connection.
- **Cạnh file hiện tại** — `:TradarSaveHere [đường dẫn]`: buffer đã có file thì chỉ ghi lại (như `:w`); tên trần nằm cùng thư mục file hiện tại (buffer mới thì thư mục làm việc); `thư-mục/tên` được tạo nếu chưa có. Buffer lấy tên file mới.

`:TradarOpen` (hoặc `<leader>rf`) duyệt thư viện bằng telescope (mới sửa nhất lên đầu, có preview; `<CR>` mở, `<C-s>` mở trong split, `<C-x>` xoá); không có telescope thì dùng `vim.ui.select`.

## Cheatsheet: query mẫu có sẵn

`<leader>rk` (hoặc `:TradarCheatsheet`) mở danh sách **query mẫu của đúng loại database** đang dùng — khỏi nhớ và khỏi đi tìm. Mỗi driver có 50–100 câu (Postgres 89, MySQL 87, ClickHouse 82, Mongo 82, Elasticsearch 79, Redis 95, SQLite 70, Cassandra 53) chia nhóm, mỗi mục một câu, bằng đúng ngôn ngữ của driver:

- **Explore / Activity / Size / Health** — bảng kèm dung lượng, câu đang chạy, ai đang khoá ai, index không dùng, `system.parts`/`query_log` (ClickHouse), `INFO`/`SLOWLOG` (Redis), `_cat/*` (Elasticsearch), `currentOp` (Mongo).
- **Query / SQL syntax** — cú pháp hay phải tra: CTE, window function, top-N mỗi nhóm, JOIN không khớp, `EXISTS`, `CASE`, phân trang keyset, ngày tháng, JSON; `argMax`/`LIMIT BY`/`FINAL` (ClickHouse); `bool`/`range`/aggregation (Elasticsearch); `$lookup`/`$group`/`$unwind` (Mongo); `ALLOW FILTERING`/`token()` (Cassandra).
- **Write** — `INSERT`, upsert (`ON CONFLICT` / `ON DUPLICATE KEY` / `$set`…), `UPDATE`, `DELETE`, `SET`/`EXPIRE`/`ZADD` (Redis), `_update_by_query` (Elasticsearch). Chạy qua guard như mọi câu lệnh ghi (hỏi lại khi `DELETE`/`UPDATE` không có `WHERE`).
- **Schema / Admin / Transaction** — `CREATE TABLE`, thêm/đổi cột, tạo index (Postgres `CONCURRENTLY`), `VACUUM`, `KILL`, tạo user và `GRANT`, `BEGIN`/`SAVEPOINT`.

- Có telescope: danh sách có **khung xem trước** là cả câu lệnh, gõ để lọc theo nhóm, tên **hoặc nội dung câu**. `<CR>` chèn câu vào buffer (dưới dòng con trỏ, chưa chạy); `<C-r>` chèn rồi chạy luôn. Không có telescope (hoặc `setup{ cheatsheet_picker = 'ui' }`) thì dùng danh sách `vim.ui.select`, chỉ chèn.
- Chỗ cần điền là `${table}`, `${column}`, `${pattern}`… — đúng cú pháp tham số ở trên: lúc chạy plugin hỏi giá trị (nhớ giá trị lần trước, `<Tab>` gợi ý), buffer vẫn giữ `${...}` để dùng lại.
- Query nằm trong server (mỗi driver một bộ, đúng ngôn ngữ native của nó), nên mọi phiên Neovim dùng chung.

**Xem cả cheatsheet trong một buffer:** `<leader>rK` (hoặc `:TradarCheatsheetFile`) mở **toàn bộ** cheatsheet của connection trong một tab — mỗi nhóm là một tiêu đề `## …`, mỗi query là `### tên` kèm một block ```` ```sql <connection> ````, giống một file [`.tdb`](File-tdb). Đọc từ trên xuống, `/` để tìm, `zM` gập lại chỉ còn tiêu đề nhóm và query, `zR` mở hết, `za` mở/đóng một mục; đặt con trỏ trong block rồi `rr` chạy luôn (`${tên}` vẫn được hỏi). Query của bạn (★) nằm cùng trong đó. Buffer là bản tạm: sửa thoải mái, `:w đường-dẫn.tdb` để giữ một bản; gọi lại lệnh thì quay về đúng buffer (chưa sửa gì thì được vẽ lại theo `cheatsheet.toml` mới). Trong workbench/`.http`/block `http`/`grpc`, lệnh này mở cheatsheet request tương ứng.

**Thêm query của riêng bạn:** `:TradarCheatsheetEdit` (hoặc `<C-e>` trong picker) mở `cheatsheet.toml` cạnh `connections.toml` (có template). Mỗi query là một bảng `[[entry]]`:

```toml
[[entry]]
driver = "postgres"       # tuỳ chọn: chỉ cho loại này; bỏ trống = mọi loại
connection = "prod"       # tuỳ chọn: chỉ cho connection này
category = "Đơn hàng"     # tuỳ chọn, mặc định "Mine"
title = "Đơn trong 1 giờ qua"
text = """
select * from orders where created_at > now() - interval '1 hour'
"""
```

Query của bạn hiện **trước** bộ có sẵn, đánh dấu `★`. File được đọc lại mỗi lần mở cheatsheet nên lưu là có hiệu lực, không cần restart; mục thiếu `title`/`text` hoặc file sai cú pháp chỉ báo cảnh báo, bộ có sẵn vẫn hiện. `${tên}` trong `text` cũng được hỏi lúc chạy.

Bộ nào cũng được test: SQLite chạy thật từng câu (kể cả ghi và DDL) trên database tạm; Mongo parse từng câu bằng shell của chính driver; Elasticsearch kiểm JSON body; các driver còn lại kiểm mỗi mục đúng một câu lệnh. Câu mẫu của Postgres, MySQL, ClickHouse, Cassandra chưa được chạy thử trên server thật — nếu câu nào lỗi trên phiên bản của bạn, báo để sửa.

## Tham số trong câu lệnh

`${tên}` hoặc `${tên=mặc-định}` trong câu lệnh sẽ được **hỏi lúc chạy**, giá trị được nhớ cho lần sau (làm giá trị điền sẵn):

```sql
select * from orders where id = ${id} and status = '${status=open}';
```

Nhiều câu dùng chung một tham số (`<leader>ra`) thì chỉ hỏi một lần cho cả lô. Hủy một câu hỏi là không chạy gì. Trong câu hỏi, `<Tab>` gợi ý các giá trị đã dùng cho tham số đó (8 giá trị gần nhất, mới nhất trước). Dòng chỉ có comment (`--`, `//`, `#`) được bỏ qua.

Đây là **thay chữ**, không phải tham số ràng buộc: `${id}` trong `where id = ${id}` là đúng những gì bạn gõ (số vào thẳng câu lệnh). Giá trị nằm giữa hai dấu nháy thì dấu nháy trong giá trị được escape (`O'Brien` an toàn trong `'${name}'` — SQL nhân đôi `''`, Mongo/ES/Redis dùng `\`). Câu lệnh sau khi thay mới được kiểm tra bởi cơ chế bảo vệ (`${where}` rỗng thành `DELETE` không có `WHERE` vẫn bị hỏi lại), và lưu vào history là bản đã thay; `:TradarSave` lưu bản có `${...}`.

## Làm mới và theo dõi

Trong buffer kết quả, `<leader>rR` chạy lại **đúng câu đã sinh ra kết quả đang xem**, ngay tại tab đó (con trỏ giữ nguyên, lỗi chỉ báo `refresh failed` và lưới cũ còn đó). `<leader>rW` / `:TradarWatch [giây]` lặp lại mỗi 5 giây (hoặc số giây bạn đưa vào) cho tới khi bấm lại, `:TradarWatch off`, gặp lỗi, hoặc bạn chuyển sang tab khác / có kết quả khác lên màn hình; footer ghi `· watching every 5s`. Dùng cho log, queue, `show processlist`.

Chỉ câu **không ghi dữ liệu** mới được lặp (`SELECT`, `find`/`aggregate` không `$out`/`$merge`, lệnh Redis chỉ đọc, `GET`/`_search` của Elasticsearch). `INSERT … RETURNING` bị từ chối: chạy lại từ buffer SQL một cách có chủ đích. Watch bỏ qua nhịp khi đang có query chạy hoặc lưới có sửa chưa lưu.

## Nhiều kết quả (tab theo bảng / collection)

Mỗi **đích** của câu lệnh có một tab kết quả riêng: bảng của `SELECT … FROM users` (câu có JOIN lấy bảng đầu tiên), collection của `db.orders.find(…)` / `db.getCollection("x")`, index của `GET /logs/_search`, key của `HGETALL user:1`. Chạy lại một câu cùng đích (đổi `WHERE`, thêm `LIMIT`…) thì **thay đúng tab đó**, các tab khác nằm yên. Câu không có đích rõ ràng (`EXPLAIN`, `SHOW`, `UPDATE`…) có tab riêng theo từng câu.

- Cửa sổ kết quả có **thanh tab** ở trên (khi có từ 2 tab): ` 1 users (120)  2 orders (8)  3 events `, tab đang xem được tô sáng, **click chuột** để chuyển.
- `]r` / `[r` sang tab sau / trước (đi vòng), `<leader>r1`…`<leader>r9` nhảy thẳng tới tab, `<leader>rx` đóng tab đang xem, `:TradarResults` mở danh sách chọn (dùng gần nhất trên cùng, `◀` đánh dấu tab đang xem).
- Mỗi tab là một buffer riêng (`tradar://results/users`), nhớ vị trí con trỏ; quay lại tab thì con trỏ ở chỗ cũ. Footer cho biết đang xem tab nào và câu nào sinh ra nó: `[2/3] select name from p · 3/3 rows · 1ms`.
- Giữ tối đa **10 tab** (`setup{ results_kept = 20 }` để đổi); đầy thì tab ít dùng nhất bị bỏ.
- Server chỉ giữ cursor mở cho **4 tab dùng gần nhất** (`setup{ results_cursors = 8 }` để đổi): các tab này vẫn tải thêm trang (`<leader>rm`) và sửa được. Tab cũ hơn vẫn xem được từ những gì đã tải, nhưng chỉ đọc — chạy lại câu lệnh để sửa/tải tiếp.
- **Lịch sử các lần chạy:** mỗi tab giữ thêm **5 lần chạy trước** của đích đó (`setup{ results_history = 10 }` để đổi). Trong tab, `[R` xem lần chạy trước, `]R` quay lại lần mới hơn; footer ghi `run 1 back of 2 (11:02:23)`. Lần chạy cũ chỉ đọc. Làm mới (`<leader>rR`, watch) thay tại chỗ và không thêm lịch sử.
- **Ghim tab:** `<leader>rp` ghim / bỏ ghim tab đang xem (thanh tab có 📌). Tab đã ghim không bị chạy lại cùng đích thay thế — lần chạy mới mở **tab riêng** — và là tab cuối cùng bị bỏ khi hết chỗ. Dùng để giữ lại một kết quả làm mốc so sánh.

## Lọc và sắp xếp kết quả

Làm trên các dòng **đã tải**, không chạy lại query và không tải thêm dòng:

- `<leader>rF` (hoặc `:TradarFilter {biểu thức}`; để trống trong câu hỏi, hoặc `:TradarFilter off`, để bỏ): `status = failed`, `age >= 30`, `name != bob`, `city ~ han` (chứa), hoặc chỉ gõ chữ để lấy dòng có ô nào chứa chữ đó. Không phân biệt hoa thường; số so sánh như số (9 < 25); ô `NULL` không khớp `<`, `>`.
- `<leader>ro`: sắp xếp theo cột dưới con trỏ — tăng, giảm, rồi về thứ tự của query. Sắp xếp ổn định, `NULL` luôn ở cuối.
- Footer ghi `filter «city = Hue»: 2 of 5 loaded · sorted by age ↑`. Trang tải thêm (`<leader>rm`) cũng được lọc/sắp xếp; làm mới giữ nguyên bộ lọc.
- **Hỏi server cho cả kết quả** (`<leader>rA`): `rF`/`ro` chỉ thấy các dòng đã tải, nên khi còn dòng chưa tải footer nhắc `rA asks the server, for all N rows`. Với kết quả SQL (Postgres, MySQL, SQLite, ClickHouse) `rA` bọc câu gốc thành `SELECT * FROM ( … ) AS tradar_q WHERE … ORDER BY …` rồi chạy vào **cùng tab** — lần chạy chưa lọc nằm trong lịch sử của tab (`[R`). Bộ lọc phải có dạng `cột = giá trị` (cũng `!=`, `>`, `<`, `>=`, `<=`, và `~` cho "chứa"); chữ trần không đổi được thành điều kiện SQL. So sánh theo collation của database (`=` có thể phân biệt hoa thường, khác với lọc trên lưới). Lọc/sắp xếp mới thay chứ không chồng lên lần trước; bấm `rA` khi không còn lọc/sắp xếp nào trên màn hình thì bỏ phần bọc, về lại câu gốc. Làm mới (`rR`, watch) lặp lại câu đã bọc. Chỉ với câu chỉ-đọc, và kết quả đó chỉ đọc (không sửa trực tiếp được). Mongo / Redis / Elasticsearch chưa hỗ trợ.
- Khi đang lọc hay sắp xếp, lưới **chỉ đọc** (dòng hiển thị không còn khớp dòng trong query); bỏ lọc để sửa. Copy ô/dòng (`gyc`, `gyr`…) lấy đúng các dòng đang hiện; export và `gyj`/`gyv`/`gym` vẫn lấy toàn bộ dòng.

## So sánh hai kết quả

`<leader>rD` (hoặc `:TradarDiff [n]`) mở một cửa sổ nhỏ liệt kê khác biệt theo **khoá** (khoá chính nếu sửa được, không thì cột đầu tiên): `~` ô đổi (`age: 30 → 31`), `+` dòng mới, `-` dòng mất, kèm số dòng giống nhau.

- Không tham số: lần chạy trước của tab so với lần mới nhất (đang xem lần cũ bằng `[R` thì so lần đó với lần mới nhất). Chưa có lần chạy trước thì chọn một tab khác.
- `:TradarDiff 2`: tab 2 (trước) so với tab đang xem (sau). Hai kết quả khác cột thì so các cột chung và ghi cột chỉ có ở một bên.
- Chỉ so các dòng đã tải; `q` đóng cửa sổ.

## Ô dài bị cắt

Mỗi cột rộng tối đa 40 ký tự (dài hơn thì cắt bằng `…`), riêng **cột cuối** được tới 400 ký tự — nên kết quả một cột như `EXPLAIN` hay `SHOW CREATE TABLE` hiện gần như đủ. Trong buffer kết quả, `zw` hiện trọn cột dưới con trỏ (tới 400 ký tự như cột cuối), bấm lại để thu về; `zW` cho mọi cột. Ô có xuống dòng được hiển thị trên một dòng (xuống dòng thành dấu cách); `K` trên ô xem trọn giá trị (giữ xuống dòng), `gK` xem cả dòng.
