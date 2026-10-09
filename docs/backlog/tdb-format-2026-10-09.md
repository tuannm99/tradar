# Neovim: định dạng `.tdb` — nhiều dialect/connection trong một file (2026-10-09)

## Bối cảnh

Người dùng nhận xét sau khi dùng plugin vài hôm: chia theo filetype (`.sql`/`.mongo`/`.redis`/`.esq`) khó dùng chung với LSP/completion builtin, và muốn một chỗ duy nhất ghi/so sánh nhiều truy vấn khác dialect cạnh nhau — đề xuất một file duy nhất, các block như fenced code trong Markdown (`` ```sql``` ``, `` ```mongo``` ``, `` ```redis``` ``), có tô màu + suggestion ngay trong từng block.

Chốt qua `AskUserQuestion` trước khi code (2 câu):

1. **LSP thật trong block** (sqlls... theo đúng ngôn ngữ con, kiểu [otter.nvim](https://github.com/jmbuhr/otter.nvim) giải cho Quarto/markdown notebook) — chọn **để sau**, làm highlighting + completion của chính tradar trước.
2. **Connection theo block** — chọn **mỗi block tự chọn riêng** (`` ```sql tên-connection ``), không phải cả file một connection.

Đuôi file ban đầu đề xuất `.td` — người dùng tự nhận ra có thể đụng (LLVM TableGen's đuôi chuẩn, Vim/Neovim có sẵn `filetype.lua` map `*.td` → `tablegen`), hỏi lại và chốt `.tdb` ("tradar database").

## Thiết kế

### Parse block: thuần text, không treesitter (`nvim/lua/tradar/blocks.lua`)

`blocks.parse(lines)` — fence `` ``` `` mở ở cột 0, info string sau fence là `<dialect> [connection]` (`connection` optional); đóng bằng `` ``` `` trơn. Trả về danh sách block `{lang, conn, start, finish}` (1-based, `start`/`finish` là dòng **nội dung** — không tính 2 dòng fence). Fence không đóng chạy tới EOF (giống cách một chuỗi không đóng vẫn đọc tới hết file chứ không bị bỏ). `blocks.at(lines, line)` — block chứa dòng đó, `nil` nếu dòng đang ở prose giữa các block hay ngay trên dòng fence.

Không dùng treesitter để tìm block vì logic chạy/connection-resolution cần hoạt động được ngay cả khi không có parser nào cài (tô màu là việc độc lập, xem dưới) — và vì parse dòng đơn giản hơn hẳn truy vấn cây cho một cấu trúc chỉ cần biết "dòng N nằm trong fence nào".

### Resolve connection: `M.block_connection(buf, row)`

`M.connection_for(buf)` (đã có, theo buffer) + ưu tiên `block.conn` nếu dòng đang ở trong một block có ghi connection riêng. Dùng ở mọi nơi theo vị trí con trỏ: `run_text` (qua `base_row`), `M.hover`, `M.goto_table`, `M.status`, `M.explain`, completion (`M.complete`/omnifunc/blink source).

### Chạy block, không chạy cả buffer (`M.run()`)

Khác hẳn mọi filetype khác — `.tdb` có thể trộn dialect trong cùng buffer, nên gửi cả buffer cho RPC `split` (tìm statement boundary theo đúng driver của MỘT connection) sẽ hỏng ngay khi buffer có hơn 1 dialect. `M.run()` giờ rẽ nhánh đầu: filetype `tdb` thì tìm block chứa con trỏ (`M.block_at`), cắt `lines` về đúng block đó, nhớ `base_row` (dòng đầu block trừ 1, 0-based) để sau đó cộng lại vào mọi vị trí (`position()`'s trả về, cursor-offset so khớp statement) trước khi tới `run_text`/diagnostics — cả hai chỉ biết toạ độ buffer thật, không biết gì về "block". Không ở trong block nào thì báo rõ lý do, không chạy nhầm cả buffer.

### Completion: cắt text theo block (`M.text_before_cursor`)

Trước đây `blink.lua`/`omnifunc` tự cắt `nvim_buf_get_lines(buf, 0, row, false)` (toàn bộ buffer tới con trỏ) rồi gọi `M.complete`. Với `.tdb`, trộn text của block trước (dialect khác) vào input gửi server sẽ làm hỏng `completion_context` phía Rust (nó không biết "bắt đầu lại" ở đâu). `M.text_before_cursor(buf, row, col)` mới — cắt từ đầu block (không phải đầu buffer) khi filetype là `tdb`, `nil` nếu ngoài block (completion tắt). `blink.lua`/`omnifunc` gọi hàm này thay cho tự cắt tay; `M.complete(buf, text, cb, row)` thêm tham số `row` optional để resolve đúng connection theo block.

### Tô màu: dùng lại parser Markdown, không viết query injection riêng

`.tdb` đăng ký `vim.treesitter.language.register('markdown', 'tdb')` — để treesitter-markdown tự lo cấu trúc fenced block + injection theo tag info string, đúng cơ chế nó đã có sẵn cho mọi file `.md` thật. Không viết `injections.scm` riêng: `vim.treesitter.language.register('javascript', 'mongo')`/`register('json', 'esq')` **đã có sẵn** trong `plugin/tradar.lua` (cho filetype `.mongo`/`.esq` độc lập) — cùng API resolve alias mà injection của markdown dùng để tìm parser theo tên tag, nên `` ```mongo``` ``/`` ```esq``` `` trong `.tdb` tự động ăn theo, không cần thêm gì. `` ```redis``` `` không có parser nào đăng ký (giống `.redis` hiện tại vốn không tô màu) — hiện plain trong block, không phải hồi quy.

**Lưu ý:** phần tô màu không verify được trong sandbox này (không có Neovim/treesitter thật để chạy, chỉ có `lua5.3` thuần) — dựa trên hiểu biết về cách injection của markdown resolve tên ngôn ngữ qua `vim.treesitter.language.get_lang()`, cùng API `register()` đã dùng. Cần người dùng tự mở một file `.tdb` thật để xác nhận.

## Test

`blocks.lua` test bằng harness `lua5.3` độc lập (không cần Neovim): parse đúng `lang`/`conn`/`start`/`finish` cho nhiều block, `conn` thiếu thì `nil`, fence không đóng chạy tới EOF, `at()` trả `nil` đúng trên dòng fence/giữa các block/buffer rỗng. Riêng phần toán `base_row`/offset trong `M.run()` (cộng/trừ khi chuyển vị trí block-relative ↔ buffer thật) verify bằng một mô phỏng tay (cùng công thức, không phải chạy code thật) — xác nhận statement thứ 2 trong một block 2 dòng map đúng về dòng buffer thật, và cursor đặt trên dòng đó resolve đúng về statement đó.

Không test được end-to-end qua Neovim thật (không có binary `nvim` trong sandbox).

## Chưa làm

- LSP thật theo từng block (otter.nvim) — để sau, đã chốt lúc scoping.
- `<leader>ra` (chạy cả file) và visual-mode `<leader>rr` chưa áp dụng cách cắt-theo-block — vẫn chạy theo connection của buffer như file thường, chưa nghĩ qua việc "chạy cả file .tdb" nghĩa là gì khi file có nhiều dialect trộn nhau (chạy tuần tự từng block theo đúng connection của nó? hợp lý nhưng chưa làm).
- Tô màu chưa verify thật trên Neovim — xem lưu ý ở trên.
