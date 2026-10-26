# tradar.nvim

Chạy query trên database ngay trong Neovim. Editor là buffer Neovim bình thường; `tradar-server` (một process chạy nền) giữ connection. Thiết kế: mục "Server headless" trong `docs/architecture.md`.

## Cài

```bash
cargo build --release -p tradar-server        # trong repo tradar
```

**Đang dev tradar (khuyên dùng lúc này)** — `dir =` trỏ thẳng vào checkout, Lua và `tradar-server` luôn khớp vì cùng một checkout; sửa gì chỉ cần restart Neovim (hoặc `:TradarRestart` riêng cho server), không qua bước git nào:

```lua
{ dir = "~/dev/local/tradar/nvim", name = "tradar.nvim", lazy = false,
  dependencies = { "nvim-telescope/telescope.nvim" },
  config = function() require("tradar").setup {} end }
```

`lazy = false`, không phải `ft = "sql"` — `.mongo`/`.redis`/`.esq` chỉ trở thành filetype thật sau khi `plugin/tradar.lua` tự nó chạy (`vim.filetype.add`), nên lazy.nvim's `ft` trigger không bao giờ nhận ra 3 filetype đó để mà load trước (gà-và-trứng); `dir=` với một plugin nhẹ (chỉ đăng ký autocmd/command, không có gì nặng lúc startup) thì `lazy = false` không đáng lo.

**Khi tradar ổn định hơn (chưa đến lúc này)** — cài như một plugin GitHub bình thường, `nvim/` vẫn là thư mục con của repo Rust nên cần `build` để compile server và `config` để trỏ `rtp` vào đúng `nvim/`:

```lua
{ "tuannm99/tradar", name = "tradar.nvim", lazy = false,
  build = "cargo build --release -p tradar-server",
  dependencies = { "nvim-telescope/telescope.nvim" },
  config = function(plugin)
    vim.opt.rtp:prepend(plugin.dir .. "/nvim")
    require("tradar").setup {}
  end }
```

Đánh đổi so với `dir=`: mỗi lần sửa tradar phải `git push` rồi `:Lazy update` mới thấy, không còn vòng lặp sửa-restart-thử ngay — chỉ đáng khi code đã đứng, không còn sửa mỗi ngày.

`:checkhealth tradar` kiểm tra binary, server, connection đã lưu. `:TradarSave` / `:TradarSaveHere` / `:TradarOpen` lưu và mở query (thư viện chung hoặc cạnh file; `setup{ queries_dir }`) — xem `docs/wiki/Truy-van-SQL.md`. `:TradarLog` mở file log của server (bám theo dòng mới, `q` đóng) — xem `docs/wiki/Xu-ly-su-co.md`. Server chạy nền và giữ nguyên bản binary cũ cho tới khi dừng: sau khi build lại, chạy `:TradarRestart` (lệnh kế tiếp tự khởi động bản mới).

## Dùng

`${tên}` / `${tên=mặc-định}` trong câu lệnh được hỏi lúc chạy (thay chữ, nhớ giá trị). `:TradarAddConnection` thêm connection bằng hỏi đáp và thử kết nối ngay. Mọi lệnh có dạng `:Tradar…`; `:Tradar <lệnh-con>` là một lối vào chung (`:Tradar kafka tail`, `:Tradar run all`), `<Tab>` gợi ý lệnh con, tham số và tên connection đã lưu.

Connection lấy từ `~/.config/tradar/connections.toml` (cùng file với TUI); `:TradarConnections` mở đúng file server đang đọc (có template nếu chưa có, `:w` là server thấy ngay và plugin báo số connection hoặc lỗi TOML), picker `:TradarConnect` cũng có dòng `+ edit connections file…`. Gắn một file `.sql` vào connection bằng dòng đầu file:

```sql
-- tradar: tên-connection
```

hoặc file `.tradar` (một dòng, tên connection) ở thư mục cha, hoặc `:TradarConnect`.

### Mongo, Redis, Elasticsearch

Dùng file `.mongo` (filetype `mongo`, tô màu như JavaScript), `.redis`, `.esq` (Elasticsearch, kiểu `GET /index/_search` + body JSON), hoặc **bất kỳ file nào** có dòng liên kết ở đầu — dấu comment tuỳ ngôn ngữ:

```
// tradar: tên-connection     (Mongo, JS)
# tradar: tên-connection      (Redis / Elasticsearch)
-- tradar: tên-connection     (SQL)
```

Mongo nhận cú pháp mongosh (`{name: 'ann'}`, `ObjectId("…")`), không chỉ JSON chặt. Kết quả dạng tài liệu mặc định hiện **JSON** (`ObjectId("…")`/`ISODate("…")` in đúng như mongosh, chỉ để xem); `<leader>rv` đổi sang **bảng** (cột phẳng `address.city`, dạng cần để sửa trực tiếp; `setup{ documents_view = "table" }` để mở bằng bảng) Trong bảng có thể sửa (`:w` → xem trước → commit, `UPDATE`/`DELETE`; chưa có `INSERT`), `gyc` và export như SQL. `<leader>re` (EXPLAIN) chỉ có cho SQL.

### gRPC

Khác hẳn SQL/Mongo/Redis/Elasticsearch: không có buffer nguồn nào gắn connection — một connection gRPC chỉ là `driver = "grpc"` trong `connections.toml` (`target` = `host:port` trần hoặc URL đầy đủ, plaintext). `:TradarGrpcConnect [tên]` (bỏ trống thì chọn qua picker, ưu tiên connection của buffer request đang đứng nếu có), rồi `:TradarGrpcServices` liệt kê service đã biết (qua server reflection thật, hoặc rỗng nếu server không hỗ trợ — nạp `.proto` tay bằng `:TradarGrpcLoadProto <đường-dẫn>`). Chọn service rồi method mở một buffer JSON mới (tab mới) đã điền sẵn khung request sinh từ message đầu vào (mọi trường ở giá trị mặc định, đúng thứ tự khai báo; `oneof` chỉ có nhánh đầu) — sửa giá trị rồi gửi, `<leader>rr`/`:TradarGrpcSend` gọi thật (response hiện ở split riêng, `q` đóng split đó — buffer response không sửa được nên không mất gì), `<leader>rx`/`:TradarGrpcCancel` huỷ nếu đang chạy lâu, `g?` xem lại các phím này. Buffer request **không** nhận `q` (buffer này sửa được — để `q` nguyên nghĩa ghi macro của vim), đóng bằng `:q`/`:tabclose` mặc định. Chọn lại đúng connection/service/method đã mở sẽ nhảy về tab cũ thay vì mở thêm tab mới. Chỉ **unary** cho v1 (xem `docs/architecture.md`, "Chốt lại scope v1" mục gRPC) — chưa streaming, chưa metadata/header.

Target `https://…` bật TLS (`?ca=…&cert=…&key=…&domain=…` ở cuối target cho CA riêng/mTLS). Metadata của request là các dòng `# tên: giá trị` ở đầu buffer request (`# deadline: 5s` đặt deadline của call); header của response hiện phía trên JSON (unary). Method streaming (`[server stream]`/`[client stream]`/`[bidi stream]` trong picker): `<leader>rr` bắt đầu call (client/bidi: gửi buffer làm message, chạy lại để gửi thêm), `<leader>rc` / `:TradarGrpcClose` đóng chiều gửi, `<leader>rx` dừng; response là transcript chỉ-đọc (`‹` nhận, `›` gửi, `#` header/trailer/trạng thái).

### Kafka tail

