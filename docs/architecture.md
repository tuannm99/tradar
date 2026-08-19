# Architecture

Tài liệu này gồm hai phần: kiến trúc đang triển khai hiện tại, và kiến trúc mục tiêu ban đầu (một Cargo workspace với pipeline `Connector → Session → Screen`) để các hệ thống không có hình dạng "query" — message broker, hệ thống watch/inspect trạng thái sống, v.v. — có thể được thêm vào mà không phải đổi hình dạng core code. **Cập nhật 2026-08-10: cả 4 bước migration đã xong** (bước 1: tách workspace, 2026-08-08; bước 2 + đầu bước 3: tách `tradar-query-workbench` + thu hẹp `Action` + thêm `Connector`/`Session`/`Capability`, 2026-08-09, gộp vì phụ thuộc vòng nhau; bước 4: tách 5 driver thành connector crate riêng + `SavedConnection.driver` sang `String` id + registry, 2026-08-10). Phần "Kiến trúc mục tiêu" bên dưới giờ mô tả đúng những gì đã dựng cho 6 connector dạng query hiện có. **Cập nhật 2026-08-16**: hai connector đầu tiên trong nhóm không-phải-query — Kafka và RabbitMQ — đã được thêm vào, dùng đúng pipeline `Connector → Session → Screen` mô tả bên dưới (`KafkaScreen`/`RabbitScreen` là hai `Screen` đầu tiên không đi qua `tradar-query-workbench`). Kubernetes, SSH, Docker vẫn còn là "mục tiêu" đúng nghĩa — chưa có connector nào trong nhóm đó được thêm vào.

## Triển khai hiện tại

Tradar là một Cargo workspace gồm mười ba crate, cấu trúc sao cho ranh giới giữa các layer đã có hình dạng ranh giới crate, đúng theo hướng phụ thuộc mô tả ở "Bố cục workspace" bên dưới. **Cập nhật 2026-08-16**: các connector crate được đổi tên prefix `tradar-connector-<tên>` (trước đó `tradar-<tên>`) và chuyển ra sống trực tiếp dưới `crates/`, bỏ hẳn thư mục lồng `crates/connectors/` — dọn dẹp thuần cấu trúc, không đổi trait/API nào, làm cùng lúc với việc thêm connector thứ 9 (HTTP).

```
Cargo.toml                    [workspace], default-members = ["crates/tradar-app"]
crates/
  tradar-core/
    src/
      action.rs               — enum Action đóng (6 variant, 3 trong đó mang thêm field `tab: usize` từ 2026-08-12 — xem "Sessions/workspace state" trong docs/backlog/roadmap-sub-project.md) + trait Component (có tick() mặc định trả false)
      capability.rs           — enum Capability
      storage/                — saved connections + session state + query file/recent list dạng TOML (dùng crate `directories` để lấy config path); driver: String (connector id).
                                  QueryFiles (thư mục queries + recent list) là global của process như theme/keymap: screen được dựng bên trong connector nên luồn xuống
                                  sẽ phải nhét "file để ở đâu" vào SPI connector; init_query_files() gọi một lần trong main.rs, query_files() trả None nếu chưa init
      config/                — load ~/.config/tradar/config.toml → theme + keymap + vim_mode() (2026-08-13; trước đó là placeholder rỗng; vim_mode() thêm 2026-08-18 — OnceLock<bool> giống theme()/keymap(), đọc bởi QueryEngine::build_screen chứ không phải QueryEditorComponent::new() -- xem "Vim mode / Normal mode qua config" trong docs/backlog/mongo-es-completion-autoclose-vimconfig.md để rõ lý do)
      theme.rs                — bảng màu theo vai trò + override từ config
      keymap.rs               — Command × Context, resolve phím → lệnh, remap từ config, hỗ trợ chuỗi 2 phím (gg)
      ui.rs                   — widget dùng chung: panel có viền/focus, selection style, centered_rect, status hint bar, HelpOverlay, TextInput/TextArea, SplitPane, ContextMenu, yank/paste clipboard (OSC52 + arboard)
      vim_list.rs             — phép toán di chuyển selection dùng chung cho mọi list (và cho row movement của editor)
  tradar-connector-spi/
    src/lib.rs                — trait Connector, trait Session, struct ConnectorDescriptor,
                                  CONNECT_TIMEOUT + with_connect_timeout (giới hạn mở kết nối, mọi connector bọc qua)
  tradar-query-workbench/
    src/
      query_driver.rs         — trait QueryDriver (connect, list_schema, execute, export_curl, edit_source/edit_sql) + SchemaInfo/QueryResult/RowEdit
      query_engine.rs         — QueryEngine: nhận một chuỗi query, giao cho QueryDriver đang active, lưu lịch sử; implement Session
      components/             — QueryScreenComponent (implement Component), + query_editor.rs/results.rs/row_edit.rs/completion.rs/file_prompt.rs/file_picker.rs/history_picker.rs
                                  (struct state+draw thuần, do QueryScreenComponent compose và định tuyến phím tới, không tự implement Component)
  tradar-connector-postgres/  tradar-connector-sqlite/  tradar-connector-elasticsearch/  tradar-connector-redis/  tradar-connector-mongo/  tradar-connector-cassandra/
      src/lib.rs               — mỗi crate: struct driver (private, implement QueryDriver) + struct XConnector (private, implement Connector)
                                    + `pub fn connector() -> Box<dyn Connector>` (constructor export duy nhất ra ngoài crate)
  tradar-connector-rabbitmq/  tradar-connector-kafka/  tradar-connector-http/     — không phụ thuộc tradar-query-workbench (không có hình dạng query — xem "Kiến trúc mục tiêu" bên dưới)
      src/lib.rs               — struct XSession (private, implement Session) + struct XConnector (private, implement Connector) + `pub fn connector()`
      src/screen.rs            — struct XScreen (private, implement Component) — Screen tự viết, không dùng QueryScreenComponent
  tradar-app/                 [[bin]] name = "tradar"
    src/
      main.rs                 — dựng registry (HashMap<String, Box<dyn Connector>>) từ 9 connector(); event loop:
                                    crossterm input -> Component actions -> spawn Connector::connect -> Session -> Screen
      components/
        mod.rs                — RootComponent: tabs: Vec<Tab> (mỗi Tab: ScreenSlot::ConnectionPicker | Active(Box<dyn Component>) + connection_picker riêng + title) + active_tab
        connection_picker.rs  — ConnectionPickerComponent (list + add/edit/delete)
        connection_form.rs    — ConnectionFormComponent: form 3 field cho add/edit, overlay trên picker
```

`Action`/`Component` nằm ở `tradar-core` (đóng, 6 variant: `Quit`/`OpenRequested`/`Opened`/`OpenFailed`/`BackToPicker`/`ShowHelp` — đổi tên từ `Connect*` thành `Open*` đúng theo "RootComponent và Action" ở mục kiến trúc mục tiêu bên dưới; `ShowHelp` thêm 2026-08-13, vẫn đúng quy tắc "không connector nào thêm variant" vì overlay phím tắt là việc của app shell, không của connector). `QueryDriver`/`SchemaInfo`/`QueryResult`/`QueryEngine` cùng toàn bộ UI dạng query nằm ở `tradar-query-workbench`. `Connector`/`Session`/`ConnectorDescriptor` nằm ở `tradar-connector-spi`, cùng với `CONNECT_TIMEOUT`/`with_connect_timeout` — giới hạn thời gian mở kết nối mà **mọi** connector đều bọc qua, đặt chung một chỗ vì client bên dưới của mỗi backend bất đồng hoàn toàn về hành vi khi host không trả lời (sqlx có timeout riêng, `redis`/`mongodb` có default riêng, `reqwest` không có gì), mà TUI thì đứng im trong lúc connect nên treo lâu sẽ bị đọc là app hỏng. Mỗi driver cụ thể sống trong crate connector riêng của nó (`tradar-connector-<tên>`, dưới `crates/`); `tradar-app` phụ thuộc cả 9 (6 connector dạng query + Kafka + RabbitMQ + HTTP) nhưng không chứa code driver nào.

### Trait `QueryDriver`

Mỗi database backend dạng query implement chung một trait, định nghĩa ở `crates/tradar-query-workbench/src/query_driver.rs` (đổi tên từ `Driver` — tên cũ mơ hồ sau khi "connector" trở thành thuật ngữ chung cho mọi loại hệ thống, kể cả loại không có hình dạng query):

```rust
#[async_trait]
pub trait QueryDriver: Send + Sync {
    async fn connect(&mut self) -> anyhow::Result<()>;
    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>>;
    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult>;
    fn export_curl(&self, _query: &str) -> Option<String> { None }
    fn edit_source(&self, _query: &str) -> Option<String> { None }
    fn edit_sql(&self, _edit: &RowEdit) -> Option<String> { None }
    async fn ping(&self) -> anyhow::Result<()> { Ok(()) }
    fn in_transaction(&self) -> bool { false }
}
```

`edit_source`/`edit_sql` (thêm 2026-08-14) là cặp method đứng sau việc **sửa cell / xoá dòng ngay trên bảng kết quả**. Cùng một lý do như `export_curl`: chỉ driver mới biết cú pháp của chính nó, nên `tradar-query-workbench` không bao giờ tự viết câu SQL. `edit_source(query)` trả về tên bảng mà một result đọc từ đó (`None` = không xác định được ⇒ bảng kết quả ở chế độ chỉ đọc); `edit_sql(&RowEdit)` trả về câu lệnh thực hiện thay đổi. Hai connector SQL cùng uỷ quyền cho `query_driver::single_table_source` và `query_driver::build_sql_edit` — được phép dùng chung vì cả hai đều phụ thuộc crate này, còn connector thì không được phụ thuộc lẫn nhau (đúng pattern của `returns_rows`/`SQL_KEYWORDS`/`split_sql_statements`). Mongo/Redis/Elasticsearch giữ mặc định `None`: reply của chúng không phải dòng của một bảng nào có thể địa chỉ hoá được.

`RowEdit` mô tả thay đổi theo ngôn ngữ của *bảng đang nhìn* chứ không theo cú pháp của dialect nào: `{ table, key: Vec<(String, String)>, change: SetValue { column, value } | DeleteRow }`. `key` chính là khoá chính của dòng, lấy từ `SchemaInfo` — đây là lý do `ColumnInfo` có thêm field `primary_key`. Không có khoá chính thì không có mệnh đề `WHERE` nào chỉ đúng một dòng, và một câu lệnh có thể chạm nhiều dòng thì không được tự động chạy thay người dùng.

`in_transaction()` (thêm 2026-08-15) là toàn bộ bề mặt trait cần cho transaction control — không có `begin_transaction()`/`commit_transaction()` riêng, vì Postgres/SQLite tự xử lý `BEGIN`/`COMMIT`/`ROLLBACK` ngay bên trong `execute()` của chính chúng (`query_driver::transaction_control()` nhận diện từ khoá đầu câu và route sang một `tokio::sync::Mutex<Option<sqlx::Transaction>>` mà driver tự giữ, thay vì gửi văn bản đó xuống DB — cần thiết vì shape pool-mỗi-câu-một-connection của `execute()` không thể tự nhiên hiểu một `BEGIN` đứng riêng). Trait chỉ cần biết *có đang mở transaction hay không*, để UI hiện badge — Mongo/Redis/Elasticsearch giữ mặc định `false`, không có khái niệm này. Xem mục 3 "Transaction control" trong `docs/backlog/mockup-ui-2026-08-15.md` để biết `in_transaction()` được UI dùng ra sao.

