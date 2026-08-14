# Tradar

Tradar (tui+radar) hoặc (tuannm+radar) là một công cụ khám phá và truy vấn database chạy trong terminal (TUI), theo tinh thần của [LazyGit](https://github.com/jesseduffield/lazygit) hay [k9s](https://k9scli.io/) — nhưng dành cho database.

Nó cho bạn một giao diện duy nhất, điều khiển bằng bàn phím, để kết nối, browse, và query các database khác nhau, để bạn không phải context-switch giữa nhiều native CLI client. Tradar không tự bịa ra ngôn ngữ query riêng: bạn viết SQL thật với database SQL, Mongo Shell JavaScript thật với MongoDB, và Query DSL thật với Elasticsearch.

## Trạng thái

Pre-alpha, nhưng chạy được: `tradar` kết nối tới một instance PostgreSQL, SQLite, MongoDB, Elasticsearch, hoặc Redis thật, chạy query, và hiện kết quả trong terminal — connection picker → query screen → results, toàn bộ điều khiển bằng bàn phím. Query editor là một editor vim-modal tự viết (không dùng thư viện ngoài) — `Normal`/`Insert` mode, di chuyển kiểu vim (`h`/`j`/`k`/`l`, `gg`/`G`, `Ctrl-d`/`Ctrl-u`, `0`/`$`), `i`/`a`/`I`/`A`/`o`/`O` để vào Insert, `x`/`dd` để xoá. Trên kết nối PostgreSQL/SQLite, query được tô màu cú pháp SQL qua treesitter (`tree-sitter-sequel`); các driver còn lại (Mongo/Elasticsearch/Redis) dùng cú pháp tự chế không có grammar khớp nên hiển thị plain text. Query editor hỗ trợ nhiều dòng: `Enter` thường chèn một dòng mới, còn `Ctrl+Enter` (hoặc `F5`, vì không phải terminal nào cũng báo Ctrl+Enter riêng biệt) chạy **câu lệnh tại vị trí con trỏ** — một file có thể chứa nhiều câu, ranh giới câu do từng driver tự nhận biết (SQL theo `;` nhưng bỏ qua `;` trong chuỗi/comment/dollar-quote, nên câu trải nhiều dòng vẫn chạy đúng). `Ctrl+A` chạy tất cả các câu trong file theo thứ tự, dừng ở câu lỗi đầu tiên. `Ctrl+C` huỷ query đang chạy. Trên kết nối Elasticsearch, `Ctrl+Y` ghi request hiện tại thành lệnh `curl` vào `./tradar-query.sh` trong thư mục làm việc. `Ctrl+S` lưu buffer query editor ra file: gõ tên trần (`report`) thì lưu vào thư mục queries mặc định (`~/.config/tradar/queries/`, tự thêm `.sql`), gõ đường dẫn có `/` thì dùng nguyên văn. `Ctrl+O` mở picker chọn file: các file mở/lưu gần đây (đánh dấu `●`) trước, rồi phần còn lại trong thư mục queries; gõ để lọc, và nếu không khớp gì thì `Enter` mở luôn chuỗi vừa gõ như một đường dẫn. Danh sách recent lưu ở `~/.config/tradar/recent.toml` nên còn nguyên sau khi thoát app; `Ctrl+R` mở danh sách lịch sử query (most-recent-first, `j`/`k`/`gg`/`G`, `Enter` để load một entry vào editor) — cả ba hoạt động bất kể đang focus panel nào. Connection được thêm/sửa/xoá ngay trong app (`a`/`e`/`d` ở connection picker), lưu vào file TOML tại đường dẫn config của platform (xem `crates/tradar-core/src/storage/mod.rs`). Bảng kết quả căn cột theo giá trị rộng nhất (ô quá dài bị cắt kèm `…`) và đánh số thứ tự dòng ở cột đầu. Khi panel Results đang focus, con trỏ chạy theo **cell** chứ không chỉ theo dòng: `j`/`k` đổi dòng, `h`/`l` (hoặc `←`/`→`) đổi cột, và bảng tự cuộn ngang vừa đủ để cell đang chọn còn trên màn hình. `y` copy cả dòng. Với kết quả là một `SELECT` đơn bảng trên PostgreSQL/SQLite và bảng đó có khoá chính, `Enter` sửa giá trị cell và `d` xoá dòng — cả hai **không ghi thẳng**: chúng sinh ra câu `UPDATE`/`DELETE` tương ứng, hiện đúng câu lệnh sắp chạy và đợi bạn bấm `y`, rồi chạy xong thì tự đọc lại query để bạn thấy ngay kết quả. Trường hợp không sửa được (join, không có khoá chính, driver không hỗ trợ) sẽ nói rõ lý do. `Ctrl+B` mở **navigator** — một panel cây bên trái liệt kê mọi connection đã lưu, chứ không chỉ connection của tab hiện tại: `l`/`→` mở một connection ra thành bảng → cột (connection chưa kết nối thì mở chính là kết nối, và nó vào một tab riêng), `h`/`←` đóng, `j`/`k`/`gg`/`G` di chuyển, `Enter` trên một connection nhảy tới tab của nó và trên một bảng/cột thì chèn tên vào editor của đúng tab đó. `Ctrl+B` bấm tiếp sẽ đóng panel; phím nào navigator không dùng đến sẽ tự trả focus về query screen. Trong query screen, `Tab` xoay vòng giữa editor và results. Export tổng quát (ngoài curl export của Elasticsearch) chưa được xây dựng. Xem `docs/architecture.md` để biết hình dạng hệ thống, gồm cả phạm vi ngôn ngữ query của từng driver.

Bấm `?` ở bất kỳ màn hình nào để xem toàn bộ phím tắt đang có hiệu lực (danh sách này sinh thẳng từ keymap đang chạy, nên remap xong là nó tự đổi theo). Đáy màn hình luôn có một thanh gợi ý phím theo ngữ cảnh.

Nhiều tab/connection cùng lúc: `Ctrl+T` mở tab mới (quay về connection picker), `Ctrl+W` đóng tab hiện tại (không đóng được tab cuối cùng), `Ctrl+Left`/`Ctrl+Right` chuyển tab. Tab bar chỉ hiện khi có từ 2 tab trở lên. `Ctrl+Q` thoát app ngay lập tức từ bất kỳ đâu (không cần `Esc` về picker rồi `q`). Khi thoát, `tradar` nhớ lại tab nào đã connect, tab nào đang active, **và nội dung query editor của từng tab** (`~/.config/tradar/session.toml`), rồi khôi phục nguyên trạng ở lần chạy tiếp theo.

## Database

**Mục tiêu v1:** PostgreSQL, SQLite, MongoDB, Elasticsearch, Redis — mỗi cái là một connector crate riêng (implementation `QueryDriver` + `Connector`) với execution model riêng:

- **PostgreSQL / SQLite** — SQL thật, kết quả dạng bảng.
- **MongoDB** — một parser tối giản cho `db.<collection>.<method>(<json-args>)` (`find`, `aggregate`, `insertOne`, `insertMany`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`); không phải JS engine thật.
- **Elasticsearch** — một console kiểu Kibana Dev Tools: gõ `METHOD /path` cộng một JSON body tuỳ chọn, gửi thẳng tới cluster, không giới hạn ở Search API.
- **Redis** — một dòng lệnh mỗi lần chạy, parse thô theo khoảng trắng; `HGETALL` và `ZRANGE`/`ZREVRANGE ... WITHSCORES` được format JSON nhận biết kiểu, mọi lệnh khác dùng chuyển đổi RESP-to-JSON tổng quát.

**Dự kiến:** MySQL, MariaDB, ClickHouse

Hỗ trợ database mới được thêm dưới dạng một connector crate mới (`QueryDriver` + `Connector`), không đụng tới phần còn lại của ứng dụng — xem `docs/architecture.md`.

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

Context gồm `global`, `picker`, `query-screen`, `sidebar`, `results`, `list`, `prompt`, `completion` — bấm `?` trong app để xem toàn bộ lệnh và phím hiện hành của từng context. Chỉ những lệnh đó remap được; phím vim **bên trong** query editor (`i`/`a`/`o`/`x`/`dd`/`hjkl`...) cố định theo vim chuẩn.

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
