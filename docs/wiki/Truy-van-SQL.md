# Truy vấn SQL trong Neovim

Áp dụng cho PostgreSQL, MySQL/MariaDB, SQLite, ClickHouse, Cassandra (CQL). Với Mongo/Redis/Elasticsearch xem [trang riêng](Mongo-Redis-Elasticsearch) — cách dùng giống nhau, khác ngôn ngữ.

Bắt đầu: tạo file `.sql`, thêm dòng đầu `-- tradar: tên-connection` (xem [Kết nối](Ket-noi)).

## Chạy câu lệnh

Tất cả phím có tiền tố `<leader>r` (đổi được bằng `setup{ prefix = ... }`).

| Phím | Việc |
|---|---|
| `rr` | chạy statement dưới con trỏ (visual mode: phần đang chọn) |
| `ra` | chạy cả file theo thứ tự, dừng ở lỗi đầu tiên |
| `rx` | huỷ query đang chạy |
| `re` | `EXPLAIN` statement dưới con trỏ (không bao giờ `ANALYZE`, vì nó *chạy* câu lệnh) |
| `rt` | picker bảng (telescope): `<CR>` chèn tên, `<C-o>` mở bảng |
| `rh` | history: `<CR>` dán, `<C-r>` chạy |
| `rs` | panel schema (cây schema → bảng → cột) |
| `rc` | đổi connection |
| `rm` | tải thêm dòng |
| `rv` | đổi bảng ↔ JSON (SQL: mảng object; tài liệu: JSON gốc) |

Ranh giới statement do chính driver quyết định. Chạy hoàn toàn async: có spinner và thời gian trên statusline, Neovim không bị đơ. Lệnh tương ứng: `:TradarRun`, `:TradarRunAll`, `:TradarCancel`, `:TradarMore`, `:TradarSchema`, `:TradarTables`, `:TradarHistory`.

`g?` trong buffer SQL, buffer kết quả hoặc panel schema hiện danh sách phím của buffer đó.

## Buffer kết quả

Là buffer Neovim thường (`j`/`k`/`gg`/`G`, `/`...). Cuộn tới cuối sẽ tự tải trang kế.

| Phím | Việc |
|---|---|
| `gK` | xem cả dòng hiện tại, mỗi trường một dòng |
| (mọi lệnh vim) | sửa thẳng trên lưới: `cw`, `dd`, `o`, `:s`, `:g`, `u`… — xem "Sửa trực tiếp" bên dưới |
| `:w` (hoặc `rw`) | xem trước toàn bộ câu lệnh rồi commit một lượt |
| `ru` | bỏ các sửa chưa commit, vẽ lại lưới |
| `gyc` / `gyr` / `gyC` | copy ô / dòng (visual: nhiều dòng) / cột |
| `gyj` / `gyv` / `gym` | copy cả kết quả dạng JSON / CSV / Markdown |
| `K` | xem trọn giá trị một ô |
| `gd` | trên ô của cột khoá ngoại: chạy `SELECT` dòng được tham chiếu |
| `q` | đóng |

Xuất file: `:TradarExport csv|json|md|tsv [file]`.

Cột rộng hơn 40 ký tự bị cắt và thêm `…`; dùng `K` hoặc `gyc` để lấy giá trị đầy đủ. Buffer kết quả không có sort/filter theo cột — sửa `ORDER BY`/`WHERE` rồi `rr` lại.

### Sửa trực tiếp

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
- `gd` trên tên bảng: mở bảng (`SELECT ... LIMIT 100`).
- Panel schema (`rs`): `<Tab>` gấp/mở bảng hoặc nhóm, `<CR>` chèn tên (hoặc gấp/mở nếu đang ở dòng nhóm).

Schema được cache 1 phút phía plugin và tự làm mới sau khi bạn chạy DDL (`CREATE`/`ALTER`/`DROP`/`RENAME`).

## Completion

Nguồn `blink.cmp` (`tradar.blink`, kích hoạt bằng `.`) hoặc `omnifunc` dự phòng. Hiểu ngữ cảnh: sau `alias.` chỉ gợi ý cột của bảng đó, sau `JOIN` ưu tiên bảng có khoá ngoại liên quan, sau `schema.`/`keyspace.` gợi ý bảng trong đó. Nếu cài `sqlls`, nên bỏ nguồn `lsp` cho `sql` trong cấu hình completion của bạn để khỏi trùng.

## Lỗi

Lỗi cú pháp được gạch chân đúng vị trí bằng `vim.diagnostic` (tự xoá ở lần sửa kế tiếp); nội dung lỗi đầy đủ mở trong buffer kết quả.

## Statusline

`require('tradar').status()` trả chuỗi như `db demo · 120/5000 rows · 83ms`: spinner khi đang chạy, `✗` khi ping nền (mỗi 15 giây) thấy connection rớt, `tx` khi đang trong transaction (cập nhật ngay sau `BEGIN`/`COMMIT`/`ROLLBACK`, không đợi lần ping kế), `⚠` với connection protected. Trong buffer kết quả, nó hiện connection **của kết quả** đó và thêm `✎ unsaved edits` khi lưới có sửa chưa commit. Rỗng ngoài buffer tradar, nên gắn vào lualine không tốn gì ở file khác.

## An toàn

Plugin hỏi xác nhận (đáp án mặc định là `Cancel`, nên `<CR>` phản xạ là an toàn) trước khi chạy:

- `UPDATE`/`DELETE` **không có `WHERE`**, `DROP`, `TRUNCATE` — trên mọi connection.
- **Mọi lệnh ghi** (`INSERT`/`UPDATE`/`DELETE`/DDL...) trên connection "protected": tên chứa một chuỗi trong `setup{ protected = {...} }` (mặc định `{"prod"}`, không phân biệt hoa thường).
- Mongo: `drop()`, `dropDatabase()`, `deleteMany({})`/`updateMany({}, …)` (bộ lọc rỗng). Redis: `FLUSHALL`/`FLUSHDB`/`SHUTDOWN`. Elasticsearch: `DELETE …`, `_delete_by_query`.
- `ra` (chạy cả file) chỉ hỏi **một lần** cho cả lô.

Đây là phân tích theo token (bỏ qua comment và chuỗi), không phải parser — nó bắt các lỗi gõ tay thường gặp chứ **không phải** rào chắn bảo mật. `setup{ confirm = false }` tắt hết.

## Huỷ query

`rx` huỷ phía client ngay; với **PostgreSQL và SQLite** còn huỷ thật phía database. Các database khác chỉ bỏ phần chờ (câu lệnh có thể vẫn chạy nốt ở phía server).
