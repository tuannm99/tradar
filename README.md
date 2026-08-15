# Tradar

Tradar (tui+radar) hoặc (tuannm+radar) là một công cụ khám phá và truy vấn database chạy trong terminal (TUI), theo tinh thần của [LazyGit](https://github.com/jesseduffield/lazygit) hay [k9s](https://k9scli.io/) — nhưng dành cho database.

Nó cho bạn một giao diện duy nhất, điều khiển bằng bàn phím, để kết nối, browse, và query các database khác nhau, để bạn không phải context-switch giữa nhiều native CLI client. Tradar không tự bịa ra ngôn ngữ query riêng: bạn viết SQL thật với database SQL, Mongo Shell JavaScript thật với MongoDB, và Query DSL thật với Elasticsearch.

## Trạng thái

Pre-alpha, nhưng chạy được: `tradar` kết nối tới một instance PostgreSQL, SQLite, MongoDB, Elasticsearch, Redis, Cassandra, RabbitMQ, hoặc Kafka thật, chạy query hoặc browse/tail/publish (tuỳ loại backend), và hiện kết quả trong terminal, toàn bộ điều khiển bằng bàn phím. Sáu backend đầu là connection picker → query screen → results; Kafka/RabbitMQ không có ngôn ngữ query nên có màn hình riêng — sidebar topic/queue + panel chính, xem mục "Database" bên dưới. Query editor là một editor vim-modal tự viết (không dùng thư viện ngoài) — `Normal`/`Insert` mode, di chuyển kiểu vim (`h`/`j`/`k`/`l`, `gg`/`G`, `Ctrl-d`/`Ctrl-u`, `0`/`$`), `i`/`a`/`I`/`A`/`o`/`O` để vào Insert, `x`/`dd` để xoá, `u` undo và **`U`** redo (không phải `Ctrl-r` chuẩn vim — phím đó đã là mở query history). `v`/`V` vào Visual mode (charwise/linewise, vùng chọn tô nền), `y`/`d`/`x` yank/xoá vùng chọn, `c` xoá xong vào thẳng Insert tại điểm cắt; `dd`/`x`/`yy` nạp vào cùng một register, `p`/`P` dán sau/trước con trỏ. `/` tìm trong buffer (khác với `/` lọc kết quả — chỉ áp dụng khi đang ở panel results), nhảy tới match ngay khi gõ, `Enter` chốt, `Esc` trả con trỏ về chỗ cũ; `n`/`N` lặp lại tìm kiếm gần nhất, có wrap quanh buffer. Trên kết nối PostgreSQL/SQLite, `za` gấp/mở câu lệnh SQL nhiều dòng đang chứa con trỏ (hiện gutter `▸`/`▾` cùng số dòng bị ẩn, `j`/`k`/`gg`/`G` nhảy qua cả cụm đã gấp như một dòng duy nhất, và gõ vào một câu đang gấp sẽ tự mở nó ra trước) — không áp dụng cho Mongo/Redis/Elasticsearch, vì mỗi câu ở các driver đó vốn đã một dòng. Trên kết nối PostgreSQL/SQLite, query được tô màu cú pháp SQL qua treesitter (`tree-sitter-sequel`); các driver còn lại (Mongo/Elasticsearch/Redis) dùng cú pháp tự chế không có grammar khớp nên hiển thị plain text. Query editor hỗ trợ nhiều dòng: `Enter` thường chèn một dòng mới, còn `Ctrl+Enter` (hoặc `F5`, vì không phải terminal nào cũng báo Ctrl+Enter riêng biệt) chạy **câu lệnh tại vị trí con trỏ** — một file có thể chứa nhiều câu, ranh giới câu do từng driver tự nhận biết (SQL theo `;` nhưng bỏ qua `;` trong chuỗi/comment/dollar-quote, nên câu trải nhiều dòng vẫn chạy đúng). `Ctrl+A` chạy tất cả các câu trong file theo thứ tự, dừng ở câu lỗi đầu tiên. `Ctrl+C` huỷ query đang chạy. Trên PostgreSQL/SQLite, gõ và chạy `BEGIN`/`COMMIT`/`ROLLBACK` như SQL chuẩn để điều khiển transaction thủ công — mode bar của editor hiện badge "transaction open (F8 commit / F9 rollback)" khi có transaction đang mở, im lặng khi auto-commit (mặc định); `F8`/`F9` là phím tắt commit/rollback ngay không cần gõ lại câu lệnh. Không áp dụng cho Mongo/Redis/Elasticsearch. Trên kết nối Elasticsearch, `Ctrl+Y` ghi request hiện tại thành lệnh `curl` vào `./tradar-query.sh` trong thư mục làm việc. `Ctrl+S` lưu buffer query editor ra file: gõ tên trần (`report`) thì lưu vào thư mục queries mặc định (`~/.config/tradar/queries/`, tự thêm `.sql`), gõ đường dẫn tương đối có thư mục con (`reports/first`) thì join vào cùng thư mục queries đó (cũng tự thêm `.sql`); chỉ đường dẫn có tiền tố escape rõ ràng (`/...` tuyệt đối, `~/...`, `./...`, `../...`) mới dùng nguyên văn ra ngoài thư mục queries. Prompt hiện một dòng preview cập nhật theo từng ký tự gõ, cho biết chính xác sẽ lưu/mở vào đâu trước khi bấm `Enter`; lần `Ctrl+S` đầu tiên trong phiên (chưa lưu/mở gì) tự prefill sẵn thư mục con của file gần nhất thay vì luôn về root. `Ctrl+O` mở một **trình duyệt thư mục thật** trong thư mục queries (kèm breadcrumb hiện đang đứng ở đâu), mở thẳng vào thư mục lưu/mở gần nhất thay vì luôn bắt đầu từ root: các file mở/lưu gần đây (đánh dấu `●`) và thư mục con hiện ở gốc, `Enter` trên thư mục đi vào (không đóng picker), `Enter` trên file thì chọn, `Backspace` khi ô lọc rỗng đi lên một cấp (không thoát ra ngoài thư mục queries); gõ để lọc trong thư mục đang đứng, và nếu không khớp gì thì `Enter` mở luôn chuỗi vừa gõ như một đường dẫn. Danh sách recent lưu ở `~/.config/tradar/recent.toml` nên còn nguyên sau khi thoát app; `Ctrl+R` mở danh sách lịch sử query (most-recent-first, `j`/`k`/`gg`/`G`, `Enter` để load một entry vào editor) — cả ba hoạt động bất kể đang focus panel nào. Connection được thêm/sửa/xoá ngay trong app (`a`/`e`/`d` ở connection picker), lưu vào file TOML tại đường dẫn config của platform (xem `crates/tradar-core/src/storage/mod.rs`). Bấm `Enter` connect thì tiêu đề picker đổi thành "connecting to '&lt;tên&gt;'…" trong lúc chờ, nên không trông như app treo khi backend chậm trả lời; connect thất bại (kể cả do hết 5s timeout) hiện lỗi ngay trong một box riêng bên dưới danh sách. Bảng kết quả căn cột theo giá trị rộng nhất (ô quá dài bị cắt kèm `…`) và đánh số thứ tự dòng ở cột đầu. Khi panel Results đang focus, con trỏ chạy theo **cell** chứ không chỉ theo dòng: `j`/`k` đổi dòng, `h`/`l` (hoặc `←`/`→`) đổi cột, và bảng tự cuộn ngang vừa đủ để cell đang chọn còn trên màn hình. `y` copy cả dòng. `/` lọc bảng theo chuỗi bạn gõ (ngay khi gõ, không phân biệt hoa thường, khớp bất kỳ cột nào) — `Enter` giữ bộ lọc, `Esc` bỏ nó đi; cột số thứ tự vẫn giữ vị trí gốc trong result nên bạn luôn biết đang nhìn dòng thứ mấy. `Space` bật/tắt panel xem đầy đủ giá trị của cell đang chọn ở dưới bảng — pretty-print nếu giá trị đó là JSON object/array (cột jsonb chẳng hạn), còn lại hiện nguyên văn; panel tự đóng ngay khi con trỏ di chuyển sang cell khác. Với kết quả là một `SELECT` đơn bảng trên PostgreSQL/SQLite và bảng đó có khoá chính, `Enter` sửa giá trị cell và `d` xoá dòng — cả hai **không ghi thẳng**: chúng sinh ra câu `UPDATE`/`DELETE` tương ứng, hiện đúng câu lệnh sắp chạy và đợi bạn bấm `y`, rồi chạy xong thì tự đọc lại query để bạn thấy ngay kết quả. Trường hợp không sửa được (join, không có khoá chính, driver không hỗ trợ) sẽ nói rõ lý do. `Ctrl+B` mở **navigator** — một panel cây bên trái liệt kê mọi connection đã lưu, chứ không chỉ connection của tab hiện tại: `l`/`→` mở một connection ra thành bảng → cột (connection chưa kết nối thì mở chính là kết nối, và nó vào một tab riêng), `h`/`←` đóng, `j`/`k`/`gg`/`G` di chuyển, `Enter` trên một connection nhảy tới tab của nó và trên một bảng/cột thì chèn tên vào editor của đúng tab đó. Đứng trên một bảng/collection/index/key (không phải cột con), `c`/`r`/`u`/`d` chèn khung Create/Read/Update/Delete đúng cú pháp connector đó vào editor của tab connection đó — cột/giá trị dùng placeholder dạng `<tên_cột>` để bạn tự điền, không đoán giá trị giả. `Ctrl+B` bấm tiếp sẽ đóng panel; phím nào navigator không dùng đến sẽ tự trả focus về query screen. Trong query screen, `Tab` xoay vòng giữa editor và results. Mỗi connection đang mở tự kiểm tra còn sống mỗi 15 giây (một round trip rẻ tuỳ driver — `SELECT 1`, lệnh `ping`, `PING`, `GET /`); rớt kết nối hiện ngay bằng badge `● disconnected` màu đỏ ở query editor và marker `✗ disconnected` cạnh tên connection trong navigator, không cần đợi chạy query mới biết. `Ctrl+E` export kết quả hiện tại ra file — luôn export toàn bộ result gốc, bỏ qua filter `/` đang áp dụng. Format chọn theo đuôi file gõ trong prompt: `.csv` (chỉ áp dụng cho kết quả dạng bảng — Mongo/Elasticsearch báo lỗi gợi ý dùng `.json` thay vì tự làm phẳng JSON lồng nhau) hoặc `.json` (bảng thành mảng object mỗi dòng, Mongo/Elasticsearch pretty-print nguyên văn); đuôi khác báo lỗi ngay trong prompt. Không giống `Ctrl+S`/`Ctrl+O`, một tên trần không rơi vào thư mục queries và không được nhớ vào recent-files list — export không phải là query. Trên kết nối Redis, mở connection mặc định vào **browse mode**: sidebar bên trái liệt kê toàn bộ key kèm type (`j`/`k`/`gg`/`G` di chuyển, `Enter` mở), thay hẳn panel results bằng view chuyên biệt theo type (field/value cho hash, index/value cho list, member cho set, member/score cho zset, giá trị đơn cho string) — bảng đó xem/lọc/yank như mọi kết quả khác, chỉ không sửa được (Redis vẫn read-only). `Ctrl+G` chuyển sang **console mode** (gõ lệnh thô như trước, layout editor + results y hệt các driver khác); `Ctrl+G` lần nữa quay lại browse. Xem `docs/architecture.md` để biết hình dạng hệ thống, gồm cả phạm vi ngôn ngữ query của từng driver.

Bấm `?` ở bất kỳ màn hình nào để xem toàn bộ phím tắt đang có hiệu lực (danh sách này sinh thẳng từ keymap đang chạy, nên remap xong là nó tự đổi theo). Đáy màn hình luôn có một thanh gợi ý phím theo ngữ cảnh.

Nhiều tab/connection cùng lúc: `Ctrl+T` mở tab mới (quay về connection picker), `Ctrl+W` đóng tab hiện tại (không đóng được tab cuối cùng), `Ctrl+Left`/`Ctrl+Right` chuyển tab. Tab bar chỉ hiện khi có từ 2 tab trở lên. `Ctrl+Q` thoát app ngay lập tức từ bất kỳ đâu (không cần `Esc` về picker rồi `q`). Khi thoát, `tradar` nhớ lại tab nào đã connect, tab nào đang active, **và nội dung query editor của từng tab** (`~/.config/tradar/session.toml`), rồi khôi phục nguyên trạng ở lần chạy tiếp theo.

## Database

**Mục tiêu v1:** PostgreSQL, SQLite, MongoDB, Elasticsearch, Redis, Cassandra, Kafka, RabbitMQ — mỗi cái là một connector crate riêng. Sáu cái đầu implement `QueryDriver` + `Connector`, dùng chung UI query editor/results; Kafka/RabbitMQ implement `Session`/`Connector` trực tiếp và tự vẽ màn hình riêng, vì không có ngôn ngữ query để gõ.

- **PostgreSQL / SQLite** — SQL thật, kết quả dạng bảng.
- **MongoDB** — một parser tối giản cho `db.<collection>.<method>(<json-args>)` (`find`, `aggregate`, `insertOne`, `insertMany`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`); không phải JS engine thật. Field trong navigator suy ra bằng cách đọc mẫu một document mỗi collection (field lồng nhau hiện dạng `parent.child`, `_id` đánh dấu là khoá).
- **Elasticsearch** — một console kiểu Kibana Dev Tools: gõ `METHOD /path` cộng một JSON body tuỳ chọn, gửi thẳng tới cluster, không giới hạn ở Search API.
- **Redis** — một dòng lệnh mỗi lần chạy, parse thô theo khoảng trắng; `HGETALL` và `ZRANGE`/`ZREVRANGE ... WITHSCORES` được format JSON nhận biết kiểu, mọi lệnh khác dùng chuyển đổi RESP-to-JSON tổng quát.
- **Cassandra** — CQL thật qua crate `scylla` (thuần Rust, tương thích cả Apache Cassandra lẫn ScyllaDB), kết quả dạng bảng như SQL. `target` là contact point trần `host:port` (không phải URI); navigator duyệt hết mọi keyspace không phải hệ thống, tên bảng hiện dạng `keyspace.table`.
- **Kafka** — sidebar liệt kê topic (bỏ topic nội bộ `__*`); `Enter` tail message **real-time** từ latest offset, `b` từ earliest, `Space` pause/resume theo dõi (message vẫn được nhận ngầm lúc pause, không rớt). `p` publish một message (key tuỳ chọn + value) vào topic đang chọn. Dùng `rdkafka` (binding `librdkafka`, build từ vendored source qua CMake — cần `cmake`/`gcc`/`libcurl-dev` trên máy build). `target` là bootstrap servers trần (`host:port[,host:port...]`).
- **RabbitMQ** — sidebar 2 mode (`Ctrl+G` đổi): **Queues** (peek N message gần nhất **không phá huỷ** — requeue lại ngay sau khi đọc) và **Exchanges** (xem danh sách binding). `p` publish (chọn exchange + routing key + payload). Dùng Management HTTP API (`reqwest`) chứ không phải AMQP — nghĩa là `r` refresh/poll theo yêu cầu, không tail real-time như Kafka. `target` là URL đầy đủ tới Management API kèm credentials (`http://user:pass@host:15672/vhost`).

**Dự kiến:** MySQL, MariaDB, ClickHouse

Hỗ trợ database mới được thêm dưới dạng một connector crate mới (`QueryDriver` + `Connector` cho backend dạng query, hoặc `Session` + `Connector` tự viết `Screen` riêng cho backend không có ngôn ngữ query như Kafka/RabbitMQ), không đụng tới phần còn lại của ứng dụng — xem `docs/architecture.md`.

## Saved connections

Ngay trong connection picker: `a` thêm, `e` sửa, `d` xoá (hỏi xác nhận). Form gồm 3 field — `name`, `driver` (chọn trong danh sách connector mà bản build này có, dùng `←`/`→`), và `target` — `Tab` chuyển field, `Enter` lưu, `Esc` huỷ. Mọi thay đổi được ghi ngay vào file TOML tại đường dẫn config của platform (xem `crates/tradar-core/src/storage/mod.rs`).

Vẫn sửa tay file đó được nếu muốn; chỉ lưu ý là khi lưu từ UI, file bị ghi lại theo định dạng TOML chuẩn nên comment và thứ tự key tự sắp trong file viết tay sẽ mất. Mỗi entry là một table `[[connections]]` với `name`, `driver` (một connector id, xem `ConnectorDescriptor::id` của từng connector crate dưới `crates/connectors/`), và `target` mà định dạng tuỳ theo driver:

```toml
[[connections]]
name = "local postgres"
driver = "postgres"
target = "postgres://user:password@localhost:5432/mydb"

[[connections]]
name = "local sqlite"
driver = "sqlite"
target = "test.db"

[[connections]]
name = "local elasticsearch"
driver = "elasticsearch"
target = "http://localhost:9200"

[[connections]]
name = "local redis"
driver = "redis"
target = "redis://localhost:6379/0"

[[connections]]
name = "local mongo"
driver = "mongo"
target = "mongodb://localhost:27017/mydb"
```

`target` của MongoDB phải kèm theo một database path (`/mydb` ở trên) — `MongoDriver::connect()` báo lỗi "connection string must include a default database" nếu thiếu.

## Cấu hình: theme và keymap

Tuỳ chọn, nằm ở `~/.config/tradar/config.toml`. Không có file thì dùng mặc định; file hỏng thì `tradar` in cảnh báo ra stderr rồi chạy tiếp bằng mặc định (không chết app).

```toml
[theme]
# Màu theo vai trò, không phải theo tên màu cụ thể. Giá trị nhận tên màu
# ("red", "bright-blue"), mã hex ("#89b4fa"), hoặc chỉ số 256-color ("75").
border-focused = "#89b4fa"
error = "red"
syntax-keyword = "176"

[keymap.global]
new-tab = "ctrl-n"            # một phím

[keymap.list]
move-down = ["j", "down"]     # hoặc nhiều phím cho cùng một lệnh
move-top = "gg"               # hoặc chuỗi 2 phím
close-tab = []                # rỗng = gỡ bỏ phím
```

Context gồm `global`, `picker`, `query-screen`, `navigator`, `results`, `list`, `prompt`, `completion` — bấm `?` trong app để xem toàn bộ lệnh và phím hiện hành của từng context. Chỉ những lệnh đó remap được; phím vim **bên trong** query editor (`i`/`a`/`o`/`x`/`dd`/`hjkl`...) cố định theo vim chuẩn.

## Triết lý

- **Keyboard-first.** Mọi tính năng hoạt động được mà không cần chuột.
- **Terminal-first.** Khởi động nhanh, ít tốn bộ nhớ, không cần trình duyệt hay shell Electron.
- **Ngôn ngữ query gốc.** SQL cho database SQL, Mongo Shell JS cho MongoDB, Query DSL cho Elasticsearch — không phải một ngôn ngữ hợp nhất tự bịa ra.
- **Core không phụ thuộc database cụ thể.** Business logic không bao giờ phụ thuộc một database cụ thể; mỗi driver được cách ly sau một interface dùng chung.

## Build

Cần Rust (edition 2024).

```bash
cargo build   # build
cargo run     # chạy
cargo test    # test
cargo clippy  # lint
cargo fmt     # format
```