`ping` (thêm 2026-08-15) là round trip rẻ nhất mà driver có để tự chứng minh còn sống — Postgres/SQLite `SELECT 1` qua pool, Mongo lệnh `ping`, Redis `PING`, Elasticsearch `GET /`. Mặc định `Ok(())` (coi như còn sống) là chủ đích: một driver không override thì hành xử y như trước khi method này tồn tại, không driver nào "tự nhiên" báo rớt kết nối. `QueryEngine::tick()` tự bắn `ping()` trong nền mỗi 15 giây (`PING_INTERVAL`, dùng `tokio::time::Instant` để test được bằng clock giả lập của `tokio::time::pause`), tối đa một lần gọi bay cùng lúc, và cập nhật `alive: bool` đọc qua `QueryEngine::alive()`. Đây là cách duy nhất TUI biết một connection rớt *trước khi* user chạy query vào nó và nhận lỗi — trước đây không có cách nào cả. `Component` có thêm `connection_alive() -> Option<bool>` (mặc định `None` — "không áp dụng", cho picker/overlay/mọi thứ không giữ connection riêng) để mang trạng thái đó từ `QueryEngine` (bên trong `tradar-query-workbench`) lên tới `QueryScreenComponent::draw` và tới navigator ở app shell — cùng contract "app chỉ chuyển tiếp, không hiểu nội dung" với `restore_state`/`outline`.

`export_curl` thay cho `Action::ExportCurl` cũ (một variant trong enum dùng chung mà chỉ Elasticsearch implement, buộc `main.rs` phải special-case theo `DriverKind`) — mặc định `None` ("không hỗ trợ export"), chỉ `ElasticsearchDriver` (trong `tradar-connector-elasticsearch`) override. Curl export giờ nằm gọn trong crate của riêng Elasticsearch (`QueryScreenComponent` chỉ gọi `self.engine.export_curl(query)`, không biết gì về ES) — mức cô lập cuối cùng đã đạt được, không còn "chờ bước 4" như ghi chú trước đây.

`SchemaInfo` và `QueryResult` là các shape đã chuẩn hoá mà phần còn lại của app render ra — driver có trách nhiệm dịch kết quả gốc của database sang các kiểu này. `QueryResult` là enum, không phải một struct duy nhất, vì kết quả dạng bảng (SQL) và kết quả dạng document (MongoDB, Elasticsearch, Redis) không cùng shape:

```rust
pub enum QueryResult {
    Table { columns: Vec<String>, rows: Vec<Vec<String>> },
    Documents(Vec<serde_json::Value>),
}
```

`Table` là kết quả Postgres và SQLite trả về. `Documents` dùng chung cho ba driver còn lại: mỗi hit/response của Elasticsearch, mỗi document của MongoDB, mỗi reply của Redis thành một `serde_json::Value` trong vec. `ResultsComponent` render `Table` thành bảng text và `Documents` thành các khối JSON pretty-print.

### Phạm vi ngôn ngữ query theo từng driver

Postgres, SQLite, và Cassandra chấp nhận SQL/CQL tuỳ ý (dùng chung `SQL_KEYWORDS`/`split_sql_statements`/`build_crud_snippet` — cú pháp CQL đủ gần SQL để không cần bản riêng). Ba driver còn lại chỉ chấp nhận một tập con hẹp, có chủ đích, không phải toàn bộ ngôn ngữ query gốc:

