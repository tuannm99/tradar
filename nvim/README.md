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

`:checkhealth tradar` kiểm tra binary, server, connection đã lưu. Server chạy nền và giữ nguyên bản binary cũ cho tới khi dừng: sau khi build lại, chạy `:TradarRestart` (lệnh kế tiếp tự khởi động bản mới).

## Dùng

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

Mongo nhận cú pháp mongosh (`{name: 'ann'}`, `ObjectId("…")`), không chỉ JSON chặt. Kết quả dạng tài liệu hiện **bảng** (cột phẳng `address.city`) để dùng `i`/`dd`/`gyc`/export như SQL; `<leader>rv` đổi sang JSON. `<leader>re` (EXPLAIN) chỉ có cho SQL.

### gRPC

Khác hẳn SQL/Mongo/Redis/Elasticsearch: không có buffer nguồn nào gắn connection — một connection gRPC chỉ là `driver = "grpc"` trong `connections.toml` (`target` = `host:port` trần hoặc URL đầy đủ, plaintext). `:TradarGrpcConnect [tên]` (bỏ trống thì chọn qua picker, ưu tiên connection của buffer request đang đứng nếu có), rồi `:TradarGrpcServices` liệt kê service đã biết (qua server reflection thật, hoặc rỗng nếu server không hỗ trợ — nạp `.proto` tay bằng `:TradarGrpcLoadProto <đường-dẫn>`). Chọn service rồi method mở một buffer JSON mới (tab mới) với nội dung `{}` — gõ request vào đó, `<leader>rr`/`:TradarGrpcSend` gọi thật (response hiện ở split riêng, `q` đóng split đó — buffer response không sửa được nên không mất gì), `<leader>rx`/`:TradarGrpcCancel` huỷ nếu đang chạy lâu, `g?` xem lại các phím này. Buffer request **không** nhận `q` (buffer này sửa được — để `q` nguyên nghĩa ghi macro của vim), đóng bằng `:q`/`:tabclose` mặc định. Chọn lại đúng connection/service/method đã mở sẽ nhảy về tab cũ thay vì mở thêm tab mới. Chỉ **unary** cho v1 (xem `docs/architecture.md`, "Chốt lại scope v1" mục gRPC) — chưa có skeleton tự sinh theo shape message, chưa streaming, chưa metadata/header.

### Kafka tail

Tail realtime một topic Kafka qua `tradar-server`, yêu cầu build server với `cargo build -p tradar-server --features kafka` (không nằm trong `default`, xem CLAUDE.md). `:TradarKafkaConnect [tên]` (picker nếu bỏ trống, lọc `driver = "kafka"`), rồi `:TradarKafkaTail [tên] [topic]` — bỏ trống phần nào thì hỏi qua `vim.ui.select`, luôn hỏi tail từ `latest` hay `earliest`. Mở một buffer transcript cuộn (tab mới, không sửa được, giống panel Topics mode của TUI) — mỗi dòng mới nhận được nối vào cuối (con trỏ chỉ bám theo khi đang ở dòng cuối; giữ 5000 dòng gần nhất), `q` dừng tail và đóng (buffer tự `bufhidden = wipe`, đóng cửa sổ cuối cùng là dừng luôn subscription phía server). `g?` xem lại phím. Khác gRPC/SQL: server **đẩy** dòng mới về (JSON-RPC notification `kafka.tail.message`, không phải trả lời một request), nên không có khái niệm "chạy xong" hay `<leader>rx` huỷ — chỉ có dừng hẳn bằng `q`. Mỗi lần `:TradarKafkaTail` là một consumer Kafka riêng (nhóm consumer group dùng-một-lần, không chia tải với ai) — tail hai lần cùng một topic thấy đúng hai luồng độc lập, không phải chia đôi dữ liệu.

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

