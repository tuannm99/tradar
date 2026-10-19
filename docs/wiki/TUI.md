# TUI

> **TUI đã deprecated**: không phát triển thêm và không sửa chủ động (chỉ làm khi có người yêu cầu). Dùng plugin Neovim + `tradar-server`. Chạy `tradar` sẽ in cảnh báo này ra stderr. Trang này chỉ nêu những gì cần để dùng.

Build và chạy:

```bash
cargo run              # hoặc: cargo build rồi chạy binary `tradar`
tradar --help          # tradar --version
```

Mặc định dựng đủ 12 connector. Muốn nhẹ hơn: `cargo build -p tradar-app --no-default-features --features mongo,elasticsearch` (hoặc `make build-slim FEATURES=mongo,elasticsearch`). Form chọn driver tự chỉ liệt kê những connector đã compile vào.

**Bấm `?` trong app** để xem toàn bộ lệnh và phím hiện hành của màn hình đang đứng.

## Connection picker

`a` thêm, `e` sửa, `d` xoá (hỏi xác nhận). Form có `name`, `driver` (chọn bằng `←`/`→`), `target` (kèm dòng ví dụ định dạng mờ bên dưới); `Tab` chuyển field, `Enter` lưu, `Esc` huỷ. Thay đổi ghi thẳng vào `connections.toml` ([Kết nối](Ket-noi)).

## Tab và navigator

Nhiều connection mở cùng lúc, mỗi cái một tab: `Ctrl+T` tab mới, `Ctrl+W` đóng, `Ctrl+Left`/`Ctrl+Right` (hoặc `Ctrl+H`/`Ctrl+L`) chuyển tab, `Ctrl+1`…`Ctrl+9` nhảy tới tab theo số. App nhớ các tab đang kết nối qua lần khởi động sau.

`Ctrl+Q` thoát. `Ctrl+N` mở **navigator**: cây `connection → (schema) → bảng → cột` của *mọi* connection đã lưu; `Enter` trên tên chèn vào editor của connection đó, `/` lọc.

## Query screen (8 backend có ngôn ngữ query)

- `F5` hoặc `Ctrl+Enter` chạy câu lệnh, `Ctrl+A` chạy tất cả, `Ctrl+C` huỷ; `Tab` đổi focus editor ↔ kết quả. `Ctrl+R` history, `Ctrl+O`/`Ctrl+S` mở/lưu file query, `Ctrl+K` lưu snippet, `F7` mở thư viện snippet, `Esc` về picker.
- Kết quả: `Enter`/`d` sửa cell/xoá dòng (hiện câu lệnh rồi chạy sau khi bấm `y`), `s` sort theo cột, `/` filter (cú pháp `cột:giá_trị`, `AND`/`OR`; `F3` xem điều kiện), `<`/`>` resize cột, `Ctrl+E` export CSV/JSON.
- `F2` đổi mode (Redis: Browse ↔ Console; Kafka/RabbitMQ: Topics/Queues ↔ Groups/Exchanges), `F4` ERD của bảng, `F1` panel migration (chỉ Postgres), `F8` commit / `F9` rollback transaction đang mở (Postgres/MySQL/SQLite; mở transaction bằng cách gõ `BEGIN`), `F6` đổi layout ngang/dọc, `Ctrl+↑`/`Ctrl+↓` zoom editor/kết quả.
- Elasticsearch: `Ctrl+Y` ghi request ra `./tradar-query.sh` dạng `curl`, `Ctrl+G` copy `curl` vào clipboard.
- Một badge đỏ "disconnected" xuất hiện khi ping nền (15 giây một lần) thấy connection rớt.

Query editor mặc định là textbox thường (gõ là chèn, `Ctrl+Z`/`Ctrl+J` undo/redo). Bật chế độ vim-modal bằng `vim-mode = true` (xem dưới).

## Màn hình riêng

- **Kafka / RabbitMQ**: sidebar topic/queue bên trái; `Enter` mở (Kafka: tail realtime, `b` từ earliest, `Space` pause), `p` publish, `r` refresh, `/` lọc sidebar, `Tab` chuyển focus sang bảng chi tiết.
- **HTTP**: form Method/URL/Headers/Body + Response. `Tab`/`Shift+Tab` đổi field, `Ctrl+P`/`F3` đổi method (trước/sau), `Ctrl+Enter`/`F5` gửi, `Ctrl+K` lưu request, `F7` mở thư viện request đã lưu.
- **Socket**: transcript cuộn được + một dòng input luôn gõ được; `Enter` gửi, `F2` bật/tắt tự thêm `\n`, `Ctrl+R` reconnect, cuộn bằng mũi tên hoặc `Ctrl+D`/`Ctrl+U`.

## Cấu hình

Tuỳ chọn, tại `~/.config/tradar/config.toml`. Không có file thì dùng mặc định; file hỏng thì in cảnh báo ra stderr và chạy tiếp bằng mặc định.

```toml
[theme]
preset = "dracula"             # "dracula" hoặc "nord"; bỏ trống dùng mặc định
border-focused = "#89b4fa"     # override từng vai trò: tên màu, hex, hoặc chỉ số 256-color
error = "red"

[keymap.global]
new-tab = "ctrl-g"             # một phím

[keymap.list]
move-down = ["j", "down"]      # nhiều phím cho một lệnh
move-top = "gg"                # chuỗi 2 phím
close-tab = []                 # rỗng = gỡ phím

[editor]
vim-mode = true                # mặc định false; chỉ áp dụng sau khi khởi động lại
```

Các context remap được: `global`, `picker`, `query-screen`, `navigator`, `results`, `list`, `prompt`, `completion`, `editor`, và (khi `vim-mode = true`) `vim-normal`, `vim-visual`, `vim-motion`. Bấm `?` để xem lệnh nào thuộc context nào.

Với `vim-mode = true`, editor là vim-modal (`Normal`/`Insert`/`Visual`/`VisualLine`): `u` undo, **`U` redo** (không phải `Ctrl-R` — phím đó đã là mở history). `Esc` thoát Insert/Visual về Normal, bấm lần nữa mới về picker.
