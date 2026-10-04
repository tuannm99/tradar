# Server headless + plugin Neovim (giai đoạn 1) — 2026-10-04

Xuất phát từ một nhận xét của user: query editor + navigator + cả bộ vim keymap tự viết trong TUI vừa khó dùng vừa tốn công bảo trì (đang làm lại một phần Neovim). Hướng đi đã chốt: tách phần "chạy query" thành một **server local** (`tradar-server`), dùng Neovim làm UI qua một plugin Lua mỏng. Nhiều session Neovim dùng chung được một server (chung connection đang mở, chung schema).

**Đã chốt với user:** TUI hiện tại **giữ nguyên chạy song song** (không xoá, không đóng băng ở giai đoạn này); quyết định bỏ/đóng băng TUI để sau khi plugin dùng ổn.

## Đã làm (giai đoạn 1)

- `crates/tradar-server` (crate thứ 17): lib + binary `tradar-server`. Chỉ phục vụ các connector có `QueryDriver` (Postgres, MySQL, SQLite, Mongo, Elasticsearch, Redis, Cassandra, ClickHouse — cùng tên feature như `tradar-app`). Kafka/RabbitMQ/HTTP/Socket chưa — xem `docs/roadmap.md`.
- Hai thay đổi nhỏ ở crate cũ, không đổi hành vi TUI: `Session::as_any()` (mặc định `None`, `tradar-connector-spi`) và `QueryEngine::driver()` (`tradar-query-workbench`). Lý do: `Connector::connect` trả `Box<dyn Session>` đóng kín, server cần lấy `Arc<dyn QueryDriver>` ra mà không bắt `tradar-connector-spi` biết kiểu `QueryEngine`.
- Protocol: JSON-RPC 2.0 (không batch, không notification) theo từng dòng (newline-delimited) qua unix socket. Method: `connectors.list`, `connections.list`, `connect`, `disconnect`, `status`, `schema`, `execute`, `fetch`, `cursor.close`, `split`, `keywords`, `edit.source`, `edit.sql`. Chi tiết trong `docs/architecture.md`, mục "Server headless".
- `nvim/` (plugin Lua): `:TradarConnect`, `:TradarRun` (visual / statement dưới con trỏ), `:TradarRunAll`, `:TradarMore`, `:TradarSchema`, `omnifunc`.
- Test: `crates/tradar-server/tests/rpc.rs` (8 test, SQLite thật, gồm một test qua unix socket thật + quyền `0600`); smoke test headless Neovim ↔ server thật chạy tay (connect, paging, `:TradarMore`, statement dưới con trỏ, navigator, omnifunc).

## Bước 2 (cùng ngày): completion theo ngữ cảnh

- Method `complete` tái dùng nguyên `query_driver::completion_context` + `CompletionSource::matches_in_context` (đã `pub` sẵn, không phải sửa crate workbench) nên TUI và Neovim gợi ý giống hệt nhau (alias `.`, JOIN xếp bảng liên quan FK lên đầu, Mongo shape).
- Client gửi toàn bộ text từ đầu buffer tới con trỏ (context cần các dòng trước, ví dụ alias khai báo ở `FROM` dòng trên); phần từ đang gõ do server tách, cùng bộ ký tự từ với `QueryEditorComponent`.
- `CompletionSource` dựng lúc `connect`, dựng lại ở mỗi `schema`. Hệ quả: bảng tạo sau khi connect chỉ được gợi ý sau một lần `schema` — đã ghi vào roadmap.
- Plugin: `omnifunc` gọi `rpc.request_sync` (chặn tối đa 500ms bằng `vim.wait`) vì omnifunc bắt buộc trả danh sách đồng bộ; gắn tự động cho `FileType sql`.
- Test: `complete_is_context_aware_like_the_tui` (alias, lọc theo prefix, nhiều dòng, JOIN không gợi ý lại bảng đã có, fallback phẳng) + smoke headless Neovim (`o.` → 3 cột, `o.us` → `user_id`). Lưu ý khi test headless: Normal mode không đặt con trỏ sau ký tự cuối (col 7 thay vì 8), cần `virtualedit=onemore` để mô phỏng Insert mode.

## Bước 3 (cùng ngày): giai đoạn A — dùng hằng ngày

Làm theo cấu hình thật của user (`~/.config/nvim`: `lazy.nvim` với `defaults.lazy = true`, leader = Space, `blink.cmp` 1.x, `telescope` + `dressing`, `lualine`, `trouble`, `which-key`, `sqlls` cài sẵn qua mason; nhóm `<leader>r*` còn trống nên dùng làm tiền tố).

