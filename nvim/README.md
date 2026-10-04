# tradar.nvim

Chạy query trên database ngay trong Neovim. Editor là buffer Neovim bình thường; `tradar-server` (một process chạy nền) giữ connection. Thiết kế: mục "Server headless" trong `docs/architecture.md`.

## Cài

```bash
cargo build --release -p tradar-server        # trong repo tradar
```

Với `lazy.nvim`:

```lua
{ dir = "~/dev/local/tradar/nvim", ft = "sql", dependencies = { "nvim-telescope/telescope.nvim" },
  config = function() require("tradar").setup {} end }
```

`:checkhealth tradar` kiểm tra binary, server, connection đã lưu. Server chạy nền và giữ nguyên bản binary cũ cho tới khi dừng: sau khi build lại, chạy `:TradarRestart` (lệnh kế tiếp tự khởi động bản mới).

## Dùng

Connection lấy từ `~/.config/tradar/connections.toml` (cùng file với TUI). Gắn một file `.sql` vào connection bằng dòng đầu file:

```sql
-- tradar: tên-connection
```

hoặc file `.tradar` (một dòng, tên connection) ở thư mục cha, hoặc `:TradarConnect`.

### Mongo, Redis, Elasticsearch

Dùng file `.mongo` (filetype `mongo`, tô màu như JavaScript), `.redis`, `.esq` (Elasticsearch, kiểu `GET /index/_search` + body JSON), hoặc **bất kỳ file nào** có dòng liên kết ở đầu — dấu comment tuỳ ngôn ngữ:

```
// tradar: tên-connection     (Mongo, JS)
# tradar: tên-connection      (Redis / Elasticsearch)
-- tradar: tên-connection     (SQL)
```

Mongo nhận cú pháp mongosh (`{name: 'ann'}`, `ObjectId("…")`), không chỉ JSON chặt. Kết quả dạng tài liệu hiện **bảng** (cột phẳng `address.city`) để dùng `i`/`dd`/`gyc`/export như SQL; `gT` đổi sang JSON. `<leader>re` (EXPLAIN) chỉ có cho SQL.

| Phím (tiền tố `<leader>r`) | Việc |
|---|---|
| `rr` | chạy statement dưới con trỏ (visual: phần chọn) |
| `ra` / `rx` | chạy cả file / huỷ query đang chạy |
| `rt` / `rh` | telescope: bảng (`<CR>` chèn tên, `<C-o>` mở bảng) / history (`<CR>` dán, `<C-r>` chạy) |
| `re` | `EXPLAIN` statement dưới con trỏ (không bao giờ `ANALYZE`, vì nó *chạy* câu lệnh) |
| `rs` / `rc` / `rm` | panel schema / chọn connection / tải thêm dòng |
| `K` | thông tin bảng/cột dưới con trỏ (kiểu, PK, FK, index; hiểu alias `o.user_id`); không biết thì rơi về hover của LSP |
| `gd` | trên tên bảng: mở bảng (`SELECT ... LIMIT 100`); chỗ khác: `definition` của LSP |

Trong buffer kết quả: `i` sửa ô (nhập giá trị — gõ `NULL` để đặt NULL — rồi hiện câu `UPDATE` để bạn xác nhận), `dd` xoá dòng (hiện câu `DELETE` rồi xác nhận); sau khi chạy, kết quả tự làm mới và con trỏ ở nguyên dòng. Chỉ sửa được kết quả của truy vấn một bảng có khoá chính và có chọn cột khoá, nếu không sẽ báo lý do. `gd` trên ô của cột khoá ngoại chạy `SELECT` dòng được tham chiếu (kết quả của truy vấn một bảng), `gyc` ô, `gyr` dòng, `gyC` cột, `gyj`/`gyv`/`gym` cả kết quả (JSON/CSV/Markdown), `:TradarExport csv|json|md|tsv [file]`, `q` đóng. Cuộn tới cuối tự tải trang kế.

Statusline: `require('tradar').status()` (chuỗi rỗng ngoài buffer SQL).
Completion: nguồn `blink.cmp` `tradar.blink` (hoặc `omnifunc`).

## An toàn

Hỏi xác nhận (mặc định là `Cancel`, nên `<CR>` phản xạ là đáp án an toàn) trước khi chạy:

- `UPDATE`/`DELETE` **không có `WHERE`**, `DROP`, `TRUNCATE` — trên mọi connection.
- **Mọi lệnh ghi** (`INSERT`/`UPDATE`/`DELETE`/DDL...) trên connection "protected" — tên chứa một trong `setup{ protected = {"prod"} }` (mặc định `{"prod"}`, không phân biệt hoa thường). Statusline hiện `⚠` bên cạnh.
- Mongo: `drop()`, `dropDatabase()`, `deleteMany({})`/`updateMany({}, …)` (bộ lọc rỗng) luôn hỏi. Redis: `FLUSHALL`/`FLUSHDB`/`SHUTDOWN`. Elasticsearch: `DELETE …`, `_delete_by_query`. Trên connection protected: mọi lệnh ghi của ngôn ngữ đó.
- `<leader>ra` chạy cả file chỉ hỏi **một lần** cho cả lô.
- `setup{ confirm = false }` tắt hết.

Đây là phân tích theo token (bỏ qua comment và chuỗi), không phải parser: bắt các lỗi tay thường gặp, không phải rào chắn bảo mật.
