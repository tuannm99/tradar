# `:TradarConnections` + rà UX Kafka/RabbitMQ/HTTP/Socket

## Đã làm

- **`:TradarConnections`** (`M.edit_connections`): mở đúng `connections.toml` server đang đọc. Trước đây plugin không có đường nào mở file này (phải biết đường dẫn, vốn khác nhau theo OS). Server thêm `connections.path` → `{path}` (minor bump `PROTOCOL_VERSION` lên `1.1`; plugin fallback `stdpath('config')/../tradar/` với server cũ). File chưa có thì buffer có sẵn template comment. Mỗi lần `:w` plugin gọi `connections.list` để báo số connection hoặc lỗi TOML (server vốn đọc lại file mỗi lần gọi). Picker `:TradarConnect` có thêm dòng `+ edit connections file…`; các thông báo "no saved connection…" của Kafka/RabbitMQ/HTTP/Socket/gRPC chỉ tới lệnh này.
- **Bug Socket tail**: `append` đặt `modifiable=false` sau tin đầu tiên nên không gõ được dòng để gửi. Nay dòng cuối là dòng nhập kiểu REPL: tin gửi/nhận chèn *phía trên*, `<CR>` (normal + insert) gửi và xoá dòng nhập.
- **Tail buffer (Kafka + Socket)** dùng chung `tradar/tail.lua`: con trỏ chỉ bám theo khi đang ở dòng cuối (trước đây luôn giật xuống đáy, không cuộn lên đọc được) và giới hạn 5000 dòng (trước đây tăng vô hạn).

- **Yêu cầu Neovim ≥ 0.10** (plugin dùng `vim.keycode`/`vim.islist`) nay ghi trong README và `:checkhealth tradar`.

## Rà, chưa làm (xếp theo giá trị)

1. **HTTP**: 4 `vim.ui.input` một dòng liên tiếp (method → URL → headers `;` → body) — không sửa được body nhiều dòng, không gửi lại/sửa nhẹ lần trước. Hướng đúng: buffer request sửa được (dòng đầu `GET url`, các dòng `Header: v`, dòng trống, body), `<leader>rr` gửi — cùng kiểu buffer request của gRPC.
2. **RabbitMQ publish**: cùng vấn đề (payload một dòng); peek/bindings chỉ là snapshot, không refresh.
3. **Kafka tail**: chưa có lọc/clear/tạm dừng; tail không tự nối lại nếu server khởi động lại.

## Kiểm chứng

Smoke test headless Neovim với `tradar-server` thật: mở/ghi `connections.toml` (template, báo số connection, báo lỗi TOML); Socket tail với echo server TCP thật (nhận, gõ+gửi, echo, dòng nhập được xoá); `tail.append` (cap 5000, read-only giữ nguyên, không bám khi cuộn lên, bám khi ở đáy). `cargo test -p tradar-server --lib` (thêm test `connections.path`).