- **Server:** request trên một kết nối chạy đồng thời (trước đó tuần tự nên `cancel` sẽ kẹt sau query chậm); `execute` nhận `query_id`, method `cancel`; lỗi có vị trí → `error.data {line, column}`; DDL tự dựng lại completion; method `snippet`.
- **Plugin viết lại:** gắn connection theo file (modeline / `.tradar` / `:TradarConnect`), chạy async có spinner + huỷ, diagnostics, kết quả có yank ô/dòng/cột + export + tự tải khi cuộn, statusline, nguồn `blink.cmp`, picker telescope (tables/history), history lưu file, ping nền 15s, `:checkhealth tradar`.
- **Tích hợp vào config của user:** `lua/plugins/tradar.lua` (spec `lazy.nvim` + gộp provider vào `blink.cmp`, bỏ nguồn `lsp` cho `sql` để `sqlls` không lấn) và một dòng `tradar` trong `lualine_x` của `lua/plugins/text-editor.lua` (cố ý đọc `package.loaded["tradar"]` để lualine không ép nạp plugin ở file không phải SQL). Không commit gì trong repo dotfiles của user.
- **Bug tìm ra nhờ smoke test (đều đã sửa):** (1) `jobstart` bị gọi trong callback libuv ("fast context") nên server không tự khởi động được — kết nối giờ `vim.schedule_wrap` về main loop; (2) `is_ddl` chỉ nhìn từ khoá đầu nên một `CREATE TABLE` đi kèm dòng comment phía trên (rất hay gặp, và chính modeline `-- tradar:` cũng là comment) không làm mới completion — giờ bỏ qua comment đầu câu lệnh; (3) kiểm tra "đã connect" ban đầu của test quá yếu (`status()` hiện tên connection dù chưa nối được) nên che mất bug (1).
- **Test:** `tradar-server` 14 test tích hợp + 3 test đơn vị (huỷ query, request đồng thời trên một socket, vị trí lỗi, DDL refresh, snippet...); smoke headless Neovim 17 kiểm tra (modeline, autostart, chạy, phân trang, cuộn tự tải, yank, export 450 dòng, diagnostic, blink, open table, history, spinner, huỷ); chạy lại trên cấu hình thật của user (lazy-load theo `ft=sql`, phím, blink gộp, lualine, picker telescope, `:checkhealth`). Điều **chưa** kiểm chứng: dùng tay trong Neovim có giao diện, và việc huỷ một câu SQLite đệ quy vô hạn có giải phóng được kết nối đó không (huỷ chỉ bỏ future phía server; xem giới hạn).

## Bước 4 (cùng ngày): bảo vệ + điều hướng theo schema

Thuần phía plugin (`guard.lua`, `init.lua`); server không đổi.

- **Bảo vệ:** hỏi xác nhận cho `UPDATE`/`DELETE` thiếu `WHERE`, `DROP`, `TRUNCATE`, và mọi lệnh ghi trên connection "protected" (tên chứa `prod`). Chọn làm client-side vì nó là chính sách của người dùng trên editor của họ, không phải của server; đánh đổi: TUI không có (TUI đã có "xem rồi mới chạy" cho row-edit nhưng không chặn câu SQL gõ tay).
- **`K`/`gd`/`<leader>re`:** xem `docs/architecture.md`. `gd` theo FK chỉ chạy được trên kết quả của truy vấn **một bảng** (cùng giới hạn `edit_source` của row-edit trong TUI).
- **Bug tìm ra nhờ test (đã sửa):** `K` trên `o.user_id` coi `o` là tên bảng nên không tìm thấy gì — alias phải được phân giải từ `FROM`/`JOIN`; test ban đầu không phân biệt được (chỉ một bảng có cột đó), nên thêm ca `u.id` (cả `users` và `orders` đều có `id`) để chứng minh alias được phân giải thật.
- **Test:** `guard` 23 ca (comment/chuỗi/identifier giả `WHERE`, CTE giấu `DELETE`, ...); smoke headless 10 ca bảo vệ (Cancel không đụng dữ liệu, "Run anyway", connection prod, lô hỏi một lần, `confirm=false`) + 11 ca điều hướng; chạy lại smoke giai đoạn A và cấu hình thật của user, kể cả việc `K`/`gd` giành lại phím sau `on_attach` của LSP.

## Quyết định thiết kế và lý do

- **JSON-RPC theo dòng, không msgpack-rpc.** Plugin chỉ cần `vim.json` + một pipe, người dùng gõ tay được bằng `socat`.
- **Cursor phía server, client chỉ kéo cửa sổ đang xem** (`execute` trả trang đầu `page_size`, `fetch` lấy tiếp). Đây là nguyên tắc "virtual scrolling" của dự án áp dụng qua ranh giới process — render 10 000 dòng (`MAX_ROWS`) trong buffer Lua một lần sẽ chậm. Tối đa 16 cursor, cái cũ nhất bị loại trước, vì mỗi cursor có thể giữ tới `MAX_ROWS` dòng.
- **`edit.sql` chỉ sinh câu lệnh, không chạy.** Giữ đúng quy tắc của TUI "hiện ra, chỉ chạy sau `y`" — client phải gọi `execute` rõ ràng.
- **Socket owner-only**: thư mục `0700` trước khi tạo socket, socket `0600` sau khi bind, vì socket này là cửa vào mọi database đã lưu. `bind` từ chối đè lên socket đang có server sống, nhưng dọn socket mồ côi sau crash.
- **Registry nhân đôi** giữa `tradar-app` và `tradar-server` (cùng mẫu `#[cfg(feature)]`) — chấp nhận ở giai đoạn này; rút ra dùng chung chỉ đáng làm khi có connector thứ ba cần.
- **Ping nền nằm ở plugin, không ở server**: `QueryEngine::tick()` không chạy ở server, nên server chỉ ping theo yêu cầu (`status`); plugin gọi `status` mỗi 15s cho các connection đang dùng.

## Giới hạn đã biết

- Đường dẫn unix socket tối đa ~108 byte; `XDG_RUNTIME_DIR` rất dài sẽ lỗi `path must be shorter than SUN_LEN` (gặp thật khi smoke test với thư mục scratchpad dài) — truyền đường dẫn ngắn làm đối số cho `tradar-server` / `setup{socket=...}`.
- Chỉ unix (WSL/Linux/macOS), chưa có Windows named pipe.
- Huỷ (`cancel`) chỉ bỏ phần chờ phía server, giống `cancel()` của TUI: database có thể vẫn chạy nốt câu lệnh (Postgres không gửi `pg_cancel_request`; SQLite không ngắt được một `sqlite3_step` đang chạy).
- Không có xác thực ngoài quyền file của socket.
