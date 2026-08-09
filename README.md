# Tradar

Tradar là một công cụ khám phá và truy vấn database chạy trong terminal (TUI), theo tinh thần của [LazyGit](https://github.com/jesseduffield/lazygit) hay [k9s](https://k9scli.io/) — nhưng dành cho database. Tên là ghép từ tên tác giả (tuannm) và "radar".

Nó cho bạn một giao diện duy nhất, điều khiển bằng bàn phím, để kết nối, browse, và query các database khác nhau, để bạn không phải context-switch giữa nhiều native CLI client. Tradar không tự bịa ra ngôn ngữ query riêng: bạn viết SQL thật với database SQL, Mongo Shell JavaScript thật với MongoDB, và Query DSL thật với Elasticsearch.

## Trạng thái

Pre-alpha, nhưng chạy được: `tradar` kết nối tới một instance PostgreSQL, SQLite, MongoDB, Elasticsearch, hoặc Redis thật, chạy query, và hiện kết quả trong terminal — connection picker → query screen → results, toàn bộ điều khiển bằng bàn phím. Query editor hỗ trợ nhiều dòng: `Enter` thường chèn một dòng mới, còn `Ctrl+Enter` (hoặc `F5`, vì không phải terminal nào cũng báo Ctrl+Enter riêng biệt) chạy query. Trên kết nối Elasticsearch, `Ctrl+Y` ghi request hiện tại thành lệnh `curl` vào `./tradar-query.sh` trong thư mục làm việc. Chưa có màn hình "add connection" tương tác, nên saved connection phải được thêm bằng tay vào file TOML tại đường dẫn `tradar` in ra khi chưa có connection nào (xem `crates/tradar-core/src/storage/mod.rs`). Query screen có một schema sidebar (`Tab` để focus, `↑`/`↓` hoặc `j`/`k` để di chuyển, `Enter` để chèn tên table/collection/index/key được chọn vào query) tự động load khi connect. Multi-tab editing và export tổng quát (ngoài curl export của Elasticsearch) chưa được xây dựng. Xem `docs/architecture.md` để biết hình dạng hệ thống, gồm cả phạm vi ngôn ngữ query của từng driver.

## Database

**Mục tiêu v1:** PostgreSQL, SQLite, MongoDB, Elasticsearch, Redis — mỗi cái là một connector crate riêng (implementation `QueryDriver` + `Connector`) với execution model riêng:

- **PostgreSQL / SQLite** — SQL thật, kết quả dạng bảng.
- **MongoDB** — một parser tối giản cho `db.<collection>.<method>(<json-args>)` (`find`, `aggregate`, `insertOne`, `insertMany`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`); không phải JS engine thật.
- **Elasticsearch** — một console kiểu Kibana Dev Tools: gõ `METHOD /path` cộng một JSON body tuỳ chọn, gửi thẳng tới cluster, không giới hạn ở Search API.
- **Redis** — một dòng lệnh mỗi lần chạy, parse thô theo khoảng trắng; `HGETALL` và `ZRANGE`/`ZREVRANGE ... WITHSCORES` được format JSON nhận biết kiểu, mọi lệnh khác dùng chuyển đổi RESP-to-JSON tổng quát.

**Dự kiến:** MySQL, MariaDB, ClickHouse

Hỗ trợ database mới được thêm dưới dạng một connector crate mới (`QueryDriver` + `Connector`), không đụng tới phần còn lại của ứng dụng — xem `docs/architecture.md`.

## Saved connections

Chưa có màn hình "add connection" tương tác, nên connection được thêm bằng tay vào file TOML tại đường dẫn `tradar` in ra khi chưa có connection nào (xem `crates/tradar-core/src/storage/mod.rs`). Mỗi entry là một table `[[connections]]` với `name`, `driver` (một connector id, xem `ConnectorDescriptor::id` của từng connector crate dưới `crates/connectors/`), và `target` mà định dạng tuỳ theo driver:

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
