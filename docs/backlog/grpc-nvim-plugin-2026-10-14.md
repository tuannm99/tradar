# gRPC: plugin Neovim (2026-10-14)

## Bối cảnh

Nửa thứ hai (và cuối cùng, cho scope v1) của "gRPC cho `tradar-server` + Neovim" — nửa server (`grpc.*` method mới) xong trong cùng ngày, xem `docs/backlog/grpc-rpc-methods-2026-10-14.md`. Đây là việc kế tiếp được ghi lại ở đó.

## Vấn đề

Mọi connector khác trong plugin Neovim (SQL, Mongo, Redis, Elasticsearch) gắn vào một **buffer nguồn**: modeline `-- tradar:`, file `.tradar`, hoặc filetype riêng (`.mongo`/`.redis`/`.esq`). gRPC không có khái niệm đó — không có "file SQL" nào để gõ một lời gọi RPC vào, chỉ có service/method chọn từ danh sách và một request JSON gõ tay. `init.lua`'s mọi hàm (`run`, `attach`, `edit_cell`, cursor/`fetch`...) đều giả định có một buffer connection-gắn-sẵn và một statement text lấy từ nó — không khớp hình dạng này.

## Thay đổi

### `nvim/lua/tradar/grpc.lua` (mới)

Tự viết `scratch`/response-split riêng, không tái dùng các hàm cùng tên trong `init.lua` (chúng là `local`, không export, và được thiết kế cho **một** buffer kết quả dùng chung xuyên session — sai mô hình cho gRPC, nơi nhiều method/buffer request có thể mở cùng lúc, mỗi cái cần response riêng không tranh nhau). Cùng lý do Kafka/RabbitMQ/HTTP/Socket trong TUI không tái dùng `QueryScreenComponent`.

- `M.connect(name)`: không cho sẵn tên → `connections.list`, lọc `driver == "grpc"`, `vim.ui.select`. Luôn gọi `grpc.connect` (idempotent phía server — gọi lại trên connection đã mở chỉ trả `already: true`) trước mọi hành động khác (`services`/`methods`/`invoke`) thay vì tự theo dõi "đã connect chưa" ở phía Lua — đơn giản hơn `init.lua`'s `ensure_connected`/`state.connected` vì không cần.
- `M.services(name)`: `grpc.services`, rỗng thì gợi ý `:TradarGrpcLoadProto`; không rỗng thì `vim.ui.select` → tự động gọi tiếp `M.methods` cho service chọn.
- `M.load_proto(name, path)`: `grpc.load_proto`, thông báo service nào mới thêm.
- `M.methods(name, service)`: `grpc.methods`, `vim.ui.select` method → `M.open_request`.
- `M.open_request(connection, service, method)`: mở buffer scratch filetype `json` trong **tab mới** (không split — không có layout cố định nào để giữ như query screen có), nội dung khởi đầu `{}`. Lưu `vim.b[buf].tradar_grpc = {connection, service, method}`; gắn command buffer-local `TradarGrpcSend` + keymap `<leader>rr` (cùng phím "chạy" dùng cho SQL, vì buffer này cũng không dùng phím đó cho gì khác).
- `M.send(buf)`: `vim.json.decode` toàn bộ buffer làm request (lỗi parse báo rõ, không gọi RPC); `grpc.invoke`; response hiện trong một buffer/split riêng gắn theo buffer request (`vim.b[request_buf].tradar_grpc_response`, dựng lần đầu rồi tái dùng cho lần gửi sau của cùng buffer đó).
- `require('tradar').rpc_opts()` (hàm mới, nhỏ, trong `init.lua`): để `grpc.lua` dùng đúng `setup{socket=...}` người dùng đã chọn thay vì `rpc.ensure({})` rơi về mặc định riêng — hai module nói với cùng một `tradar-server`, phải cùng socket.

### 4 command mới (`nvim/plugin/tradar.lua`)

`TradarGrpcConnect [tên]`, `TradarGrpcServices`, `TradarGrpcMethods <service>`, `TradarGrpcLoadProto <path>`. Không có autocmd `FileType`/`attach` nào — gRPC không có filetype riêng, mọi command gọi được từ bất kỳ đâu.

### Chưa làm

- **Skeleton request tự sinh theo shape message**: buffer luôn mở với `{}` trần, người dùng tự gõ toàn bộ field — khác `crud_snippet`'s "sinh khung" cho SQL (vd `SELECT col1, col2 FROM t LIMIT 100`). `MethodDescriptor::input()` đủ thông tin để sinh JSON mẫu (field name + kiểu + giá trị mặc định) qua `prost-reflect`, nhưng việc đó thuộc `tradar-server` (method mới `grpc.skeleton` hoặc mở rộng `grpc.methods`'s response) hơn là phía Lua tự đoán hình dạng message — để sau, không chặn dùng được.
- Không có telescope picker riêng cho service/method (dùng `vim.ui.select` trần, qua `dressing.nvim` nếu người dùng đã cài) — khác `:TradarTables`/`:TradarHistory` đã có telescope riêng. Số service/method của một gRPC server thường nhỏ (vài chục), danh sách phẳng chưa cần fuzzy-find.
- Response hiện nguyên JSON một dòng (`vim.json.encode`, không pretty-print) — đúng quy ước `render.documents` đã dùng cho Mongo/Elasticsearch/Redis, không phải thiếu sót riêng của gRPC.
- `:checkhealth tradar` không có mục riêng cho gRPC — `connections.list`/`server reachable` đã chung cho mọi driver nên không cần thêm.

## Test

Không có hạ tầng test Lua tự động trong repo này (chưa từng có cho `nvim/`) — xác nhận bằng smoke test headless Neovim thật, chạy tay trong phiên code (không phải CI):

- Cài `neovim` 0.9.5 (chưa có sẵn trong sandbox), build một server gRPC toy độc lập (`greeter.Greeter`, reflection on, cùng `.proto` với test Rust) chạy thật trên TCP loopback, cùng `tradar-server` thật (feature `grpc`, build từ PR trước) qua unix socket riêng, `connections.toml` riêng (qua `XDG_CONFIG_HOME`, không đụng cấu hình thật của máy).
- Script `nvim --headless -l smoke.lua`: stub duy nhất `vim.ui.select` (trả lời theo kịch bản cố định, vì không có người bấm) — không mock `tradar.rpc`, không mock hai server. 9 kiểm tra: `TradarGrpcConnect` theo tên → `services()`+picker mở đúng buffer request cho `SayHello` → buffer mang đúng `connection`/`service`/`method` và khởi đầu `{}` → sửa JSON, gửi bằng `<leader>rr` → response buffer xuất hiện, parse JSON được, `message`/`length` đúng → `:TradarGrpcSend` gọi lại không lỗi. 9/9 pass.
- Dọn: dừng cả hai server, xoá thư mục build của server toy (không commit vào repo — chỉ để trả lời câu hỏi "plugin này nói đúng với server thật không", giống tinh thần spike trước đó không commit code thử nghiệm).
