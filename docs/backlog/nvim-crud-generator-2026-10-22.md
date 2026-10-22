# Sinh câu lệnh CRUD trong Neovim (2026-10-22)

User: trong Neovim chưa thấy chỗ nào sinh query CRUD cho một collection.

Đúng vậy: server đã có từ lâu (`snippet` nhận `op` = create/read/update/delete và `columns`; mọi `QueryDriver` có `crud_snippet`; TUI từng có column picker — `crud-snippet-column-picker.md`), nhưng plugin chỉ gọi `op = 'read'` trong `open_table` (chạy ngay). Không cần đổi server hay protocol.

- `lua/tradar/crud.lua`: `M.crud(buf, entry, op, columns)` + `:TradarCrud [op]`. Bảng = bảng dưới con trỏ (`nav.table_at_cursor`, dùng lại `word_under_cursor`/`find_table` của `gd`), không có thì `vim.ui.select` chọn trong schema. Thao tác hỏi nếu chưa nêu. Cột: "tất cả / mặc định của driver" hoặc "Pick columns…" (lặp `vim.ui.select` bật/tắt, `Done`); delete không hỏi cột (khoá lấy từ PK/`_id`). Cột trả về theo thứ tự bảng, không theo thứ tự bấm.
- Câu lệnh được **chèn** dưới dòng con trỏ (dòng trống hai bên), con trỏ nhảy tới dòng đầu, **không chạy** — khác `open_table` (SELECT chạy ngay) vì ba trong bốn thao tác ghi dữ liệu; chạy bằng `<leader>rr`, qua `guard` như câu thường.
- Lối vào: `:TradarCrud` (và `:Tradar crud`), `<leader>rg`, `c` trong panel schema, `<C-g>` trong picker `rt`. Dùng `vim.ui.select` nên telescope-ui-select/dressing đổi giao diện được.
- Mongo/ES/Redis/Cassandra dùng đúng `crud_snippet` của connector (test của từng connector đã có); spec Neovim chạy trên SQLite.

Test: `crud_spec` (19 kiểm tra: cạnh con trỏ, chọn cột, thứ tự, không chạy, update/delete, chọn bảng, huỷ, tên sai, completion, panel schema).
