# Cài đặt

Cần Rust (edition 2024) và Neovim ≥ 0.10.

## 1. Build `tradar-server`

```bash
cargo build --release -p tradar-server     # chạy trong repo tradar
```

Mặc định đã gồm PostgreSQL, MySQL/MariaDB, SQLite, MongoDB, Elasticsearch, Redis, Cassandra, ClickHouse, gRPC, Socket, RabbitMQ, HTTP. **Kafka không nằm trong mặc định** vì cần `cmake`, `gcc`, `libcurl-dev` để build `librdkafka`:

```bash
cargo build --release -p tradar-server --features kafka
```

Chỉ cần vài loại? Build gọn hơn: `cargo build --release -p tradar-server --no-default-features --features postgres,sqlite`.

## 2. Thêm plugin (lazy.nvim)

**Đang dev tradar** — trỏ thẳng vào checkout, Lua và `tradar-server` luôn khớp nhau:

```lua
{ dir = "~/dev/local/tradar/nvim", name = "tradar.nvim", lazy = false,
  dependencies = { "nvim-telescope/telescope.nvim" },
  config = function() require("tradar").setup {} end }
```

Dùng `lazy = false` (không phải `ft = "sql"`): `.mongo`/`.redis`/`.esq` chỉ thành filetype sau khi plugin tự chạy, nên trigger `ft` không bao giờ nhận ra chúng.

**Cài như plugin GitHub thường:**

```lua
{ "tuannm99/tradar", name = "tradar.nvim", lazy = false,
  build = "cargo build --release -p tradar-server",
  dependencies = { "nvim-telescope/telescope.nvim" },
  config = function(plugin)
    vim.opt.rtp:prepend(plugin.dir .. "/nvim")
    require("tradar").setup {}
  end }
```

Đánh đổi: mỗi lần sửa tradar phải push rồi `:Lazy update` mới thấy — chỉ đáng khi code đã ổn định.

## 3. Kiểm tra

```vim
:checkhealth tradar
```

Kiểm tra binary `tradar-server`, server có chạy không, phiên bản protocol có khớp plugin không, số connection đã lưu, và các plugin tuỳ chọn (telescope, blink.cmp).

Plugin tự tìm `tradar-server` theo thứ tự: tuỳ chọn `server_cmd` → `PATH` → `<repo>/target/release` → `<repo>/target/debug`. Server tự khởi động ở lần dùng đầu tiên.

## Sau khi build lại server

Server chạy nền và giữ nguyên bản binary cũ cho tới khi dừng. Sau khi build lại:

```vim
:TradarRestart
```

Lệnh kế tiếp sẽ tự khởi động bản mới.

## Tuỳ chọn `setup{}`

| Tuỳ chọn | Ý nghĩa |
|---|---|
| `socket` | đường dẫn unix socket của server (mặc định `$XDG_RUNTIME_DIR/tradar/server.sock`) |
| `server_cmd` | đường dẫn tới binary `tradar-server` |
| `autostart = false` | không tự khởi động server |
| `protected = { "prod" }` | tên connection chứa chuỗi này (không phân biệt hoa thường) bị coi là "protected" — xem [An toàn](Truy-van-SQL#an-toàn) |
| `confirm = false` | tắt mọi hộp xác nhận an toàn |
| `prefix` | tiền tố phím (mặc định `<leader>r`) |
| `keymaps = false` | không đăng ký phím nào |
| `documents_view` | `"table"` hoặc `"json"` — ép cách hiện kết quả dạng tài liệu |
| `page_size = 100` | số dòng mỗi trang kết quả (trang kế tải bằng `<leader>rm`) |
| `auto_load_more = true` | tự tải trang kế khi con trỏ xuống cuối (mặc định tắt) |

Plugin tuỳ chọn: `nvim-telescope/telescope.nvim` (picker bảng/history), `blink.cmp` (completion), `which-key.nvim` (nhóm phím).
