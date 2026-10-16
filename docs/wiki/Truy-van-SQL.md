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
| `rv` | (kết quả dạng tài liệu) đổi bảng ↔ JSON |

Ranh giới statement do chính driver quyết định. Chạy hoàn toàn async: có spinner và thời gian trên statusline, Neovim không bị đơ. Lệnh tương ứng: `:TradarRun`, `:TradarRunAll`, `:TradarCancel`, `:TradarMore`, `:TradarSchema`, `:TradarTables`, `:TradarHistory`.

`g?` trong buffer SQL, buffer kết quả hoặc panel schema hiện danh sách phím của buffer đó.

## Buffer kết quả

Là buffer Neovim thường (`j`/`k`/`gg`/`G`, `/`...). Cuộn tới cuối sẽ tự tải trang kế.

| Phím | Việc |
|---|---|
| `i` | sửa ô (gõ `NULL` để đặt NULL) |
| `dd` | xoá dòng |
| `gyc` / `gyr` / `gyC` | copy ô / dòng (visual: nhiều dòng) / cột |
| `gyj` / `gyv` / `gym` | copy cả kết quả dạng JSON / CSV / Markdown |
| `K` | xem trọn giá trị một ô |
| `gd` | trên ô của cột khoá ngoại: chạy `SELECT` dòng được tham chiếu |
| `q` | đóng |

Xuất file: `:TradarExport csv|json|md|tsv [file]`.

Cột rộng hơn 40 ký tự bị cắt và thêm `…`; dùng `K` hoặc `gyc` để lấy giá trị đầy đủ. Buffer kết quả không có sort/filter theo cột — sửa `ORDER BY`/`WHERE` rồi `rr` lại.

### Sửa ô / xoá dòng

`i` và `dd` **không chạy ngay**: plugin sinh câu `UPDATE`/`DELETE`, hiện cho bạn xem, chỉ chạy sau khi bạn chọn `Run`; xong tự chạy lại truy vấn gốc và giữ nguyên dòng đang đứng. Chỉ sửa được khi kết quả đến từ **một bảng có khoá chính** và cột khoá có trong kết quả; nếu không, plugin báo rõ lý do (không biết bảng nguồn, bảng không có khoá chính, cột khoá không có trong kết quả, hoặc giá trị khoá là NULL).

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

`require('tradar').status()` trả chuỗi như `db demo · 120/5000 rows · 83ms`: spinner khi đang chạy, `✗` khi ping nền (mỗi 15 giây) thấy connection rớt, `tx` khi đang trong transaction, `⚠` với connection protected. Rỗng ngoài buffer tradar, nên gắn vào lualine không tốn gì ở file khác.

## An toàn

Plugin hỏi xác nhận (đáp án mặc định là `Cancel`, nên `<CR>` phản xạ là an toàn) trước khi chạy:

- `UPDATE`/`DELETE` **không có `WHERE`**, `DROP`, `TRUNCATE` — trên mọi connection.
- **Mọi lệnh ghi** (`INSERT`/`UPDATE`/`DELETE`/DDL...) trên connection "protected": tên chứa một chuỗi trong `setup{ protected = {...} }` (mặc định `{"prod"}`, không phân biệt hoa thường).
- Mongo: `drop()`, `dropDatabase()`, `deleteMany({})`/`updateMany({}, …)` (bộ lọc rỗng). Redis: `FLUSHALL`/`FLUSHDB`/`SHUTDOWN`. Elasticsearch: `DELETE …`, `_delete_by_query`.
- `ra` (chạy cả file) chỉ hỏi **một lần** cho cả lô.

Đây là phân tích theo token (bỏ qua comment và chuỗi), không phải parser — nó bắt các lỗi gõ tay thường gặp chứ **không phải** rào chắn bảo mật. `setup{ confirm = false }` tắt hết.

## Huỷ query

`rx` huỷ phía client ngay; với **PostgreSQL và SQLite** còn huỷ thật phía database. Các database khác chỉ bỏ phần chờ (câu lệnh có thể vẫn chạy nốt ở phía server).
