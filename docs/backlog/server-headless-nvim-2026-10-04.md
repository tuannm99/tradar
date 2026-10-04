# Server headless + plugin Neovim (giai đoạn 1) — 2026-10-04

Xuất phát từ một nhận xét của user: query editor + navigator + cả bộ vim keymap tự viết trong TUI vừa khó dùng vừa tốn công bảo trì (đang làm lại một phần Neovim). Hướng đi đã chốt: tách phần "chạy query" thành một **server local** (`tradar-server`), dùng Neovim làm UI qua một plugin Lua mỏng. Nhiều session Neovim dùng chung được một server (chung connection đang mở, chung schema).

**Đã chốt với user:** TUI hiện tại **giữ nguyên chạy song song** (không xoá, không đóng băng ở giai đoạn này); quyết định bỏ/đóng băng TUI để sau khi plugin dùng ổn.

## Đã làm (giai đoạn 1)

- `crates/tradar-server` (crate thứ 17): lib + binary `tradar-server`. Chỉ phục vụ các connector có `QueryDriver` (Postgres, MySQL, SQLite, Mongo, Elasticsearch, Redis, Cassandra, ClickHouse — cùng tên feature như `tradar-app`). Kafka/RabbitMQ/HTTP/Socket chưa — xem `docs/roadmap.md`.
- Hai thay đổi nhỏ ở crate cũ, không đổi hành vi TUI: `Session::as_any()` (mặc định `None`, `tradar-connector-spi`) và `QueryEngine::driver()` (`tradar-query-workbench`). Lý do: `Connector::connect` trả `Box<dyn Session>` đóng kín, server cần lấy `Arc<dyn QueryDriver>` ra mà không bắt `tradar-connector-spi` biết kiểu `QueryEngine`.
- Protocol: JSON-RPC 2.0 (không batch, không notification) theo từng dòng (newline-delimited) qua unix socket. Method: `connectors.list`, `connections.list`, `connect`, `disconnect`, `status`, `schema`, `execute`, `fetch`, `cursor.close`, `split`, `keywords`, `edit.source`, `edit.sql`. Chi tiết trong `docs/architecture.md`, mục "Server headless".
- `nvim/` (plugin Lua): `:TradarConnect`, `:TradarRun` (visual / statement dưới con trỏ), `:TradarRunAll`, `:TradarMore`, `:TradarSchema`, `omnifunc`.
- Test: `crates/tradar-server/tests/rpc.rs` (8 test, SQLite thật, gồm một test qua unix socket thật + quyền `0600`); smoke test headless Neovim ↔ server thật chạy tay (connect, paging, `:TradarMore`, statement dưới con trỏ, navigator, omnifunc).

## Quyết định thiết kế và lý do

- **JSON-RPC theo dòng, không msgpack-rpc.** Plugin chỉ cần `vim.json` + một pipe, người dùng gõ tay được bằng `socat`.
- **Cursor phía server, client chỉ kéo cửa sổ đang xem** (`execute` trả trang đầu `page_size`, `fetch` lấy tiếp). Đây là nguyên tắc "virtual scrolling" của dự án áp dụng qua ranh giới process — render 10 000 dòng (`MAX_ROWS`) trong buffer Lua một lần sẽ chậm. Tối đa 16 cursor, cái cũ nhất bị loại trước, vì mỗi cursor có thể giữ tới `MAX_ROWS` dòng.
- **`edit.sql` chỉ sinh câu lệnh, không chạy.** Giữ đúng quy tắc của TUI "hiện ra, chỉ chạy sau `y`" — client phải gọi `execute` rõ ràng.
- **Socket owner-only**: thư mục `0700` trước khi tạo socket, socket `0600` sau khi bind, vì socket này là cửa vào mọi database đã lưu. `bind` từ chối đè lên socket đang có server sống, nhưng dọn socket mồ côi sau crash.
- **Registry nhân đôi** giữa `tradar-app` và `tradar-server` (cùng mẫu `#[cfg(feature)]`) — chấp nhận ở giai đoạn này; rút ra dùng chung chỉ đáng làm khi có connector thứ ba cần.
- **Không có background ping**: `QueryEngine::tick()` (nơi ping 15s) không chạy ở server. `status` ping theo yêu cầu. Cố tình chưa làm vòng ping nền.

## Giới hạn đã biết

- Đường dẫn unix socket tối đa ~108 byte; `XDG_RUNTIME_DIR` rất dài sẽ lỗi `path must be shorter than SUN_LEN` (gặp thật khi smoke test với thư mục scratchpad dài) — truyền đường dẫn ngắn làm đối số cho `tradar-server` / `setup{socket=...}`.
- Chỉ unix (WSL/Linux/macOS), chưa có Windows named pipe.
- Chưa huỷ được query đang chạy (TUI có `cancel()`, protocol chưa có).
- Completion hiện chỉ là keyword + tên schema, chưa có `completion_context` (alias `.`, gợi ý JOIN theo FK).
- Không có xác thực ngoài quyền file của socket.