Tail realtime một topic Kafka qua `tradar-server`, yêu cầu build server với `cargo build -p tradar-server --features kafka` (không nằm trong `default`, xem CLAUDE.md). `:TradarKafkaConnect [tên]` (picker nếu bỏ trống, lọc `driver = "kafka"`), rồi `:TradarKafkaTail [tên] [topic]` — bỏ trống phần nào thì hỏi qua `vim.ui.select`, luôn hỏi tail từ `latest` hay `earliest`. Mở một buffer transcript cuộn (tab mới, không sửa được, giống panel Topics mode của TUI) — mỗi dòng mới nhận được nối vào cuối (con trỏ chỉ bám theo khi đang ở dòng cuối; giữ 5000 dòng gần nhất), `p` tạm dừng/tiếp tục (tin vẫn về, hiện khi tiếp tục), `f` lọc theo chuỗi con (không phân biệt hoa thường; đổi bộ lọc dựng lại buffer từ các tin đã nhận), `c` xoá, `R` đăng ký lại từ `latest` khi mất kết nối tới server (plugin tự thử một lần sau ~1,5s và ghi `[reconnected — …]` vào transcript; tin trong lúc mất kết nối không được phát lại); winbar hiện connection/topic, `PAUSED`, bộ lọc, trạng thái mất kết nối. `q` dừng tail và đóng (buffer tự `bufhidden = wipe`, đóng cửa sổ cuối cùng là dừng luôn subscription phía server). `g?` xem lại phím. Khác gRPC/SQL: server **đẩy** dòng mới về (JSON-RPC notification `kafka.tail.message`, không phải trả lời một request), nên không có khái niệm "chạy xong" hay `<leader>rx` huỷ — chỉ có dừng hẳn bằng `q`. Mỗi lần `:TradarKafkaTail` là một consumer Kafka riêng (nhóm consumer group dùng-một-lần, không chia tải với ai) — tail hai lần cùng một topic thấy đúng hai luồng độc lập, không phải chia đôi dữ liệu.

### Socket tail

Raw TCP, netcat-style, qua `tradar-server` — cùng kênh đẩy với Kafka tail, trong `default` (không cần toolchain gì thêm, khác `kafka`). `:TradarSocketTail [tên]` (picker nếu bỏ trống, lọc `driver = "socket"`) mở ngay một buffer transcript (tab mới, insert mode sẵn) — khác Kafka: buffer này **sửa được** và **hai chiều**: dòng cuối cùng là dòng nhập (kiểu prompt REPL): gõ rồi `<CR>` (normal hoặc insert) là gửi và xoá dòng nhập; dòng gửi/nhận chèn phía trên nó, buffer giữ 5000 dòng gần nhất. `›` đánh dấu dòng đã gửi, `‹` dòng nhận được — cùng ký hiệu TUI's Socket screen đã dùng. `q` dừng hẳn (đóng kết nối TCP phía server) và đóng buffer; `g?` xem lại phím. Không có bước connect riêng như Kafka/gRPC — một connection Socket *là* chính phiên TCP đó, nên `:TradarSocketTail` vừa mở vừa dial luôn; gọi lại lần hai cùng connection mở một phiên TCP độc lập hoàn toàn (một netcat session khác), không chia sẻ gì với phiên trước.

### RabbitMQ

Browse queues/exchanges/bindings, peek message (không tiêu huỷ, `ackmode=ack_requeue_true`), publish — qua Management HTTP API, giống TUI nhưng không có sidebar riêng: mỗi thao tác là một lệnh. Không cần kênh đẩy (API không có endpoint stream) nên không có toolchain/feature riêng, nằm sẵn trong `default`. `:TradarRabbitmqConnect [tên]` (picker nếu bỏ trống, lọc `driver = "rabbitmq"`). `:TradarRabbitmqPeek [tên] [queue]` — bỏ trống queue thì chọn qua `vim.ui.select` (kèm `ready`/`unacked`/`consumers` của từng queue), kết quả hiện trong một split dùng lại được (không sửa được, `q` đóng, `g?` xem phím) — mỗi payload một khối, `[redelivered]` đánh dấu nếu có. `:TradarRabbitmqBindings [tên] [exchange]` — tương tự, chọn exchange (exchange mặc định không tên bị lọc, không có gì để browse). `:TradarRabbitmqPublish [tên]` mở một **buffer publish sửa được** (mỗi connection một buffer, giữ lại để sửa nhẹ rồi publish lại): `exchange:` (bỏ trống = default exchange), `routing-key:` (với default exchange là tên queue), dòng trống, rồi payload nhiều dòng; `<leader>rr` (hoặc `:TradarRabbitmqPublish` khi đang trong buffer) publish. Trong split peek/bindings, `r` làm mới (peek lại / liệt kê lại). Không có tail — khác Kafka/Socket, RabbitMQ qua server chỉ là request/response thuần, peek là chụp một lần, không theo dõi liên tục.