Dòng mở fence là `` ```<dialect> [connection] `` — thiếu `connection` thì block đó rơi về connection của buffer (modeline `-- tradar: name`/`.tradar`/`:TradarConnect`, giống file thường). `<leader>rr`/`K`/`gd`/`<leader>re`/completion đều áp dụng cho **block đang chứa con trỏ**, không phải cả buffer — đặt con trỏ ngoài mọi block thì `<leader>rr` báo rõ "không ở trong block nào" thay vì chạy nhầm. `<leader>ra` (chạy cả file)/`<leader>rx` (huỷ)/visual-mode `<leader>rr` chưa áp dụng cách này — vẫn theo connection của buffer như trước.

Tô màu: `.tdb` dùng chung parser treesitter của Markdown (fenced code block tự inject theo tag — `sql` là grammar thật nên tô đúng cú pháp SQL; `mongo`/`esq` tô theo JavaScript/JSON qua cùng alias plugin đã đăng ký cho filetype `.mongo`/`.esq` riêng; `redis` không có grammar nào khớp nên hiện plain, giống hệt file `.redis` hiện tại). Suggestion (completion bảng/cột) theo đúng dialect của block, không phải LSP thật — LSP thật theo từng block (qua [otter.nvim](https://github.com/jmbuhr/otter.nvim)) để dành cho sau.

| Phím (tiền tố `<leader>r`) | Việc |
|---|---|
| `rr` | chạy statement dưới con trỏ (visual: phần chọn) |
| `ra` / `rx` | chạy cả file / huỷ query đang chạy |
| `rt` / `rh` | telescope: bảng (`<CR>` chèn tên, `<C-o>` mở bảng) / history (`<CR>` dán, `<C-r>` chạy) |
| `re` | `EXPLAIN` statement dưới con trỏ (không bao giờ `ANALYZE`, vì nó *chạy* câu lệnh) |
| `rs` / `rc` / `rm` | panel schema / chọn connection / tải thêm dòng |
| `K` | thông tin bảng/cột dưới con trỏ (kiểu, PK, FK, index; hiểu alias `o.user_id`); không biết thì rơi về hover của LSP |
| `gd` | trên tên bảng: mở bảng (`SELECT ... LIMIT 100`); chỗ khác: `definition` của LSP |
| `g?` | liệt kê phím của buffer hiện tại (SQL, kết quả, panel schema đều có) |

Trong buffer kết quả: `i` sửa ô (nhập giá trị — gõ `NULL` để đặt NULL), `dd` xoá dòng — cả hai chỉ **xếp thay đổi vào hàng đợi** (dòng được đánh dấu `● cột = giá trị` / `✗ delete`), chưa chạy gì; `<leader>rw` liệt kê các câu `UPDATE`/`DELETE` và chỉ chạy sau khi bạn chọn `Commit`, `<leader>ru` bỏ hết thay đổi đang chờ. Sau khi commit, kết quả tự làm mới và con trỏ ở nguyên dòng; dừng ở câu lỗi đầu tiên. Chỉ sửa được kết quả của truy vấn một bảng có khoá chính và có chọn cột khoá, nếu không sẽ báo lý do. `gd` trên ô của cột khoá ngoại chạy `SELECT` dòng được tham chiếu (kết quả của truy vấn một bảng), `gyc` ô, `gyr` dòng, `gyC` cột, `gyj`/`gyv`/`gym` cả kết quả (JSON/CSV/Markdown), `<leader>rv` đổi bảng ↔ JSON (cả kết quả SQL, dạng mảng object; ở view JSON các thao tác theo ô tạm tắt), `gK` xem cả dòng hiện tại mỗi trường một dòng (kiểu `\x`), `K` xem trọn giá trị một ô (hữu ích khi ô bị cắt bớt — xem dưới), `:TradarExport csv|json|md|tsv [file]`, `q` đóng. Cuộn tới cuối tự tải trang kế.

**Cột quá rộng tự bị cắt** (cap 40 ký tự hiển thị, giống TUI) — ô dài hơn hiện dấu `…` ở cuối thay vì kéo giãn cả cột; `K` trên ô đó xem giá trị đầy đủ. Chưa có cách resize tay như TUI (`<`/`>`) — nếu cần xem/copy giá trị đầy đủ, dùng `K` hoặc `gyc`.

Buffer kết quả **không có sort/filter theo cột** như TUI — chủ đích: Neovim đã có sẵn editor SQL thật ngay cạnh, sửa `ORDER BY`/`WHERE` rồi `<leader>rr` chạy lại tiện hơn một UI tương tác riêng.

Cần Neovim ≥ 0.10 (`:checkhealth tradar` kiểm tra). Statusline: `require('tradar').status()` (chuỗi rỗng ngoài buffer SQL).
Completion: nguồn `blink.cmp` `tradar.blink` (hoặc `omnifunc`).

## An toàn

Hỏi xác nhận (mặc định là `Cancel`, nên `<CR>` phản xạ là đáp án an toàn) trước khi chạy:

- `UPDATE`/`DELETE` **không có `WHERE`**, `DROP`, `TRUNCATE` — trên mọi connection.
- **Mọi lệnh ghi** (`INSERT`/`UPDATE`/`DELETE`/DDL...) trên connection "protected" — tên chứa một trong `setup{ protected = {"prod"} }` (mặc định `{"prod"}`, không phân biệt hoa thường). Statusline hiện `⚠` bên cạnh.
- Mongo: `drop()`, `dropDatabase()`, `deleteMany({})`/`updateMany({}, …)` (bộ lọc rỗng) luôn hỏi. Redis: `FLUSHALL`/`FLUSHDB`/`SHUTDOWN`. Elasticsearch: `DELETE …`, `_delete_by_query`. Trên connection protected: mọi lệnh ghi của ngôn ngữ đó.
- `<leader>ra` chạy cả file chỉ hỏi **một lần** cho cả lô.
- `setup{ confirm = false }` tắt hết.

Đây là phân tích theo token (bỏ qua comment và chuỗi), không phải parser: bắt các lỗi tay thường gặp, không phải rào chắn bảo mật.
