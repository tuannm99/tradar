# HTTP: buffer request sửa được

Từ mục 1 của đợt rà UX (`nvim-connections-tail-fixes-2026-10-20.md`).

- `:TradarHttpSend` trước đây hỏi lần lượt method/URL/headers/body bằng `vim.ui.input` một dòng: không gõ được body nhiều dòng, headers phải dồn một dòng bằng `;`, mỗi lần gửi lại phải nhập lại từ đầu.
- Nay mở **buffer request** (`tradar-http://<conn>`, `filetype=http`): dòng đầu `METHOD url`, các dòng `Key: Value`, dòng trống, body; `#` trước dòng trống là comment. `M.parse(lines)` là hàm thuần (có kiểm tra bằng smoke test). `<leader>rr`/`:TradarHttpSend` trong buffer gửi qua `http.send` — server không đổi.
- Mỗi connection một buffer, giữ trong phiên Neovim; response vẫn vào split `tradar://http`. Không chiếm `q` (buffer sửa được, như request buffer gRPC).
- Kiểm chứng: smoke test headless Neovim + `tradar-server` thật + HTTP server Python: parse (method/URL/headers/body nhiều dòng, URL trần = GET, rỗng → lỗi), POST thật tới server (path, header, body nhiều dòng), mở lại dùng lại buffer đã sửa.