### HTTP

Postman-style request/response — không sidebar riêng như TUI's `HttpScreen`, một lệnh duy nhất đi qua từng trường. Không cần kênh đẩy (giống RabbitMQ, mỗi request là một request/response thuần) nên không cần toolchain gì thêm, nằm sẵn trong `default`. `:TradarHttpConnect [tên]` (picker nếu bỏ trống, lọc `driver = "http"`) — không probe gì cả (base URL có thể rỗng hoặc chưa sống), chỉ ghi nhận connection, nên không bao giờ lỗi, khác `rabbitmq.connect`. `:TradarHttpSend [tên]` mở một **buffer request sửa được** (tab mới, `filetype=http`, mỗi connection một buffer, giữ nội dung để sửa nhẹ rồi gửi lại): dòng đầu `METHOD url` (chỉ URL = GET; URL đã có scheme dùng nguyên, còn lại nối vào target của connection làm base), tiếp theo các dòng `Key: Value`, dòng trống, rồi body nhiều dòng; dòng `#` trước dòng trống là comment. `<leader>rr` (hoặc `:TradarHttpSend` khi đang đứng trong buffer) gửi; `g?` xem phím; không chiếm `q`. Kết quả (status/headers/body) hiện trong một split dùng lại được (không sửa được, `q` đóng, `g?` xem phím), cùng buffer `tradar://http` cho mọi lần gửi.

### Định dạng `.tdb` (nhiều dialect, nhiều connection, một file)

Thay vì chia theo filetype (`.sql`/`.mongo`/`.redis`/`.esq`) — bất tiện khi muốn so sánh/ghi chú nhiều truy vấn khác dialect cạnh nhau — một file `.tdb` gồm nhiều block kiểu fenced code của Markdown, mỗi block tự chọn dialect và (tuỳ chọn) connection riêng:

````
# ghi chú thoải mái ở ngoài block, không chạy được

```sql pg-local
SELECT * FROM users LIMIT 10;
```

```mongo atlas-dev
db.users.find({active: true})
```

```redis
GET session:123
```
````