- **Elasticsearch**: mô phỏng theo Dev Tools console của Kibana, không phải client Search-only cố định. Dòng đầu là `METHOD /path` (vd `GET my-index/_search`); các dòng còn lại (nếu có) là JSON request body, gửi nguyên văn. Không có cấu hình auth/TLS client-cert, và mỗi lần chạy chỉ một request (không có script nhiều request). Toàn bộ JSON response được bọc thành một `Documents` một phần tử, không unwrap thành từng document theo hit. `Ctrl+Y` trên một kết nối Elasticsearch xuất request hiện tại thành lệnh `curl` ghi vào `./tradar-query.sh` (đường dẫn cố định, ghi đè mỗi lần export).
- **Redis**: một dòng lệnh duy nhất, tách theo khoảng trắng (không hỗ trợ quoting/escaping), gửi qua `redis::cmd`. Chuyển đổi kết quả chỉ nhận biết kiểu cho `HGETALL` (→ JSON object) và `ZRANGE`/`ZREVRANGE ... WITHSCORES` (→ mảng object `{member, score}`); mọi lệnh khác dùng chuyển đổi RESP-to-JSON tổng quát. Không có pipelining, transaction (`MULTI`/`EXEC`), pub/sub, hay xử lý riêng cho stream (`XADD`/`XRANGE`).
  - **Key browser** (2026-08-15, mục 4 "Mở rộng UI theo mockup" trong `docs/backlog/mockup-ui-2026-08-15.md`): `QueryScreenComponent` (`tradar-query-workbench`) có thêm một `mode: ScreenMode { Browse, Console }`, `Some` sidebar (`browse: Option<BrowseSidebarComponent>`) chỉ khi `connection.driver == "redis"` — cùng cơ chế match-theo-driver-id đã dùng sẵn cho `Dialect::Sql`, không thêm `Capability` mới cho một tính năng chỉ một connector dùng. Mở connection Redis mặc định vào `Browse` (sidebar liệt kê mọi key kèm type, thay hoàn toàn layout editor/results bằng sidebar/results); `F2` (`Command::ToggleBrowseMode`, context `query-screen`; đổi từ `Ctrl+G` sau khi rà lại keymap — mnemonic "G" không gợi gì tới "browse") chuyển sang `Console` — layout, editor, và mọi hành vi gõ lệnh tay giữ nguyên y hệt trước khi tính năng này tồn tại. `Enter` trên sidebar (`Context::Browse`, `Command::BrowseOpen`) gọi `QueryEngine::submit_browse` (song song với `submit_query`, không push `history`, cùng epoch/outcome channel) → `QueryDriver::browse_entry` — chỉ RedisDriver override (mặc định `bail!`). `browse_entry` tra `SchemaInfo.kind` (field mới, `None` ở bốn driver kia; RedisDriver điền qua `TYPE` mỗi key khi `list_schema` — giờ loop `SCAN` tới cursor 0 thay vì chỉ lấy batch đầu 100 key, cùng đánh đổi N round-trip đã chấp nhận với MongoDB's per-collection `find_one`) để chọn lệnh Redis đúng type (`GET`/`HGETALL`/`LRANGE key 0 -1`/`SMEMBERS`/`ZRANGE key 0 -1 WITHSCORES`), chạy qua `execute()` y hệt console mode, rồi reshape `Documents` JSON đó thành `QueryResult::Table` (field/value cho hash, index/value cho list, member cho set, member/score cho zset) — không đụng gì tới `execute()`/`shape_reply` nên console mode gõ tay các lệnh này vẫn ra `Documents` JSON như trước. Type ngoài 5 loại đã chọn (`stream`, ...) báo lỗi rõ ràng qua `results.set_error`, không có view riêng. Kết quả hiện thẳng trong `ResultsComponent` đã có sẵn — không sửa gì ở đó, vì `Table` là shape nó đã render từ trước; điều này cũng có nghĩa Redis vẫn read-only trong browse mode giống console mode (RedisDriver không override `edit_sql`/`edit_source`).
- **MongoDB**: một parser tối giản cho đúng shape `db.<collection>.<method>(<json-args>)` — không phải JS engine thật. Hỗ trợ `find`, `aggregate`, `insertOne`, `insertMany`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`. Không có method chaining (`.sort()`, `.limit()`), không có `$where`, không có bulk operation hay transaction; bất cứ gì ngoài shape này trả về lỗi "unsupported query".

**Cassandra** (2026-08-15, `tradar-connector-cassandra`, mục 7 "Mở rộng UI theo mockup" trong `docs/backlog/mockup-ui-2026-08-15.md`) — connector thứ 6, xây qua crate `scylla` (thuần Rust, nói CQL binary protocol thật, tương thích cả Apache Cassandra lẫn ScyllaDB). Khác các driver dạng URI (Postgres/Mongo), `target` là một contact point trần `host:port` (vd `127.0.0.1:9042`) — `SessionBuilder::known_node` chỉ cần đúng vậy, và Cassandra không có một database mặc định để gộp vào chuỗi kết nối. `list_schema` duyệt **mọi keyspace không phải hệ thống** (`system_schema.tables`/`system_schema.columns`, lọc `keyspace_name` bắt đầu bằng `system`), đặt tên `SchemaInfo` dạng `<keyspace>.<table>` giống cách Postgres ghi `public.users` (tái dùng nguyên `quote_identifier` đã tách theo dấu `.`). Một cột là "khoá" (`ColumnInfo.primary_key`) nếu `kind` của nó là `partition_key` hoặc `clustering` — CQL không có một cờ "primary key" đơn cột như SQL, nhưng partition key + clustering column cùng nhau chính là thứ một `WHERE` cần để trỏ đúng một row, đúng khái niệm `primary_key` đã dùng cho SQL. `execute()` với một câu không trả rows (INSERT/UPDATE/DELETE/DDL) luôn báo `Affected { rows: 0 }` — không phải rút gọn, CQL wire protocol không có khái niệm affected-row count cho các câu đó (`Void` result không mang gì khác). Không có `in_transaction`/transaction control (giữ default `false` như Mongo/ES/Redis) — Cassandra không có `BEGIN`/`COMMIT`/`ROLLBACK` kiểu SQL. Không override `edit_sql`/`edit_source` (sửa cell trực tiếp trong grid) ở bản đầu — cú pháp `WHERE` cho CQL cần đủ partition key + clustering columns mà `build_sql_edit`/`single_table_source` hiện chưa phân biệt được với SQL chuẩn, để dành hơn là bịa ra câu sai. `testcontainers-modules` không có feature `cassandra` (đã xác nhận qua `cargo add --dry-run` trước khi viết test) nên integration test dùng `testcontainers::GenericImage` trực tiếp với wait-strategy tự viết (đợi log `"Starting listening for CQL clients"`) và `with_startup_timeout` nới ra 150s — ảnh JVM này thường mất 60-90s mới sẵn sàng, vượt timeout mặc định 60s của `testcontainers`.

Hai điểm quan trọng phát hiện lúc root-cause integration test lúc đầu cứ timeout (đọc thẳng `system.local` qua `cqlsh` mới lộ ra, không phải do Docker chậm):
- **`connect()` thành công không có nghĩa query thật cũng chạy được.** `scylla` chỉ dùng contact point cho control connection; mọi query thật đi qua một pool connection riêng, mở tới địa chỉ Cassandra tự quảng bá qua `system.local.rpc_address` — mặc định là IP nội bộ Docker bridge, không route được từ ngoài container. Bắt buộc set env `CASSANDRA_BROADCAST_RPC_ADDRESS=127.0.0.1` (cả `docker-compose.yml` lẫn container test qua `with_env_var`) cho setup single-node/dev; vì Cassandra luôn tự quảng bá đúng port nội bộ 9042 (không cấu hình port quảng bá riêng được), container test còn phải ép port host trùng 9042 (`with_mapped_port`, không dùng `with_exposed_port` ngẫu nhiên) — nên không chạy song song được với `docker compose up cassandra` đang chiếm cùng port.
- **`SELECT` từ `system_schema.columns` không đảm bảo thứ tự dòng** — composite key có thể trả về sai thứ tự khai báo. `list_schema` phải lấy thêm cột `position` (0-based, tính riêng trong từng `kind`) và sort theo `(kind_rank, position)` trước khi build `ColumnInfo`.

### Keymap, theme và widget dùng chung

Thêm 2026-08-13. Ba module trong `tradar-core` mà **mọi** component phải đi qua thay vì tự làm:

- **`keymap`** — component không match `KeyCode` trực tiếp nữa; nó hỏi `keymap().resolve_in(&[context...], &mut pending, key)` và nhận về một `Command`. `Context` (`Global`/`Picker`/`QueryScreen`/`List`/`Prompt`) tồn tại vì cùng một phím mang nghĩa khác nhau tuỳ chỗ (`enter` = connect ở picker, = chèn tên schema ở query screen). `resolve_in` nhận *nhiều* context theo thứ tự ưu tiên, dùng chung một ô `pending` — đó là cách một màn hình check binding riêng của nó trước rồi mới rơi xuống điều hướng list dùng chung. `pending: Option<KeyPress>` do chính component giữ và chính là `pending_g: bool` cũ được tổng quát hoá cho chuỗi 2 phím bất kỳ, không riêng `gg`.
- **`theme`** — màu đặt tên theo *vai trò* (`border_focused`, `error`, `syntax_keyword`), không theo tên màu. Component không được viết `Color::Red` thẳng.
- **`ui`** — `panel()`/`selection_style()`/`centered_rect()`/`draw_status_bar()`/`HelpOverlay`. Nằm ở `tradar-core` vì cả `tradar-app` và `tradar-query-workbench` đều cần, mà hai crate đó không được phụ thuộc nhau.

Cả `theme` và `keymap` được nạp một lần lúc khởi động (`config::init`) vào một `OnceLock`, đọc qua hàm `theme()`/`keymap()` trả về `&'static` — nếu chưa nạp thì rơi về mặc định dựng sẵn. Chọn global thay vì truyền tham số xuyên mọi `handle_key_event`/`draw` là có chủ đích: nó tránh phải đổi signature của trait `Component` (và `Session::build_screen` ở `tradar-connector-spi`) chỉ để mang theo hai thứ config bất biến suốt vòng đời process. Đánh đổi: test không thay được keymap giữa chừng, nên logic remap được test trực tiếp trên `Keymap` ở `tradar-core`, còn component thì test theo binding mặc định.

Hai bất biến của phần dispatch phím, cả hai đều có test hồi quy trong `query_screen.rs` (dễ vô tình phá khi thêm binding mới):

1. **Ký tự thường gõ trong Insert mode luôn là text, không bao giờ là lệnh.** Không có quy tắc này thì bind `?` cho help sẽ khiến không gõ được dấu `?` vào query. Chỉ phím có `CONTROL`/`ALT` (hoặc phím không phải `Char`) mới đi tới keymap khi editor đang ở Insert mode.
2. **Lệnh gắn với một pane cụ thể chỉ chạy khi pane đó đang focus** (`required_focus`: `yank` → Results, `insert-name` → Sidebar). Bấm ở chỗ khác thì phím rơi xuống editor như thể không có binding — nên `enter` vẫn xuống dòng bình thường khi đang gõ query.

### Quy tắc cách ly (isolation rule)

Đây là quy tắc giữ cho driver pluggable, và được áp dụng ở mọi nơi, không chỉ ở top level — giờ được Cargo enforce, không chỉ quy ước:

- Code trong mỗi crate connector (`tradar-connector-*`, dưới `crates/`) chỉ implement `QueryDriver` (từ `tradar-query-workbench`) và `Connector`/`Session` (từ `tradar-connector-spi`); struct driver/connector cụ thể không `pub` ra ngoài crate, chỉ `pub fn connector()` được export.
- Code trong `tradar-query-workbench` (components, `query_engine`) chỉ phụ thuộc trait `QueryDriver` — không bao giờ phụ thuộc một connector crate cụ thể nào (thật ra không *thể* phụ thuộc — mỗi connector crate phụ thuộc `tradar-query-workbench`, không phải ngược lại, nên một cycle sẽ chặn ngay ở compile time nếu vi phạm).
- `tradar-app/src/main.rs` là nơi duy nhất biết toàn bộ tập connector (hàm `registry()` gọi `connector()` của cả 9 crate) — không connector crate nào phụ thuộc connector crate khác hay phụ thuộc `tradar-app`.

Thêm một database mới nghĩa là: tạo một crate connector mới `tradar-connector-<tên>` dưới `crates/` implement `QueryDriver` + `Connector`/`Session` + export `connector()`, thêm một dòng dependency vào `tradar-app/Cargo.toml`, và một dòng trong `registry()`. Việc này không bao giờ được yêu cầu sửa `tradar-query-workbench`, `tradar-connector-spi`, `tradar-core`, hay bất kỳ connector crate nào khác.

### Trạng thái hiện tại

Walking skeleton v1 chạy được từ đầu đến cuối: `tradar` load các saved connection từ `storage`, dựng registry từ 8 connector crate, kết nối qua `Connector` tương ứng với connector id đã chọn (Postgres, SQLite, Elasticsearch, Redis, MongoDB, Cassandra, RabbitMQ, hoặc Kafka, tất cả đều implement đầy đủ chạy được với backend thật). Sáu connector dạng query chạy query gõ vào query editor của `QueryScreenComponent` thông qua `QueryEngine`; Kafka/RabbitMQ browse/tail/publish qua `KafkaScreen`/`RabbitScreen` tự viết, không đi qua editor đó. Cả hai đường đều render kết quả hoặc lỗi thật.

Những phần còn mỏng/thiếu đáng chú ý:

- ~~Chưa có màn hình "add connection" tương tác~~ — đã có từ 2026-08-14: `a`/`e`/`d` trong connection picker, form ở `crates/tradar-app/src/components/connection_form.rs`; xem `docs/backlog/roadmap-sub-project.md` mục 5.5.
- `QueryDriver::list_schema` đã implement và test cho cả sáu driver, và đã nối vào TUI dưới dạng **navigator** (`crates/tradar-app/src/components/navigator.rs`) — một cây `connection → bảng → cột` phủ mọi connection đã lưu, không chỉ connection của tab hiện tại. Nó nằm ở app shell chứ không trong screen vì chỉ `RootComponent` biết có những connection nào khác và chúng đang mở ở tab nào; screen chỉ cung cấp dữ liệu qua `Component::outline()`. Cùng cách đó, `NavConnection.alive` (từ `Component::connection_alive()`) gắn `✗ disconnected` sau tên một connection đang mở tab mà ping nền vừa phát hiện rớt — xem `ping`/`connection_alive` ở mục "Trait `QueryDriver`" phía trên. **Cập nhật 2026-08-19**: cây không còn cố định 2 cấp — `SchemaInfo` thêm `schema`/`object_kind`, và `flatten_outline` (trong `query_screen.rs`, không phải `navigator.rs` — navigator chỉ vẽ theo `depth`/`is_object`, không biết gì về schema/kind) chèn thêm cấp schema/keyspace/database (Postgres/Cassandra/MongoDB) rồi, riêng Postgres, một cấp object-kind (Tables/Views/Functions/Procedures) khi có nhiều hơn 1 loại trong cùng schema. Đây là **đảo ngược một phần** quyết định "dừng ở 2 cấp" ở `docs/backlog/roadmap-sub-project.md` mục 6 — lý do dừng khi đó (mỗi connector trả gì khi không có khái niệm schema/view/function) vẫn đúng, chỉ là giờ đã trả lời được: SQLite/Elasticsearch/Redis tiếp tục không set hai field đó nên cây của chúng không đổi gì (`flatten_outline` bỏ qua thẳng cấp nào không có driver nào set). Chi tiết đầy đủ + quyết định thiết kế ở `docs/backlog/navigator-schema-level.md`.
- MongoDB (`list_schema`) suy ra field bằng cách đọc mẫu một document mỗi collection lúc connect — xem chi tiết và đánh đổi ở `docs/backlog/features-batch-2026-08-14.md` mục "Schema đọc được sâu hơn".
- `config/` là module placeholder rỗng; cấu hình app ngoài file connections chưa tồn tại.

## Kiến trúc mục tiêu: connector pluggable

Cả sáu driver dạng query dùng chung một shape: `connect → list_schema → execute(query) -> Table | Documents`, được enforce bởi trait `QueryDriver` duy nhất và UI `QueryScreenComponent` duy nhất ở trên. Shape đó không khớp với message broker — Kafka/RabbitMQ không phải "gửi một chuỗi query, nhận về rows" mà là browse-topic/queue, tail message theo thời gian thực, publish một message — hay các hệ thống watch/inspect trạng thái sống (Kubernetes, Docker, Prometheus) và công cụ dạng remote-shell (SSH) vẫn còn ở nhóm "chưa có connector nào". Cassandra (CQL) là ngoại lệ trong nhóm phi-query ban đầu: nó khớp shape query nên tái dùng được UI hiện tại luôn — đã làm xong (2026-08-15). Kafka và RabbitMQ thì không khớp shape query — mỗi cái tự viết `Session`/`Screen` riêng theo đúng phần "Kiến trúc mục tiêu" bên dưới — cũng đã làm xong (2026-08-16, xem `docs/backlog/mockup-ui-2026-08-15.md`).

Phần sau định nghĩa shape mà toàn bộ 6 connector dạng query hiện có (Postgres, SQLite, Elasticsearch, Redis, MongoDB, Cassandra) đã được xây theo, và là shape Kafka/RabbitMQ đã dùng để tự viết `Session`/`Screen` riêng (giờ là ví dụ thật, không còn chỉ là đặc tả) — cũng là shape các hệ thống phi-query còn lại (Kubernetes, SSH, ...) sẽ được xây vào khi chúng thực sự được lên kế hoạch. Xem "Triển khai hiện tại" ở trên để biết layout thật hiện có.

### Các quyết định

- **Pluggable = static/compile-time, không phải dynamic loading.** Không load `.so`/`.wasm`, không có hệ sinh thái plugin bên thứ ba. Mỗi connector là một Rust crate được compile thẳng vào binary `tradar`.
- **Tách thành Cargo workspace, mỗi module một crate.** Spec v1 đã gác lại việc này cho tới khi có "một lý do thứ hai cụ thể"; việc thêm các connector có hình dạng khác hẳn (message queue, hệ thống watch-based) bên cạnh các connector dạng query hiện có chính là lý do đó. Workspace biến quy tắc cách ly thành một sự thật của dependency graph trong Cargo, không chỉ là quy ước ghi bằng comment.
- **Mỗi connector sở hữu Screen riêng của nó**, không dùng chung shape query-editor. Các connector *có hình dạng query* (SQL, Mongo, ES, Redis, Cassandra) vẫn dùng chung một crate UI để không phải tự implement lại.
- **"Backend"/"Driver" đổi tên thành Connector** xuyên suốt (`PostgresConnector`, `KafkaConnector`, ...) — một database Postgres, một cluster Kafka, và một host SSH không phải "backend"/"driver" theo bất kỳ nghĩa chung nào mà cái tên cũ nắm bắt được.
- **Kết nối và dựng UI là hai việc khác nhau, do hai kiểu khác nhau đảm nhiệm:** một `Connector` tạo ra một `Session`; một `Session` tạo ra một `Screen`.

### Bố cục workspace

```
Cargo.toml                       [workspace]
crates/
  tradar-core/                   — Action, trait Component, Capability,
                                    storage (SavedConnection, ConnectionStore), config
  tradar-connector-spi/          — trait Connector, trait Session, ConnectorDescriptor, CONNECT_TIMEOUT
                                    (SPI dành cho connector — xem "Vì sao tách khỏi tradar-core" bên dưới)
  tradar-query-workbench/        — QueryScreenComponent, ResultsComponent, QueryEditorComponent,
                                    QueryEngine (implement Session), trait QueryDriver, SchemaInfo/QueryResult
  tradar-connector-postgres/  tradar-connector-sqlite/  tradar-connector-mongo/  tradar-connector-elasticsearch/  tradar-connector-redis/  tradar-connector-cassandra/
  tradar-connector-rabbitmq/  tradar-connector-kafka/  tradar-connector-http/
  tradar-app/ (binary crate)     — main.rs (registry + event loop), RootComponent, ConnectionPickerComponent
```

Toàn bộ layout trên đã dựng đúng như mô tả kể từ 2026-08-10. `tradar-query-workbench` là bản chuyển gần như nguyên khối từ `components/query_screen.rs` et al. trước migration -- một vài chỗ đổi hình dạng cho khớp `Session`, xem "Sai khác khi triển khai thật" ở mục "Screen không bao giờ làm IO" bên dưới. **Cập nhật 2026-08-16**: `tradar-kafka/` và `tradar-rabbitmq/` giờ tồn tại thật — hai connector phi-query đầu tiên, `KafkaSession`/`RabbitSession` là hai `impl Session` đầu tiên ngoài `QueryEngine`. Sau đó cùng ngày thêm connector phi-query thứ ba, HTTP (`tradar-connector-http`, `HttpSession`/`HttpScreen`), và toàn bộ 9 connector crate được đổi tên prefix `tradar-connector-<tên>` + chuyển ra khỏi thư mục lồng `connectors/` (không còn khớp cây thư mục vẽ ở trên tại thời điểm viết — xem "Triển khai hiện tại" ở đầu file để có cây đúng hiện tại). Các hệ thống phi-query còn lại theo kế hoạch (gRPC, Socket — xem "Thiết kế UI: HTTP, gRPC, Socket" bên dưới; Kubernetes, SSH, Docker vẫn chưa lên kế hoạch cụ thể).

`tradar-query-workbench` đặt tên là "workbench", không phải "ui" — nó gói cả editor, execution, và history cho các connector dạng query, không chỉ là widget.

**Vì sao tách `Connector`/`Session`/`ConnectorDescriptor` ra `tradar-connector-spi` thay vì để trong `tradar-core`** (quyết định 2026-08-08, trước khi bước 3 bắt đầu nên không tốn refactor gì): `Connector`/`Session` là một SPI (service provider interface) dành riêng cho connector, không phải domain cốt lõi của ứng dụng (state, action, config, storage). Nếu để chung trong `tradar-core`, crate này sẽ dần phình thành "God crate" chứa đủ thứ không liên quan tới nhau. Tách riêng giúp `tradar-core` tập trung vào state/action/config/hạ tầng ứng dụng, còn connector chỉ cần phụ thuộc `tradar-connector-spi` cộng các kiểu dùng chung cần thiết. **Đổi tên 2026-08-17**: crate này ban đầu tên `tradar-connector-api` — sau khi 9 connector crate được đổi sang prefix `tradar-connector-<tên>` (xem "Triển khai hiện tại"), tên `-api` đọc dễ nhầm với một connector tên "api" trong danh sách đó; đổi hậu tố thành `-spi` (đúng thuật ngữ đã dùng ngay câu trên) để phân biệt rõ "đây là hợp đồng phải implement" với "đây là một connector thật".

Hướng phụ thuộc, được Cargo enforce (không chỉ là quy ước):

- `tradar-core` không phụ thuộc gì trong workspace.
- `tradar-connector-spi` phụ thuộc `tradar-core` (cho `SavedConnection`, `Action`, `Component`).
- `tradar-query-workbench` phụ thuộc `tradar-core` và `tradar-connector-spi`.
- Mỗi connector crate luôn phụ thuộc `tradar-connector-spi`, và phụ thuộc `tradar-query-workbench` chỉ khi nó có hình dạng query (Postgres, SQLite, Mongo, Elasticsearch, Redis, Cassandra). Kafka, RabbitMQ, HTTP, và các connector không phải dạng query khác chỉ phụ thuộc `tradar-connector-spi`/`tradar-core` và tự dựng `Session`/`Component` implementation riêng.
- `tradar-app` là crate duy nhất phụ thuộc mọi connector crate. Không connector crate nào phụ thuộc connector crate khác, và không connector crate nào phụ thuộc `tradar-app`.

Hướng dẫn không ràng buộc: khi một connector crate lớn hơn phạm vi connection setup + execution + schema listing (completion, formatting, explain plan), tách nội bộ thành các submodule `client/`, `executor/`, `metadata/`. Chỉ làm việc này khi crate thực sự vượt quá độ rõ ràng của một file — không dựng sẵn khung bây giờ.

### Connector, Session, và Screen

```rust
#[async_trait]
pub trait Connector: Send + Sync {
    fn descriptor(&self) -> &ConnectorDescriptor;
    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>>;
}

pub trait Session: Send + Sync {
    /// Rút hết message từ channel nội bộ mà các background task của session này
    /// báo về, cập nhật state của chính nó. Có giới hạn mỗi lần gọi — xem
    /// "Screen never does IO" bên dưới. Trả về true nếu có gì đổi (cần vẽ lại).
    fn tick(&mut self) -> bool;

    /// `restore` là những gì `Component::restore_state` của screen này trả
    /// về lúc app tắt lần trước — `None` khi connect mới.
    fn build_screen(
        self: Box<Self>,
        action_tx: UnboundedSender<Action>,
        restore: Option<&str>,
    ) -> Box<dyn Component>;
}
```

**Cập nhật 2026-08-16**: chữ ký thật (`crates/tradar-connector-spi/src/lib.rs`) đã lệch khỏi bản đặc tả gốc theo hai chỗ trên — `tick()` trả `bool` thay vì `()` (để event loop bỏ qua vẽ lại khi không có gì đổi) và `build_screen` nhận thêm `restore: Option<&str>` (khớp `Component::restore_state`, không có trong đặc tả gốc vì lúc đó chưa tính khôi phục session qua restart). Khối code trên đã cập nhật theo code thật.

- **Connector**: factory gần như stateless. Nhận một `SavedConnection`, tạo ra một `Session`. Là giai đoạn duy nhất làm handshake ban đầu (mở kết nối TCP, xác thực, ping).
- **Session**: actor sống lâu dài. Sở hữu mọi thứ chạm vào IO hoặc tồn tại lâu hơn một khung render — connection/client handle, mọi background task nó spawn, một channel nội bộ để các task đó báo kết quả về, và cache (schema, topic metadata, mapping info). `KafkaSession` sở hữu producer, metadata client, và task tail (một `StreamConsumer` riêng mỗi lần tail) của nó; `RabbitSession` sở hữu `reqwest::Client` và cache queue/exchange/message của nó. **`Session` là thứ duy nhất trong pipeline này được phép spawn task hoặc sở hữu channel.**
- **Screen**: cái mà `RootComponent` thực sự giữ và route phím/draw tới — một giá trị implement `Component`. Nó đọc state của `Session` để render, và biến key event thành lời gọi command đồng bộ trên `Session` (vd `session.submit_query(text)`, `session.publish(topic, payload)`). Nó không bao giờ chạm socket, file, hay `tokio::spawn` trực tiếp.

Connector dạng query: `QueryEngine` (trong `tradar-query-workbench`) implement `Session` — `tick()` của nó rút các reply query-completion, `build_screen()` của nó trả về `QueryScreenComponent`. Connector phi-query (Kafka, RabbitMQ, từ 2026-08-16): `KafkaSession`/`RabbitSession` implement `Session` trực tiếp, `build_screen()` trả về `KafkaScreen`/`RabbitScreen` tự viết — hai `impl Session` đầu tiên ngoài `QueryEngine`, xác nhận pipeline này hoạt động đúng như đặc tả cho một shape hoàn toàn khác query.

Không cần trait mới cho việc phân biệt Screen/Component/widget — nó chỉ đặt tên cho một pattern code đã dùng sẵn. `QueryScreenComponent` là một Screen (implement `Component`), nhưng bên trong compose `query_editor.rs`, `results.rs`, và `row_edit.rs`, không cái nào tự implement `Component` — chỉ là struct state+draw thuần mà screen sở hữu và gọi trực tiếp. `KafkaScreen`/`RabbitScreen` theo cùng pattern: chỉ 1 file (`screen.rs`) mỗi cái, không tách widget con riêng vì sidebar+panel chính đủ đơn giản để giữ thẳng trong struct Screen (không có compose sâu như query editor).

### Screen không bao giờ làm IO — Session là actor

Một Screen không bao giờ được gọi `tokio::spawn` hay sở hữu channel trực tiếp. Nếu làm vậy, code UI của mỗi connector sẽ bị đan xen với IO/business logic của nó — đúng kiểu coupling mà quy tắc cách ly driver tồn tại để ngăn, chỉ là bị đẩy lên một layer cao hơn.

1. Một phím bấm hoặc lời gọi `update()` trên Screen biến thành một lời gọi method **đồng bộ** trên Session của nó (vd `self.session.submit_query(text)`, `self.session.start_tail(topic, from_beginning)`), trả về ngay lập tức.
2. Method đó của Session gọi `tokio::spawn` cho IO mà command cần, đưa cho task được spawn nửa `Sender` của một channel do Session sở hữu.
3. Event loop gọi `Component::tick()` trên screen đang active mỗi vòng lặp (mặc định trả `false`, không làm gì); `tick()` của Screen forward tới `self.session.tick()`.
4. `Session::tick()` rút channel nội bộ của chính nó — **có giới hạn** (vd tối đa 64 message mỗi lần gọi) — cập nhật state của chính nó, trả `bool` báo có gì đổi. Nó không bao giờ block.
5. Lời gọi `draw()` tiếp theo của Screen render bất cứ state nào Session đang giữ.

```rust
pub trait Component {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action>;
    fn handle_mouse_event(&mut self, _event: MouseEvent) -> Option<Action> { None }
    fn update(&mut self, action: Action) -> Option<Action>;
    fn tick(&mut self) -> bool { false }
    fn restore_state(&self) -> Option<String> { None }
    fn outline(&self) -> Vec<OutlineEntry> { Vec::new() }
    fn insert_text(&mut self, _text: &str) {}
    fn crud_snippet(&self, _name: &str, _op: CrudOp) -> Option<String> { None }
    fn outline_error(&self) -> Option<String> { None }
    fn connection_alive(&self) -> Option<bool> { None }
    fn status_hints(&self) -> Vec<crate::ui::Hint> { Vec::new() }
    fn draw(&mut self, frame: &mut Frame, area: Rect);
}
```

**Cập nhật 2026-08-16**: đây là chữ ký thật (`crates/tradar-core/src/action.rs`) — lớn hơn nhiều bản đặc tả gốc chỉ có 4 method, vì các đợt việc UI trước (navigator, CRUD snippet, connection-alive badge, help overlay) mỗi lần đều thêm 1 method mới **có default**, không đổi method nào đã có. Chỉ `handle_key_event`/`update`/`draw` là bắt buộc — một Screen mới như `KafkaScreen`/`RabbitScreen` chỉ cần implement đúng 3 cái đó cộng `tick` (để forward `Session::tick()`) và bất kỳ default nào nó thực sự cần override (`connection_alive`, `status_hints`); `outline`/`insert_text` để mặc định vì sidebar riêng của chúng không tham gia cây navigator (xem "Thiết kế UI: Kafka và RabbitMQ" bên dưới). `status_hints()` (thêm 2026-08-16 cùng đợt Kafka/RabbitMQ) sửa một bug thật: thanh status bar trước đó hardcode hint của `QueryScreenComponent` cho mọi screen active — sai ngay khi `KafkaScreen`/`RabbitScreen` tồn tại, vì "f5 run" không có nghĩa gì ở đó. Giờ mỗi Screen tự khai hint của mình, `RootComponent` chỉ vẽ.

Giới hạn này quan trọng khi một connector thực sự là một firehose — một Kafka consumer hàng nghìn message/giây, một Elasticsearch tail, một vòng scrape Prometheus. Một `while let Ok(msg) = rx.try_recv()` không giới hạn sẽ làm đói render; rút một số lượng cố định mỗi tick và để phần còn lại cho tick sau giữ UI phản hồi tốt bất kể throughput của producer — cùng kỹ thuật mà game engine và GUI framework (iced, egui) dùng cho event queue của chúng.

Đây là lý do vì sao không có `Action::Plugin(Box<dyn Any>)` (hay biến thể type-erased tương đương): nó vẫn sẽ yêu cầu `RootComponent` route một payload type-erased tới screen đang active, mất type safety và khả năng debug đúng chỗ quan trọng nhất — bên trong việc xử lý message riêng của một connector. Với mỗi Session sở hữu một channel riêng của kiểu message cụ thể của chính nó, không message nội bộ nào của connector từng phải băng qua ranh giới type-erased, và enum `Action` của `tradar-core` không bao giờ cần một catch-all variant hay một variant mới khi thêm connector.

Một ngoại lệ: lần **connect đầu tiên**, vì Session chưa tồn tại để spawn nó. `main.rs` của `tradar-app` tự spawn lời gọi `Connector::connect(...).await` (hậu duệ của `spawn_connect` hiện tại); một khi resolve thành một Session, mọi việc spawn task tiếp theo cho connection đó là việc của Session.

**Phát hiện khi triển khai thật (2026-08-09), không nằm trong đặc tả gốc ở trên:** task connect này không được phép tự dựng `Screen` (`Box<dyn Component>`) rồi gửi thẳng qua `action_tx` như đường ống `Action::Opened` phía trên gợi ý. `Component` không (và không thể) được ràng buộc `Send`, vì `QueryEditorComponent` giữ `edtui::EditorState`, bên trong có `Rc<RefCell<dyn ClipboardTrait>>` — không `Send`. Một future `tokio::spawn` thì bắt buộc `Send`, nên nếu future đó tạo và giữ một `Box<dyn Component>` (dù chỉ để gửi đi ngay), toàn bộ future bị compiler từ chối. Cách giải quyết trong `crates/tradar-app/src/main.rs`: một enum nội bộ `ConnectOutcome` (không phải `Action`, không đi qua `tradar-core`) mang `Box<dyn Session>` (đã `Send + Sync` sẵn) từ task `spawn_connect` về qua một channel riêng; event loop (chạy trên một task/thread duy nhất, không `spawn`) mới gọi `session.build_screen(action_tx.clone())` để dựng `Screen`, rồi mới bọc kết quả vào `Action::Opened` đưa cho `RootComponent::update`. Nói cách khác: `Box<dyn Session>` băng qua ranh giới `tokio::spawn`, `Box<dyn Component>` thì không bao giờ.

### ConnectorDescriptor và Capability

```rust
pub enum Capability {
    Query,
    Schema,
    Streaming,
    Publish,
    Tail,
    Explain,
    Export,
}

pub struct ConnectorDescriptor {
    pub id: &'static str,
    pub display_name: &'static str,
    pub icon: &'static str,
    pub capabilities: &'static [Capability],
}
```

Cho phép một connection picker hay một screen suy luận về việc một connector *có thể làm gì* mà không hardcode danh tính của nó. Ví dụ: Postgres/SQLite khai `[Query, Schema, Explain, Export]`; Redis khai `[Query, Streaming]`; Kafka khai `[Streaming, Publish, Tail]`; RabbitMQ khai `[Schema, Publish]` (không có `Streaming`/`Tail` — Management HTTP API không có endpoint streaming, xem "Thiết kế UI: Kafka và RabbitMQ" bên dưới). Vẫn chưa có gì dùng `Capability` để branch UI — nó được định nghĩa sẵn từ đầu vì retrofit sau khi đã có vài connector sẽ gây xáo trộn hơn nhiều so với định nghĩa shape từ đầu, và để chưa dùng thì không tốn gì cả.

### Registry

**Đã dựng (2026-08-10).** `SavedConnection.driver` (`tradar-core::storage`) là một `String` connector id (vd `"postgres"`, `"sqlite"`), không còn enum đóng `DriverKind` — file `connections.toml` trên đĩa không đổi format, vì `DriverKind` vốn đã serialize thành cùng chuỗi thường này (`#[serde(rename_all = "lowercase")]`).

- Mỗi connector crate export đúng một hàm `pub fn connector() -> Box<dyn Connector>`, mọi thứ khác trong crate (struct driver, struct connector) không `pub`.
- `crates/tradar-app/src/main.rs`'s `registry()` là nơi duy nhất biết toàn bộ tập connector: gọi `connector()` của cả 9 crate, dựng `HashMap<String, Box<dyn Connector>>` (key lấy từ `descriptor().id`) một lần lúc khởi động, bọc trong `Arc` để clone rẻ vào mỗi task connect. Thêm một connector nghĩa là thêm một dòng dependency trong `tradar-app/Cargo.toml` và một dòng trong `registry()` — không đổi `tradar-core`, `tradar-connector-spi`, `tradar-query-workbench`, hay bất kỳ connector crate nào khác.
- Một `connection.driver` id không match trong registry là lỗi runtime hiển thị cho người dùng qua `Action::OpenFailed` (`"unknown connector '{id}': not compiled into this build"`), không phải lỗi compile-time — kiểm tra trong `spawn_connect`.

### RootComponent và Action

**Đã dựng (2026-08-09).** Field cố định `query_screen: QueryScreenComponent` của `RootComponent` trở thành:

```rust
enum ScreenSlot {
    ConnectionPicker,
    Active(Box<dyn Component>),
}
```

`Action` thu hẹp lại còn đúng các event ở tầng application mà core cần, đổi tên `Connect*` → `Open*` (không phải mọi thứ đều là "connection" theo nghĩa Postgres/Kafka — một host SSH hay Docker daemon hợp với "open a screen for this target" hơn):

```rust
pub enum Action {
    Quit,
    OpenRequested { connection: SavedConnection, epoch: u64 },
    Opened { connection: SavedConnection, screen: Box<dyn Component>, epoch: u64 },
    OpenFailed { error: String, epoch: u64 },
    BackToPicker,
}
```

Enum này đóng và giữ đóng — không connector nào thêm variant, vì message nội bộ của connector không bao giờ băng qua ranh giới này. `tradar-core` không cần biết `QueryEngine`, `SchemaInfo`, hay bất kỳ kiểu riêng của connector nào là gì nữa; `Opened` chỉ mang theo Screen đã dựng sẵn. `SavedConnection`/`ConnectionStore` giữ nguyên tên hiện tại — "một cách đã lưu để tới một target" vẫn đúng kể cả với SSH/Docker/Kubernetes.

**Tác dụng phụ:** `Action::ExportCurl` (variant trong enum dùng chung mà chỉ Elasticsearch implement, buộc `main.rs` phải special-case nó) đã bị loại bỏ khỏi `Action`. Thay vào đó: `QueryDriver::export_curl(&self, query) -> Option<String>` (mặc định `None`) trong `tradar-query-workbench`, chỉ `ElasticsearchDriver` (sống trong crate riêng `tradar-elasticsearch`) override; `QueryScreenComponent` gọi `self.engine.export_curl(query)` khi `Ctrl+Y` mà không biết Elasticsearch là gì. Logic curl-export giờ nằm gọn trong đúng một crate, không rò rỉ vào `Action`/`main.rs`/`tradar-query-workbench` — mức cô lập đã dự tính ban đầu.

### Bức tranh tổng thể

```
App
 ├── Registry        (Connector id -> Connector, dựng một lần lúc khởi động trong tradar-app)
 ├── Navigation      (RootComponent: ConnectionPicker <-> Screen đang active)
 └── Screen                             — render, dispatch command
      │
      ▼
    Session                             — actor; sở hữu IO
      ├── Connection / client handle
      ├── Background task nó đã spawn
      ├── Channel nội bộ (bounded, rút có giới hạn trong tick())
      └── Cache (schema, topic, mapping, ...)
```

Luồng ví dụ (submit một query):

```
key press → QueryScreen.handle_key_event → QueryScreen.update() gọi
session.submit_query(text) trực tiếp (lời gọi sync, trả về ngay)
  → Session spawn một task → task await driver → task gửi kết quả vào channel của Session
  → tick tiếp theo của event loop: RootComponent.tick() → QueryScreen.tick() → Session.tick() rút (có giới hạn)
  → state của Session giờ giữ kết quả → QueryScreen.draw() render nó
```

### Đã cân nhắc và gác lại

Nêu ra trong lúc review thiết kế, chủ đích để ngoài shape mục tiêu ở trên cho tới khi có một connector cụ thể thực sự cần — mỗi mục có điều kiện kích hoạt để xem lại, giống cách spec v1 đã gác lại chính việc tách workspace:

- **Tách Session thành các sub-component Runtime/Store/Client.** Xem lại nếu implementation `Session` của một connector cụ thể (vd Kafka) lớn tới mức trách nhiệm của nó (connection, cache, task/channel plumbing) khó điều hướng trong một file.
- **`Arc<dyn Session>` / `SessionHandle` thay vì `build_screen(self: Box<Self>)`.** Thiết kế hiện tại cho Screen sở hữu độc quyền Session của nó. Chỉ xem lại khi có nhu cầu chia sẻ một Session giữa nhiều Screen — vd hai tab cùng trỏ vào một connection, hoặc reconnect-and-reuse.
- **`tick(cx: &mut Context)` mang theo delta/now/frame thay vì `tick(&mut self)`.** Xem lại khi một connector thực sự cần timer, animation, hay debounce logic phụ thuộc thông tin wall-clock/frame.
- **Lifecycle hook tường minh (`on_open`, `on_close`, `suspend`, `resume`, `dispose`).** Xem lại khi một connector giữ một resource cần shutdown có kiểm soát thay vì dựa vào `Drop` — vd một Kafka consumer cần rời group sạch sẽ.
- **`Capability` dạng bitflags thay vì enum thuần.** Xem lại nếu số lượng variant lớn tới mức (vài chục) một slice `&'static [Capability]` trở nên cồng kềnh so với kiểu flags.
- **Command pattern (`enum Command` gửi từ Screen tới Session) thay vì gọi method đồng bộ trực tiếp.** Xem lại nếu một tính năng cụ thể cần intercept/replay command — undo/redo, macro, session recording — hiện chưa có cái nào được lên kế hoạch.
- **`Component: Send`.** Trait hiện không (và ban đầu không thể) ràng buộc `Send` — lý do gốc là `QueryEditorComponent` giữ `edtui::EditorState`, bên trong có `Rc<RefCell<dyn ClipboardTrait>>` (xem `ConnectOutcome` trong "Triển khai hiện tại"). `edtui` đã bị bỏ (2026-08-12, xem `docs/backlog/roadmap-sub-project.md`) và editor tự viết hiện tại không có field non-`Send` nào rõ ràng — lý do gốc có thể không còn đúng, nhưng **chưa verify** toàn bộ implementor của `Component` (`RootComponent`, `ConnectionPickerComponent`, `QueryScreenComponent` và các sub-component của nó) thực sự `Send`, và việc bound `Send` sẽ đơn giản hoá `main.rs` (bỏ được channel `ConnectOutcome` riêng, dựng `Screen` thẳng trong task connect). Xem lại nếu channel `ConnectOutcome` này từng trở thành điểm khó bảo trì thật.
- ~~**`Workspace → Tab → Screen` thay vì một `ScreenSlot::Active(Box<dyn Component>)` duy nhất.**~~ Đã làm (2026-08-12), theo hướng thực dụng chứ không đúng nguyên văn: không có type `Workspace` mới, `RootComponent` (`crates/tradar-app/src/components/mod.rs`) đóng vai trò đó, đổi từ một `screen`/`connection_picker` sang `tabs: Vec<Tab>` + `active_tab: usize` (`Tab` gồm `screen: ScreenSlot`, `connection_picker` riêng, và `title` để tab bar có gì hiển thị). Xem chi tiết đầy đủ ở mục "Sessions/workspace state" trong `docs/backlog/roadmap-sub-project.md`. Split-view (nhiều tab hiển thị cùng lúc, không chỉ chuyển qua lại) vẫn chưa làm — xem lại nếu thực sự cần.
- **`ConnectorFactory` thay vì trait object `Connector` trực tiếp trong registry.** Xem lại nếu một connector cần khởi tạo non-singleton (vd instance riêng cho từng tab với config khác nhau), hiện chưa có gì cần việc này.

**Ghi nhận thêm 2026-08-08** — một buổi review kiến trúc rộng hơn đề xuất tổ chức lại toàn bộ dự án theo hướng "Engineering Workbench" thay vì theo từng database: thêm layer `Runtime` (workspace/scheduler/lifecycle/background task) tách khỏi App, tách UI thành `tradar-ui` (widget dùng chung) + `tradar-editor` (vim/textarea/cursor/completion) tách khỏi `tradar-query-workbench`, một loạt crate hạ tầng dùng chung (`tradar-table`, `tradar-tree`, `tradar-json`, `tradar-log`, `tradar-terminal`, `tradar-theme`, `tradar-keymap`, `tradar-command`, `tradar-icons`, `tradar-search`, `tradar-utils`, `tradar-plugin`), một AI service độc lập gắn vào editor, và các connector mới ngoài phạm vi hiện tại (Kubernetes, Docker, SSH). Tất cả các ý này **chưa được đưa vào shape mục tiêu ở trên** — cùng lý do như các mục phía trên: chưa có connector/tính năng cụ thể nào cần chúng, và phần lớn còn chưa nằm trong phạm vi v1/planned của `CLAUDE.md` hay `docs/roadmap.md`. Ghi lại ở đây để trace, không phải để scaffold ngay:
  - **`tradar-runtime`** (workspace manager, scheduler, lifecycle, background task tách khỏi App/Session) — xem lại khi có nhu cầu thật sự ngoài event loop hiện tại của `main.rs` (vd nhiều workspace/tab chạy song song thực sự cần điều phối).
  - **`tradar-ui` / `tradar-editor` tách khỏi `tradar-query-workbench`** — xem lại khi có Screen không phải dạng query cần dùng lại editor/widget (hiện `QueryEditorComponent` — editor vim-modal tự viết từ 2026-08-12, không còn phụ thuộc `edtui` — chỉ được dùng ở đây).
  - **Các crate hạ tầng dùng chung** (`tradar-table`, `tradar-tree`, `tradar-json`, `tradar-log`, `tradar-terminal`, `tradar-theme`, `tradar-keymap`, `tradar-command`, `tradar-icons`, `tradar-search`, `tradar-utils`) — xem lại từng cái khi code tương ứng thực sự tồn tại và bị dùng lại ở ≥2 chỗ; không tạo crate rỗng trước.
  - **`tradar-plugin`** — xem lại nếu dynamic loading (`.so`/`.wasm`) thực sự được quyết định làm, trái với "Non-goals" bên dưới hiện tại.
  - **AI service gắn vào editor** — xem lại khi tính năng AI cụ thể (completion, apply-patch, ...) được scope, hiện chưa có trong `docs/roadmap.md`.
  - **Connector Kubernetes/Docker/SSH** — cùng nhóm với Kafka/RabbitMQ ở trên: hợp lý về mặt shape (`Connector`/`Session`/`Screen` đã tính tới các hệ thống không phải query), nhưng chưa có trong danh sách connector v1/planned; thêm vào danh sách connector tương lai khi thực sự được lên kế hoạch.

### Thiết kế UI: Kafka và RabbitMQ (2026-08-16, đã code)

Chốt 2 quyết định UX còn treo ở `docs/backlog/mockup-ui-2026-08-15.md` mục 5 và 6 (mockup Screen 7 có Kafka nhưng thiếu chi tiết tail-mode; RabbitMQ chưa có mockup screen riêng) thành shape cụ thể, dựa trên `Connector`/`Session`/`Screen` đã đặc tả ở trên. **Cả hai đã implement xong trong cùng ngày** (`crates/tradar-connector-kafka`, `crates/tradar-connector-rabbitmq`) — mục này giữ nguyên làm tài liệu thiết kế, chỉ đánh dấu chỗ nào implementation lệch khỏi bản thiết kế gốc.

**Quy ước dùng chung cho cả hai**: mỗi Screen có sidebar bên trái (danh sách entity cấp 1 — topic/queue) + panel chính bên phải, và một phím toggle giữa 2 "mode" liên quan trong cùng connector — cùng ý tưởng `mode: ScreenMode{Browse,Console}` mà Redis đã lập cho `QueryScreenComponent`, áp dụng lại cho Screen non-query. Sidebar **không** tái dùng `navigator.rs`: contract của navigator là một cây `connection → table → column` đồng nhất qua mọi connector (`Component::outline()` giữ shell không biết "table" là gì); Kafka/RabbitMQ không có "column" theo nghĩa đó và có 2 loại entity cấp 1 khác nhau (topic vs queue/exchange), ép vào navigator sẽ buộc nó phải biết hình dạng riêng của từng connector — đúng thứ nó được thiết kế để tránh. Sidebar của Kafka/RabbitMQ là widget riêng do `KafkaScreen`/`RabbitScreen` sở hữu, giống cách `BrowseSidebarComponent` hiện do `QueryScreenComponent` sở hữu cho Redis.

**Kafka** — Capability `[Streaming, Publish, Tail]` (đã liệt kê sẵn ở mục Capability trên).

- Mode **Topics** (mode duy nhất trong v1 đã code — xem "Sai khác khi triển khai thật" ngay dưới): sidebar là danh sách topic (tên, số partition; `__` prefix — topic nội bộ của Kafka — bị lọc bỏ, giống cách bỏ `system*` keyspace của Cassandra). Panel chính là bảng message đang tail — cột partition/offset/key/value, dòng mới chèn cuối bảng và auto-scroll (kiểu `tail -f`/k9s log). `Enter` trên một topic bắt đầu tail từ latest offset; `b` tail từ earliest. `Space` pause/resume theo dõi — khi pause, `KafkaSession` vẫn nhận và giữ message trong buffer (không rớt, cap ở 500 message gần nhất — cũ hơn bị rớt để tránh leak trên topic chạy lâu), chỉ dừng auto-scroll UI (`paused_at_len: Option<usize>` đóng băng view ở độ dài buffer lúc pause). `r` refresh lại danh sách topic.
- Publish: `p` mở compose panel nhỏ (input key tuỳ chọn + input value 1 dòng). `Enter` gửi ngay — khác quy ước "hiện lệnh rồi `y` mới chạy" của row-edit trong query-screen, vì quy ước đó dành cho *hành động phá huỷ tự sinh từ dữ liệu đang xem* (UPDATE/DELETE suy ra từ một row); publish là nhập liệu trực tiếp, không có gì để "xem trước" ngoài chính nội dung người dùng vừa gõ.
- **Tail real-time, không phải poll**: `KafkaSession` sở hữu một task consumer chạy nền đẩy message qua channel nội bộ; `tick()` rút có giới hạn — đây chính là ví dụ firehose đã nêu ở "Screen không bao giờ làm IO" bên trên. Kiến trúc Session/tick vốn đã được thiết kế sẵn cho đúng use case này (bounded per-tick drain), nên poll theo interval chỉ là cùng cơ chế cộng thêm độ trễ, không đơn giản hơn. Verify tay: publish 1 message qua `kafka-console-producer` bên ngoài trong lúc đang tail trong app, message tự xuất hiện không cần thao tác gì.
- Client library: **`rdkafka`** (binding `librdkafka`, C, feature `cmake-build` để tự build từ vendored source, cần `cmake`/`gcc`/`libcurl-dev` trên máy build). Phá tiền lệ "ưu tiên pure-Rust, tránh toolchain C" đã chọn cho Cassandra (`scylla` thay vì `cassandra-cpp`) — lý do: `kafka-protocol` (pure Rust, giữ đúng tiền lệ) chỉ là tầng protocol thấp, phải tự dựng lại toàn bộ consumer-group join/sync/heartbeat state machine ở trên, tốn công không tương xứng với v1. Rủi ro build-time thật đã gặp lúc code: `cmake-build` cần header `curl/curl.h` (từ `libcurl4-openssl-dev`) dù cấu hình `WITH_CURL=0` — không có sẵn trên máy dev mặc định, phải cài thêm.
- **Sai khác khi triển khai thật**: mode **Groups** (lag theo consumer group) trong thiết kế gốc bị **cắt khỏi v1** — lấy lag đúng nghĩa cần dựng 1 consumer tạm với `group.id` được chọn rồi gọi `committed()`, phức tạp hơn đáng kể so với phần còn lại. Ghi vào `docs/roadmap.md` như fast-follow riêng, không chặn việc Topics mode chạy được đầu-cuối.

**RabbitMQ** — Capability `[Schema, Publish]` (không có `Streaming`/`Tail` — xem lý do dưới).

- Mode 1 **Queues** (mặc định): sidebar là danh sách queue trong vhost đang chọn (tên, ready/unacked/consumer count — lấy thẳng từ response Management API). Panel chính khi chọn 1 queue: bảng N message gần nhất — cột routing-key, exchange nguồn, redelivered, payload-preview. Không tự tail — phím `r` refresh 1 lần (xem lý do kỹ thuật dưới).
- Mode 2 **Exchanges**: sidebar là danh sách exchange (tên, type: direct/fanout/topic/headers). Panel chính khi chọn 1 exchange: danh sách binding (queue đích, routing key/pattern).
- Publish: `p` mở compose panel như Kafka (chọn exchange + routing key + payload), `Enter` gửi ngay — cùng lý do đã nêu ở Kafka.
- **Quyết định kỹ thuật chốt (giải quyết câu hỏi treo "Management HTTP API vs AMQP")**: dùng **Management HTTP API** qua `reqwest` (đã là dependency sẵn có nhờ driver Elasticsearch, không thêm crate mới) cho mọi thao tác — browse (`GET /api/queues`, `/api/exchanges`, `/api/exchanges/{vhost}/{exchange}/bindings/source`), xem message không phá huỷ (`POST /api/queues/{vhost}/{queue}/get` với `ackmode: "ack_requeue_true"` — RabbitMQ ack rồi requeue lại ngay, message không rời queue thật nên peek nhiều lần vẫn an toàn), và publish (`POST /api/exchanges/{vhost}/{exchange}/publish`). Đánh đổi: Management API không có endpoint streaming, không thể tail real-time như Kafka — chỉ poll theo yêu cầu người dùng (phím `r`), đây là lý do `Capability` của RabbitMQ không có `Streaming`/`Tail`, khác Kafka. Một AMQP client thật (vd `lapin`) sẽ mở consume/tail liên tục thật — gác lại, xem lại nếu "peek theo yêu cầu" không đủ dùng trong thực tế.
- `target` là một URL đầy đủ tới Management API, credentials trong userinfo, vhost là path segment đã percent-encode sẵn (vd `http://user:password@localhost:15672/%2f`, `%2f` là vhost mặc định `/`) — parse bằng `reqwest::Url::parse`, không thêm crate `url` riêng.
- RabbitMQ 4.x deprecate queue non-durable/non-exclusive theo mặc định (`transient_nonexcl_queues`) — phát hiện lúc viết integration test (`PUT /api/queues` với `durable: false` bị 400), không phải bug của connector, chỉ là hành vi mới của broker cần biết khi tạo queue thử.

**Non-goals của riêng thiết kế Kafka/RabbitMQ này** (ngoài các non-goal chung ở mục dưới): seek/reset consumer group offset (Kafka — cùng lý do mode Groups bị cắt khỏi v1); tạo/xoá queue, exchange, hay vhost (RabbitMQ — chỉ browse + publish, không quản trị); tail real-time cho RabbitMQ qua AMQP; bất kỳ UI branch nào theo `Capability` (giữ đúng non-goal chung đã có).

### Thiết kế UI: HTTP, gRPC, Socket (2026-08-16 — HTTP đã code, gRPC/Socket vẫn kế hoạch)

User yêu cầu bổ sung ba connector mới, **không phải database**: HTTP client kiểu Postman, gRPC client, và raw TCP socket kiểu netcat. Chốt qua `AskUserQuestion` trước khi viết thiết kế:

- **Thứ tự làm**: cả ba cùng lúc, không phân đợt theo sản phẩm (khác Kafka/RabbitMQ trước đây làm tuần tự) — nhưng mục "Gợi ý thứ tự build" cuối phần này vẫn đề xuất thứ tự *code* để giảm rủi ro lãng phí công sức, không mâu thuẫn với quyết định "cùng lúc" ở tầm sản phẩm.
- **gRPC**: cả hai — server reflection trước, fallback đọc file `.proto` khi server không bật reflection.
- **HTTP**: giao diện kiểu **Postman** (field URL/method/headers/body riêng), **không** tái dùng console kiểu Elasticsearch/`tradar-query-workbench` — quyết định này đảo lại đề xuất ban đầu của Claude (console, rẻ hơn) sau khi user phản hồi trực tiếp muốn Postman-style.
- **Socket**: netcat-style — kết nối giữ live, gửi/nhận theo dòng hoặc byte thô, không phải một request/response đơn lẻ rồi đóng.

Cả ba đều là connector **phi-query** theo đúng nghĩa mục trên: mỗi cái tự viết `Session`/`Screen` riêng (`crates/tradar-connector-http`, `tradar-connector-grpc`, `tradar-connector-socket`), **không cái nào phụ thuộc `tradar-query-workbench`** — đây là quy tắc cách ly đã nêu ở đầu CLAUDE.md ("code trong mỗi connector crate (`tradar-connector-*`, dưới `crates/`) chỉ implement `Connector`/`Session` và không phụ thuộc gì khác trong workspace"), áp dụng y hệt Kafka/RabbitMQ. Bản plan đầu tiên của mục này từng viết "tái dùng thẳng `QueryEditorComponent`/`ResultsComponent`" cho HTTP — đó là vi phạm quy tắc trên, bị bắt và sửa ngay khi bắt tay code HTTP (xem "Sai khác khi triển khai thật" cuối mục HTTP dưới đây); gRPC's phần dưới đã được cập nhật lại theo hướng đúng, chưa tự tái xác nhận bằng code vì gRPC chưa code. Điểm chung: sidebar riêng không tái dùng `navigator.rs` (cùng lý do đã nêu cho Kafka/RabbitMQ — mỗi connector có "entity cấp 1" hình dạng khác nhau, ép vào cây `connection → table → column` sẽ buộc navigator phải biết hình dạng riêng của từng cái).

**HTTP** (Postman-style, đã code 2026-08-16) — Capability `[Publish]`.

- `target` = base URL tuỳ chọn (v.d. `https://api.example.com`, có thể để trống nếu chỉ gõ URL đầy đủ mỗi lần) — field URL trong request builder prefill bằng base này, gõ đè được. `Connector::connect` không probe liveness (khác Rabbit/Kafka) — target có thể trỏ tới thứ chưa chạy, request đầu tiên mới thật sự biết sống hay không.
- Layout: **không có sidebar cố định** (khác plan gốc) — panel chính chia trên/dưới (khớp bố cục Postman thật hơn sidebar+panel của Kafka/RabbitMQ):
  - **Trên — request builder**: Method (đổi bằng `Ctrl+P`/`Ctrl+N`, cycle GET/POST/PUT/PATCH/DELETE/HEAD/OPTIONS, hoạt động bất kể đang focus field nào — không phải `Ctrl+Left`/`Ctrl+Right` như plan ban đầu: `Context::Global` bind hai phím đó cho `PrevTab`/`NextTab` và luôn được `RootComponent` resolve trước, nên một binding cùng phím ở `Context::Http` sẽ không bao giờ chạy tới; phát hiện lúc rà lại keymap 2026-08-18, xem `docs/backlog/keymap-and-performance-2026-08-18.md`), URL (`ui::TextInput`), Headers (`ui::TextArea` — widget mới, xem bên dưới — dạng `Key: Value` mỗi dòng, parse lúc gửi, không dựng bảng 2 cột riêng cho v1), Body (`ui::TextArea` cùng loại).
  - **Dưới — response**: status line (code/status text/số header/thời gian) + body cuộn qua `List` + `tradar_core::vim_list` (`j`/`k`/`gg`/`G`/`Ctrl-d`/`Ctrl-u`) — tự vẽ, không tái dùng `ResultsComponent`.
- **`ui::TextArea` (mới, `tradar-core`)**: textarea nhiều dòng thuần, không modal (khác `QueryEditorComponent` — không có Normal/Insert, không vim `hjkl`, chỉ arrow keys + Enter/Backspace/Delete/Home/End, giống `TextInput` nhưng nhiều dòng) — đặt ở `tradar-core` (không phải `tradar-http`) vì không có gì HTTP-riêng trong đó và gRPC's request/response JSON editor sẽ cần đúng thứ này.
- Phím: `Tab`/`Shift+Tab` (`Command::NextField`/`PrevField`, tái dùng đúng 2 command Rabbit's compose overlay đã dùng) xoay vòng focus qua 4 pane (Url → Headers → Body → Response), `Ctrl+Enter`/`F5` gửi (`Command::HttpSend`), `Ctrl+K` lưu request hiện tại (prompt đặt tên, tái dùng UX thư viện snippet dù storage khác), `Ctrl+L` mở overlay picker request đã lưu (tự viết, không tái dùng `SnippetPickerComponent` vì shape dữ liệu khác), `y` khi Response focus yank body qua `ui::yank_to_clipboard` (mới, xem dưới). 3 `Context` mới: `Http` (chỉ bindings không-in-được — mọi field luôn ở "insert mode" nên một binding chữ cái ở đây sẽ không gõ được chữ đó), `HttpResponse` (`y`, tách riêng vì là chữ cái), `HttpRequests` (overlay picker).
- Storage: `SavedHttpRequest { name, method, url, headers, body }` + `HttpRequestStore`/`HttpRequests` (`http_requests.toml`) trong `tradar-core::storage`, đúng pattern `SavedSnippet`/`SnippetStore`/`Snippets` — **không** scope theo driver như snippet (một request HTTP dùng lại được cho mọi connection HTTP, không có khái niệm "sai driver" như SQL snippet).
- Client: `reqwest` — đã là dependency sẵn có (Elasticsearch, RabbitMQ), không thêm crate.
- **Tiện thể dedupe**: `yank_to_clipboard` (OSC52) trước ở `tradar-query-workbench`'s `query_screen.rs`, chuyển sang `tradar_core::ui::yank_to_clipboard` (kéo theo dời `base64` dependency sang `tradar-core`) vì giờ có 2 nơi cần: `query_screen.rs` và `HttpScreen`.
- Non-goal v1 (giữ nguyên từ plan, chưa làm): không có biến môi trường/template kiểu `{{baseUrl}}`; không có UI riêng cho auth scheme (Bearer/Basic — gõ thẳng header `Authorization`); không multipart/form-data (chỉ raw body text); không request chaining/pre-request script; không hiện danh sách response header đầy đủ (chỉ đếm số lượng trong tiêu đề panel).
- **Sai khác khi triển khai thật**: (1) không tái dùng `QueryEditorComponent`/`ResultsComponent` như plan gốc — vi phạm quy tắc cách ly connector, sửa bằng `ui::TextArea` mới + response pane tự vẽ, xem đầu mục lớn ở trên; (2) không có sidebar cố định danh sách request đã lưu như plan mô tả — chỉ giữ overlay picker (`Ctrl+L`), vì plan có cả sidebar cố định lẫn "Ctrl+L mở picker" cùng lúc là thừa nhau; đơn giản hơn mà vẫn đúng tinh thần Postman (sidebar của Postman thật cũng ẩn được). Chi tiết đầy đủ, gồm bug lúc viết integration test (gunicorn log ra stderr không phải stdout) và toàn bộ test đã chạy, xem `docs/backlog/http-connector.md`.

**gRPC** — Capability đề xuất `[Schema, Publish]` (Schema vì reflection cho ra cây service/method browse được, Publish vì invoke là gửi rồi nhận).

- `target` = `host:port` trần (cùng quy ước Cassandra/Kafka).
- **Khám phá service/method** (điểm phức tạp nhất của cả ba connector mới): thử **server reflection** trước (gọi service `grpc.reflection.v1alpha.ServerReflection` chuẩn của gRPC) để lấy `FileDescriptorProto`, build `prost_reflect::DescriptorPool` — không cần user cung cấp gì ngoài address. Nếu server không hỗ trợ (lỗi khi gọi reflection), sidebar hiện rỗng kèm thông báo, phím `L` mở prompt đường dẫn `.proto` (tái dùng shape `ui::TextInput`/`FilePromptComponent`), parse bằng **`protox`** (pure-Rust protobuf compiler — tránh phụ thuộc `protoc` phải cài sẵn trên máy chạy, giữ đúng ưu tiên "pure Rust khi có thể" đã chọn cho Cassandra qua `scylla` thay vì `cassandra-cpp`) ra `FileDescriptorSet`, nạp vào cùng `DescriptorPool`.
- Message không biết shape lúc compile-time — không dùng `prost`/`tonic` codegen từ `.proto` cố định như cách dùng thông thường, mà dùng **`prost-reflect::DynamicMessage`** (dựng/đọc message tuỳ ý lúc runtime từ `DescriptorPool`, có hỗ trợ serde để map JSON ⇄ message theo đúng "protobuf canonical JSON mapping"). Đây là kỹ thuật `grpcurl` (Go) đã dùng, bản Rust tương đương dựa trên `prost-reflect`.
- Screen: sidebar service → method (2 cấp, cây riêng không qua navigator — cùng lý do đã nêu). `Enter` trên method nạp một request skeleton (JSON rỗng/giá trị mặc định theo field của input message, sinh từ descriptor — cùng ý tưởng "sinh khung" như `build_crud_snippet`) vào **editor request — `ui::TextArea`** (widget mới dựng cho HTTP, xem mục HTTP ở trên — không phải `QueryEditorComponent`, connector này không được phụ thuộc `tradar-query-workbench`) để gõ/sửa JSON. `Ctrl+Enter`/`F5` invoke. Response: tự vẽ theo cùng mẫu response pane của HTTP (status/metadata + body cuộn qua `List`/`vim_list`, không phải `ResultsComponent`) — unary = 1 message hiện trọn; server-streaming = mỗi message nhận thêm append vào, giống cách Kafka tail append message mới.
- **Đề xuất cắt phạm vi v1 (cần user xác nhận trước khi code, chưa tự quyết)**: chỉ hỗ trợ **unary + server-streaming**, **không** làm client-streaming/bidi-streaming — hai loại đó cần gửi nhiều message tương tác trong lúc đang nhận, phức tạp hơn đáng kể so với phần còn lại (đúng lý do Kafka từng cắt mode Groups khỏi v1). Cũng chưa làm metadata/header của request (gRPC có khái niệm tương đương HTTP header) — để dành fast-follow, giữ v1 tối giản như RabbitMQ đã làm.
- Crate: `tonic` (chỉ phần client, không cần codegen server), `prost` + `prost-reflect` (dynamic message + serde JSON mapping), `protox` (parse `.proto` không cần `protoc`). **Rủi ro cao nhất trong cả ba** — chưa từng làm trong codebase này, khả năng thiết kế lệch khi code thật (kiểu "Sai khác khi triển khai thật" đã thấy ở Kafka) là cao nhất; nên spike/prototype phần reflection + dynamic message trước khi cam kết UI chi tiết.

**Socket** (raw TCP, netcat-style) — Capability đề xuất `[Streaming]`.

- `target` = `host:port` trần.
- `SocketSession` sở hữu `tokio::net::TcpStream`, một task nền đọc liên tục đẩy chunk nhận được (`Vec<u8>`, decode UTF-8 khi hợp lệ) qua channel nội bộ; `tick()` rút có giới hạn — đúng pattern firehose-safe đã đặc tả (giống hệt Kafka tail, chỉ khác nguồn dữ liệu). Buffer cap một số lượng chunk gần nhất (giống cap 500 message của Kafka) để không leak trên kết nối sống lâu.
- Screen: **không có sidebar** (không có khái niệm "topic/queue" — kết nối chính là phiên duy nhất). Một panel cuộn hiện dữ liệu nhận được, mỗi entry có timestamp, decode UTF-8 khi được còn không thì hiện dạng hex dump (kiểu `xxd`) để không mất thông tin với giao thức nhị phân. Một dòng input (`ui::TextInput`) ở đáy — gõ rồi `Enter` gửi, có toggle tự thêm `\n` cuối dòng khi gửi (mặc định bật — khớp phần lớn giao thức dòng lệnh văn bản: RESP inline, SMTP, IRC...). `r` ngắt và kết nối lại (không có cơ chế tự retry giống connection pool của DB).
- Non-goal v1: UDP (user chỉ được hỏi về TCP), TLS (`rustls`/`native-tls` — plain TCP trước, để dành fast-follow), không parse/frame theo bất kỳ giao thức cụ thể nào — đúng chủ đích: đây là công cụ cho lúc *chưa có* connector riêng cho thứ đang cần xem, không phải một connector giao thức mới.

**Chung cho cả ba, chưa làm — ghi để tránh quên khi bắt tay code**:

- 3 `Context` mới trong `tradar_core::keymap` (`Http`, `Grpc`, `Socket`), mỗi cái tự khai binding riêng như `Context::Rabbit`/`Context::Kafka` đã làm, kết hợp `Context::List` cho sidebar ở HTTP/gRPC (Socket không có sidebar nên không cần).
- `main.rs`'s `registry()` thêm 3 dòng.
- README.md/`docs/backlog/` cập nhật khi từng connector thật sự chạy được đầu-cuối, đúng quy ước đã làm với Cassandra/Kafka/RabbitMQ.

**Gợi ý thứ tự build** (không phải thứ tự sản phẩm — user đã chốt "cả ba cùng lúc" ở tầm đó; đây chỉ là gợi ý kỹ thuật để tránh lãng phí công sức nếu một phần bị pivot giữa chừng): **HTTP trước** — xong (2026-08-16), xác nhận pattern "Screen phi-query tự vẽ + `ui::TextArea` dùng chung" chạy tốt, và bắt được ngay lỗi cách ly (định tái dùng `QueryEditorComponent`/`ResultsComponent` — không được phép, xem mục HTTP ở trên) trước khi lỗi đó lặp lại ở gRPC. Còn lại: **Socket** (đơn giản kỹ thuật nhất, không phần nào mới ngoài chính TCP) → **gRPC** (rủi ro cao nhất, giờ đã có kinh nghiệm build UI phi-query mới từ HTTP). Đây chỉ là đề xuất — hỏi lại trước khi bắt đầu code nếu muốn thứ tự khác.

### Query/HTTP screen: layout ngang/dọc + zoom, chuột trái/phải/giữa (2026-08-17, đã code)

User yêu cầu: (1) editor/results (và HTTP request/response) đổi được layout ngang/dọc cộng zoom in/out; (2) mọi thứ bàn phím làm được thì chuột cũng phải làm được, chuột trái/phải/giữa dùng hợp lý. Chốt qua `AskUserQuestion` trước khi code: zoom = phóng to panel đang focus (resize tỉ lệ split, **không** full-screen ẩn hẳn panel kia); layout ngang/dọc áp dụng cho cả query screen lẫn HTTP screen; chuột phải = context menu, chuột giữa = paste.

**`tradar_core::ui::SplitPane`** (widget dùng chung mới) — một split 2 pane (primary/secondary) tự nhớ orientation (`Vertical`/`Horizontal`) và tỉ lệ (`primary_percent`, mặc định 50). `toggle_orientation()` lật ngang/dọc; `zoom_in`/`zoom_out(focus_is_primary: bool)` phóng to/thu nhỏ đúng pane đang focus, kẹp trong `[20, 80]` (`MIN_SPLIT_PERCENT`/`MAX_SPLIT_PERCENT`) — **cố tình không cho xuống dưới 20%**: pane kia không bao giờ biến mất hẳn, đúng quyết định "resize chứ không full-screen" đã chốt. `split(area) -> (Rect, Rect)` dựng qua `Layout` bình thường. Đặt ở `tradar-core` vì cả `QueryScreenComponent` (`tradar-query-workbench`) lẫn `HttpScreen` (`tradar-connector-http`) đều cần đúng hành vi này và không được phụ thuộc lẫn nhau.

- **`QueryScreenComponent`**: field `split: SplitPane` thay cho `editor_height = (area.height / 3).clamp(5, 12)` cố định cũ — editor/results giờ theo tỉ lệ `split`, không còn giới hạn cứng 5-12 dòng (đây là thay đổi hành vi có chủ đích, hệ quả trực tiếp của việc cho zoom). Thanh buffer-search (`/` trong editor) giờ carve từ đáy của chính editor pane (`ui::split_bottom_bar`) thay vì một hàng riêng trong layout 3 phần cũ.
- **`HttpScreen`**: field `split: SplitPane` thay `Constraint::Percentage(55)/Percentage(45)` cố định — request builder là primary, response là secondary.
- Phím (giống nhau ở cả hai màn hình, `Context::QueryScreen` và `Context::Http`): `F6` = `Command::ToggleSplitOrientation`, `Ctrl+Up` = `Command::ZoomIn`, `Ctrl+Down` = `Command::ZoomOut`. Chọn tổ hợp phím không phải chữ cái đơn thường vì cả hai context đều có field đang gõ text (editor Insert mode ở QueryScreen, mọi field ở HttpScreen) — chữ cái đơn sẽ bị nuốt mất khi đang gõ, đúng lý do `Context::Http` từ đầu chỉ chứa binding không-in-được.

**`tradar_core::ui::ContextMenu`** (widget dùng chung mới) — popup nhỏ tại điểm click, danh sách `(label, Command)`. **Không tích hợp qua keymap** — đây là hành động cụ thể do component gọi nơi mở nó tự quyết định (menu trên 1 row kết quả khác menu trên response HTTP), không phải binding remap được. `handle_key_event` trả `ContextMenuOutcome::{Open, Confirmed(Command), Closed}` (`j`/`k`/mũi tên di chuyển, `Enter` xác nhận, phím khác không làm gì); `click(bounds, col, row)` hit-test. Xác nhận một item trả về đúng `Command` mà phím tắt tương ứng sẽ tạo ra — component gọi nơi mở menu tự dispatch nó qua **đúng cùng một hàm** xử lý phím tắt, không phải code riêng cho chuột (xem `dispatch_command` bên dưới).

- **Tái cấu trúc bắt buộc để chuột và bàn phím dùng chung code**: `QueryScreenComponent::handle_key_event`'s khối `match command { ... }` (toàn bộ phần dispatch sau khi keymap resolve xong) được tách thành `dispatch_command(&mut self, command: Command) -> Option<Action>` — cả `handle_key_event` (sau khi `keymap().resolve_in` xong) lẫn `handle_mouse_event` (khi context menu confirm) đều gọi hàm này. `HttpScreen` làm y hệt. Tránh có 2 code path cho cùng một hành động rồi lệch nhau dần.
- **Right-click trên một row kết quả** (`QueryScreenComponent`): mở menu 5 mục — Edit cell, Delete row, Yank, Toggle preview, Toggle table/JSON — đúng bộ lệnh `Context::Results` đã có. Không lọc theo "sửa được hay không" trước khi hiện menu — cứ hiện đủ 5 mục, bấm vào cái không áp dụng được (v.d. bảng không có khoá chính) thì rơi vào đúng luồng từ chối-kèm-lý-do đã có sẵn (row_edit overlay), không cần logic kiểm tra riêng cho chuột.
- **Right-click trên response pane** (`HttpScreen`): chỉ 1 mục — Yank body — và chỉ mở khi đã có response (`self.session.response.is_some()`).
- **Middle-click = paste** (`tradar_core::ui::paste_from_clipboard() -> Option<String>`, qua crate `arboard` với `default-features = false` để không kéo theo `image`/`image-data` không cần — chỉ cần đọc text). Khác `yank_to_clipboard` (OSC52, một chiều, ghi qua escape sequence không cần thư viện ngoài): paste cần **đọc** clipboard hệ điều hành thật, OSC52 không có chiều đọc để terminal trả lời. `None` khi không có clipboard (SSH/headless không forward display là trường hợp phổ biến) — middle-click lặng lẽ không làm gì thay vì lỗi, vì đây là tiện ích phụ không đáng chặn người dùng.
  - `QueryScreenComponent`: middle-click trong vùng editor → `query_editor.insert_at_cursor(text)` (đúng hàm navigator đã dùng để chèn tên bảng/cột, nên paste chèn đúng vị trí con trỏ y hệt gõ tay).
  - `HttpScreen`: thêm `TextInput::insert_str`/`TextArea::insert_str` (method mới) — chèn từng ký tự vào đúng vị trí con trỏ; với `TextArea`, `\n` được xử lý như phím Enter thật (tách dòng đúng cách) chứ không chèn ký tự xuống dòng trần vào giữa một dòng. Middle-click luôn **focus đúng field dưới con trỏ trước**, dù có paste được hay không — không có clipboard thì chỉ mất phần paste, không mất phần focus.
- **Trước khi thêm Right/Middle vào mouse filter của `main.rs`**: event loop trước đó lọc mouse event ngay từ đầu, chỉ cho qua `Down(Left)`/`ScrollDown`/`ScrollUp` (lý do hiệu năng, xem "Vấn đề đã biết" trong `docs/backlog/known-issues.md`) — mở rộng thêm `Down(Right)`/`Down(Middle)` vào đúng whitelist đó, không đổi gì về cơ chế lọc `Moved`/`Drag` đã có.
- **Verify tay qua tmux với sqlite thật** (chú ý: `tmux send-keys -H` với chuỗi hex nhiều byte gửi escape sequence SGR mouse **không hoạt động** đúng — tmux có vẻ tách rời timing giữa các byte khiến parser CSI của crossterm không ghép được thành một sự kiện; cách đúng là `tmux send-keys -l $'...'` (chuỗi literal ANSI-C quoting) gửi nguyên khối. Ghi lại ở đây phòng khi cần test mouse qua tmux lần sau): `F6` đổi layout ngang/dọc thấy đúng ngay; `Ctrl+Up` khi focus editor phóng to editor thấy rõ; right-click một row kết quả mở đúng menu 5 mục; click "Delete row" trong menu chạy đúng `dispatch_command(DeleteRow)` — bị từ chối đúng lý do "không có khoá chính" giống hệt bấm phím `d`, xác nhận chuột và bàn phím đi qua chung một code path.
- **Chưa làm / cố tình không làm ngay** (biết trước, chưa đủ lý do làm ngay): right-click chưa có ở Navigator/ConnectionPicker/HistoryPicker — mẫu `ContextMenu` đã sẵn để mở rộng khi có yêu cầu cụ thể; middle-click paste chưa có ở các `TextInput` khác ngoài HTTP fields (form connection, các prompt một dòng) — cùng lý do, hạ tầng (`TextInput::insert_str`) đã có sẵn, chỉ cần thêm lời gọi khi cần; Browse mode (Redis, sidebar cố định 28 cột) **cố tình không** có zoom/orientation — nó không phải cặp editor/results, là layout riêng.

## Non-goals của kiến trúc mục tiêu

- Implement bất kỳ connector không phải dạng query nào khác ngoài Kafka/RabbitMQ — tài liệu này chỉ định nghĩa shape để chúng được xây dựng vào (Cassandra, Kafka, RabbitMQ đã làm xong, không còn thuộc nhóm này -- xem "Trạng thái hiện tại" ở trên). Kubernetes/SSH/Docker vẫn thuộc non-goal này.
- Dynamic plugin loading (`.so`/`.wasm`, phân phối plugin bên thứ ba).
- ~~Một UI "add connection" tương tác (vẫn sửa tay TOML).~~ Non-goal này chỉ áp dụng cho phạm vi migration connector pluggable; màn hình đó đã được làm sau, xem `docs/backlog/roadmap-sub-project.md` mục 5.5.
- Bất kỳ UI nào thực sự branch theo `Capability` — enum và descriptor shape được định nghĩa sẵn bây giờ; việc dùng chúng là việc của tương lai.
- ~~Bất kỳ thay đổi nào cho phần vim-modal query editor ngoài việc `QueryScreenComponent` chuyển vào `tradar-query-workbench`.~~ Không còn đúng — non-goal này chỉ áp dụng cho phạm vi migration connector pluggable (đã xong từ lâu); `QueryEditorComponent` đã được viết lại hoàn toàn (bỏ `edtui`) trong một việc riêng, xem "Đánh bóng UI tổng thể" trong `docs/backlog/roadmap-sub-project.md`.
