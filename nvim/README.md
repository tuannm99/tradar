# tradar.nvim

Chạy query trên database ngay trong Neovim. Editor là buffer Neovim bình thường; `tradar-server` (một process chạy nền) giữ connection. Thiết kế: mục "Server headless" trong `docs/architecture.md`.

## Cài

```bash
cargo build --release -p tradar-server        # trong repo tradar
```

**Đang dev tradar (khuyên dùng lúc này)** — `dir =` trỏ thẳng vào checkout, Lua và `tradar-server` luôn khớp vì cùng một checkout; sửa gì chỉ cần restart Neovim (hoặc `:TradarRestart` riêng cho server), không qua bước git nào:

```lua
{ dir = "~/dev/local/tradar/nvim", name = "tradar.nvim", lazy = false,
  dependencies = { "nvim-telescope/telescope.nvim" },
  config = function() require("tradar").setup {} end }
```

`lazy = false`, không phải `ft = "sql"` — `.mongo`/`.redis`/`.esq` chỉ trở thành filetype thật sau khi `plugin/tradar.lua` tự nó chạy (`vim.filetype.add`), nên lazy.nvim's `ft` trigger không bao giờ nhận ra 3 filetype đó để mà load trước (gà-và-trứng); `dir=` với một plugin nhẹ (chỉ đăng ký autocmd/command, không có gì nặng lúc startup) thì `lazy = false` không đáng lo.

**Khi tradar ổn định hơn (chưa đến lúc này)** — cài như một plugin GitHub bình thường, `nvim/` vẫn là thư mục con của repo Rust nên cần `build` để compile server và `config` để trỏ `rtp` vào đúng `nvim/`:

```lua
{ "tuannm99/tradar", name = "tradar.nvim", lazy = false,
  build = "cargo build --release -p tradar-server",
  dependencies = { "nvim-telescope/telescope.nvim" },
  config = function(plugin)
    vim.opt.rtp:prepend(plugin.dir .. "/nvim")
    require("tradar").setup {}
  end }
```

Đánh đổi so với `dir=`: mỗi lần sửa tradar phải `git push` rồi `:Lazy update` mới thấy, không còn vòng lặp sửa-restart-thử ngay — chỉ đáng khi code đã đứng, không còn sửa mỗi ngày.

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

### Định dạng `.tdb` (nhiều dialect, nhiều connection, một file)

Thay vì chia theo filetype (`.sql`/`.mongo`/`.redis`/`.esq`) — bất tiện khi muốn so sánh/ghi chú nhiều truy vấn khác dialect cạnh nhau — một file `.tdb` gồm nhiều block kiểu fenced code của Markdown, mỗi block tự chọn dialect và (tuỳ chọn) connection riêng:

````
# ghi chú thoải mái ở ngoài block, không chạy được

```sql pg-local
SELECT * FROM users LIMIT 10;
```

```mongo atlas-dev
db.users.find({active: true})
```

```redis
GET session:123
```
````

Dòng mở fence là `` ```<dialect> [connection] `` — thiếu `connection` thì block đó rơi về connection của buffer (modeline `-- tradar: name`/`.tradar`/`:TradarConnect`, giống file thường). `<leader>rr`/`K`/`gd`/`<leader>re`/completion đều áp dụng cho **block đang chứa con trỏ**, không phải cả buffer — đặt con trỏ ngoài mọi block thì `<leader>rr` báo rõ "không ở trong block nào" thay vì chạy nhầm. `<leader>ra` (chạy cả file)/`<leader>rx` (huỷ)/visual-mode `<leader>rr` chưa áp dụng cách này — vẫn theo connection của buffer như trước.

Tô màu: `.tdb` dùng chung parser treesitter của Markdown (fenced code block tự inject theo tag — `sql` là grammar thật nên tô đúng cú pháp SQL; `mongo`/`esq` tô theo JavaScript/JSON qua cùng alias plugin đã đăng ký cho filetype `.mongo`/`.esq` riêng; `redis` không có grammar nào khớp nên hiện plain, giống hệt file `.redis` hiện tại). Suggestion (completion bảng/cột) theo đúng dialect của block, không phải LSP thật — LSP thật theo từng block (qua [otter.nvim](https://github.com/jmbuhr/otter.nvim)) để dành cho sau.

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