Dòng mở fence là `` ```<dialect> [connection] `` — thiếu `connection` thì block đó rơi về connection của buffer (modeline `-- tradar: name`/`.tradar`/`:TradarConnect`, giống file thường). `<leader>rr`/`K`/`gd`/`<leader>re`/completion đều áp dụng cho **block đang chứa con trỏ**, không phải cả buffer — con trỏ trên dòng fence của block cũng chạy block đó; đặt con trỏ ở giữa các block thì `<leader>rr` báo rõ thay vì chạy nhầm. `<leader>ra` (chạy cả file)/`<leader>rx` (huỷ)/visual-mode `<leader>rr` chưa áp dụng cách này — vẫn theo connection của buffer như trước.

Tô màu: `.tdb` dùng chung parser treesitter của Markdown (fenced code block tự inject theo tag — `sql` là grammar thật nên tô đúng cú pháp SQL; `mongo`/`esq` tô theo JavaScript/JSON qua cùng alias plugin đã đăng ký cho filetype `.mongo`/`.esq` riêng; `redis` không có grammar nào khớp nên hiện plain, giống hệt file `.redis` hiện tại). Suggestion (completion bảng/cột) theo đúng dialect của block, không phải LSP thật — LSP thật theo từng block (qua [otter.nvim](https://github.com/jmbuhr/otter.nvim)) để dành cho sau.

| Phím (tiền tố `<leader>r`) | Việc |
|---|---|
| `rr` | chạy statement dưới con trỏ (visual: phần chọn) |
| `ra` / `rx` | chạy cả file / huỷ query đang chạy |
| `rt` / `rh` | telescope: bảng (`<CR>` chèn tên, `<C-o>` sinh câu xem dòng vào buffer — không chạy, `<C-g>` sinh create/read/update/delete) / history (`<CR>` dán, `<C-r>` chạy) |
| `rg` | `:TradarCrud [create\|read\|update\|delete]`: sinh câu lệnh mẫu của bảng dưới con trỏ vào buffer (không chạy); `c` trong panel schema |
| `re` | `EXPLAIN` statement dưới con trỏ (không bao giờ `ANALYZE`, vì nó *chạy* câu lệnh) |
| `rs` / `rc` / `rm` | panel schema / chọn connection / tải thêm dòng |
| `rz` | phóng cửa sổ kết quả ra toàn màn hình / trả lại (`:TradarResultsZoom`; cũng có trong buffer kết quả) |
| `[R` / `]R`, `rp` | (buffer kết quả) lần chạy trước / sau của tab (`setup{ results_history }` = 5); ghim tab để lần chạy mới mở tab riêng |
| `rF` / `ro` | (buffer kết quả) lọc / sắp xếp các dòng đã tải (`:TradarFilter {biểu thức\|off}`); lưới chỉ đọc khi đang bật |
| `rD` | (buffer kết quả) khác biệt với lần chạy trước hoặc tab n (`:TradarDiff [n]`), theo khoá |
| `rR` / `rW` | (buffer kết quả) chạy lại kết quả tại chỗ / theo dõi mỗi vài giây; chỉ câu không ghi dữ liệu |
| `[r` / `]r`, `r1`…`r9`, `rx` | (buffer kết quả) mỗi bảng/collection/index/key một tab kết quả (thanh tab trên cửa sổ, click được): tab trước / sau, nhảy tới tab n, đóng tab; `:TradarResults` chọn (`setup{ results_kept }` = 10 tab, `results_cursors` = 4 tab giữ cursor mở để tải thêm/sửa). Câu vừa chạy nháy sáng (`setup{ flash_ms }`, nhóm `TradarRunning`) |
| `K` | thông tin bảng/cột dưới con trỏ (kiểu, PK, FK, index; hiểu alias `o.user_id`); không biết thì rơi về hover của LSP |
| `gd` | trên tên bảng: sinh câu xem dòng (`SELECT ... LIMIT 100`) vào buffer, `rr` để chạy (`setup{ open_runs = true }` chạy ngay); chỗ khác: `definition` của LSP |
| `g?` | liệt kê phím của buffer hiện tại (SQL, kết quả, panel schema đều có) |

Trong buffer kết quả: nếu kết quả sửa được (một bảng, có khoá chính, cột khoá có trong kết quả, Postgres/MySQL/SQLite) thì lưới là buffer **sửa được như bình thường** — `cw`, `dd`, `o`/`yyp` + gõ dòng mới, `:s`, `:g`, `u`…; chưa có gì chạy cho tới khi `:w`. `:w` so lưới với các dòng đã vẽ rồi mở buffer xem trước `tradar://commit` liệt kê mọi `UPDATE`/`DELETE`/`INSERT` (update → delete → insert); `:w`/`<CR>` ở đó chạy lần lượt rồi tự làm mới và giữ con trỏ, `q` huỷ, `<leader>ru` bỏ các sửa chưa commit. Dòng được nhận theo giá trị cột khoá hiện trên dòng (không theo số dòng) nên `dd`/`yyp`/`:sort` đều đúng; ô để trống ở dòng mới = dùng `DEFAULT`, gõ `NULL` để đặt NULL; sửa cột khoá hiện thành `DELETE`+`INSERT`. Dòng trùng khoá, không tách được đủ cột, hay sửa header/footer thì `:w` báo lỗi, không đoán. Commit chạy trong **một transaction** (`BEGIN`/`COMMIT`) với Postgres/MySQL/SQLite: một câu lỗi thì `ROLLBACK` hết, lưới vẽ lại từ DB, xem trước mở lại với toàn bộ câu lệnh + lỗi (nếu bạn đã tự `BEGIN` thì chạy bên trong transaction đó và không `COMMIT` hộ); connector khác không có transaction nên các câu đã chạy giữ nguyên và xem trước chỉ còn phần chưa chạy. Khi kết quả không sửa được, `i`/`dd`/`o`… báo lý do (không biết bảng nguồn, không có khoá chính, thiếu cột khoá). `gd` trên ô của cột khoá ngoại chạy `SELECT` dòng được tham chiếu (kết quả của truy vấn một bảng), `gyc` ô, `gyr` dòng, `gyC` cột, `gyj`/`gyv`/`gym` cả kết quả (JSON/CSV/Markdown), `<leader>rv` đổi bảng ↔ JSON (cả kết quả SQL, dạng mảng object; ở view JSON các thao tác theo ô tạm tắt), `gK` xem cả dòng hiện tại mỗi trường một dòng (kiểu `\x`), `K` xem trọn giá trị một ô (hữu ích khi ô bị cắt bớt — xem dưới), `:TradarExport csv|json|md|tsv [file]`, `q` đóng. Mỗi trang 20 dòng (`setup{ page_size }`), trang kế tải bằng `<leader>rm` (không tự tải khi cuộn; `setup{ auto_load_more = true }` để như cũ). Khi query chạy, winbar của cửa sổ kết quả hiện spinner + thời gian + `<leader>rx` huỷ; visual mode `<leader>rr` tách vùng chọn theo `;` rồi chạy lần lượt.

