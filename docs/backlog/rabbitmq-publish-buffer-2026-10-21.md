# RabbitMQ: buffer publish sửa được + refresh peek/bindings

Mục 2 của đợt rà UX (`nvim-connections-tail-fixes-2026-10-20.md`).

- `:TradarRabbitmqPublish`: thay chuỗi 3 `vim.ui.input` bằng buffer sửa được (`exchange:` / `routing-key:` / dòng trống / payload nhiều dòng), `M.parse_publish` là hàm thuần; `<leader>rr` hoặc lệnh trong buffer để publish. Mỗi connection một buffer, giữ trong phiên. Server không đổi.
- Split peek/bindings: phím `r` chạy lại đúng thao tác vừa làm (peek lại queue, liệt kê lại bindings) — peek vốn là snapshot nên trước đây phải gõ lại cả lệnh.
- Không đổi server: publish vào default exchange với `exchange = ""` (URL `.../%2F//publish`) đã được test TUI kiểm chứng trên RabbitMQ thật, không cần ánh xạ sang `amq.default`.
- Kiểm chứng: smoke test headless Neovim + `tradar-server` thật + mock Management API (Python): parse, peek → `r` ra kết quả mới, publish payload nhiều dòng tới đúng exchange/routing key. Chưa chạy lại trên RabbitMQ thật (Docker Hub trả 429 khi kéo image trong phiên này).
