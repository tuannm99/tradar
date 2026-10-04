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

`:checkhealth tradar` kiểm tra binary, server, connection đã lưu.

## Dùng

Connection lấy từ `~/.config/tradar/connections.toml` (cùng file với TUI). Gắn một file `.sql` vào connection bằng dòng đầu file:

```sql
-- tradar: tên-connection
```

hoặc file `.tradar` (một dòng, tên connection) ở thư mục cha, hoặc `:TradarConnect`.

| Phím (tiền tố `<leader>r`) | Việc |
|---|---|
| `rr` | chạy statement dưới con trỏ (visual: phần chọn) |
| `ra` / `rx` | chạy cả file / huỷ query đang chạy |
| `rt` / `rh` | telescope: bảng (`<CR>` chèn tên, `<C-o>` mở bảng) / history (`<CR>` dán, `<C-r>` chạy) |
| `rs` / `rc` / `rm` | panel schema / chọn connection / tải thêm dòng |

Trong buffer kết quả: `gyc` ô, `gyr` dòng, `gyC` cột, `gyj`/`gyv`/`gym` cả kết quả (JSON/CSV/Markdown), `:TradarExport csv|json|md|tsv [file]`, `q` đóng. Cuộn tới cuối tự tải trang kế.

Statusline: `require('tradar').status()` (chuỗi rỗng ngoài buffer SQL).
Completion: nguồn `blink.cmp` `tradar.blink` (hoặc `omnifunc`).