**Cột quá rộng tự bị cắt** (cap 40 ký tự hiển thị, giống TUI) — ô dài hơn hiện dấu `…` ở cuối thay vì kéo giãn cả cột; `K` trên ô đó xem giá trị đầy đủ. Chưa có cách resize tay như TUI (`<`/`>`) — nếu cần xem/copy giá trị đầy đủ, dùng `K` hoặc `gyc`.

Buffer kết quả **không có sort/filter theo cột** như TUI — chủ đích: Neovim đã có sẵn editor SQL thật ngay cạnh, sửa `ORDER BY`/`WHERE` rồi `<leader>rr` chạy lại tiện hơn một UI tương tác riêng.

Cần Neovim ≥ 0.10 (`:checkhealth tradar` kiểm tra). Bảng đầy đủ mọi phím/lệnh theo từng buffer: `docs/wiki/Phim-tat.md`. Statusline: `require('tradar').status()` (chuỗi rỗng ngoài buffer SQL).
Completion: nguồn `blink.cmp` `tradar.blink` (hoặc `omnifunc`).

## An toàn

Hỏi xác nhận (mặc định là `Cancel`, nên `<CR>` phản xạ là đáp án an toàn) trước khi chạy:

- `UPDATE`/`DELETE` **không có `WHERE`**, `DROP`, `TRUNCATE` — trên mọi connection.
- **Mọi lệnh ghi** (`INSERT`/`UPDATE`/`DELETE`/DDL...) trên connection "protected" — tên chứa một trong `setup{ protected = {"prod"} }` (mặc định `{"prod"}`, không phân biệt hoa thường). Statusline hiện `⚠` bên cạnh.
- Mongo: `drop()`, `dropDatabase()`, `deleteMany({})`/`updateMany({}, …)` (bộ lọc rỗng) luôn hỏi. Redis: `FLUSHALL`/`FLUSHDB`/`SHUTDOWN`. Elasticsearch: `DELETE …`, `_delete_by_query`. Trên connection protected: mọi lệnh ghi của ngôn ngữ đó.
- `<leader>ra` chạy cả file chỉ hỏi **một lần** cho cả lô.
- `setup{ confirm = false }` tắt hết.

Đây là phân tích theo token (bỏ qua comment và chuỗi), không phải parser: bắt các lỗi tay thường gặp, không phải rào chắn bảo mật.

## Test

`make test-nvim` (hoặc `nvim/tests/run.sh [tên]`) chạy `nvim/tests/*_spec.lua`: mỗi spec một Neovim headless, một `tradar-server` và một database SQLite rỗng riêng (connection `local`). Cần `nvim` và `python3`, không cần Docker. Telescope và plenary (phiên bản ghim) được `nvim/tests/deps.sh` tải vào `.deps/` (gitignore) lần đầu — cần mạng; `H.telescope()` nạp chúng cho spec cần picker. Spec mới dùng `nvim/tests/helpers.lua` (`H.setup`, `H.run_until`, `H.check`, `H.done`); `-- env: K=V` ở đầu file đặt biến môi trường cho server.
