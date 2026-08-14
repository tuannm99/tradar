# Architecture

Tài liệu này gồm hai phần: kiến trúc đang triển khai hiện tại, và kiến trúc mục tiêu ban đầu (một Cargo workspace với pipeline `Connector → Session → Screen`) để các hệ thống không có hình dạng "query" — message broker, hệ thống watch/inspect trạng thái sống, v.v. — có thể được thêm vào mà không phải đổi hình dạng core code. **Cập nhật 2026-08-10: cả 4 bước migration đã xong** (bước 1: tách workspace, 2026-08-08; bước 2 + đầu bước 3: tách `tradar-query-workbench` + thu hẹp `Action` + thêm `Connector`/`Session`/`Capability`, 2026-08-09, gộp vì phụ thuộc vòng nhau; bước 4: tách 5 driver thành connector crate riêng + `SavedConnection.driver` sang `String` id + registry, 2026-08-10). Phần "Kiến trúc mục tiêu" bên dưới giờ mô tả đúng những gì đã dựng cho 5 connector dạng query hiện có; nó vẫn còn là "mục tiêu" đúng nghĩa cho các hệ thống không có hình dạng query (Kafka, Kubernetes, SSH, ...) — chưa có connector nào trong nhóm đó được thêm vào.

## Triển khai hiện tại

Tradar là một Cargo workspace gồm mười crate, cấu trúc sao cho ranh giới giữa các layer đã có hình dạng ranh giới crate, đúng theo hướng phụ thuộc mô tả ở "Bố cục workspace" bên dưới.

```
Cargo.toml                    [workspace], default-members = ["crates/tradar-app"]
crates/
  tradar-core/
    src/
      action.rs               — enum Action đóng (6 variant, 3 trong đó mang thêm field `tab: usize` từ 2026-08-12 — xem "Sessions/workspace state" trong docs/backlog.md) + trait Component (có tick() mặc định trả false)
      capability.rs           — enum Capability
      storage/                — saved connections + session state + query file/recent list dạng TOML (dùng crate `directories` để lấy config path); driver: String (connector id).
                                  QueryFiles (thư mục queries + recent list) là global của process như theme/keymap: screen được dựng bên trong connector nên luồn xuống
                                  sẽ phải nhét "file để ở đâu" vào SPI connector; init_query_files() gọi một lần trong main.rs, query_files() trả None nếu chưa init
      config/                — load ~/.config/tradar/config.toml → theme + keymap (2026-08-13; trước đó là placeholder rỗng)
      theme.rs                — bảng màu theo vai trò + override từ config
      keymap.rs               — Command × Context, resolve phím → lệnh, remap từ config, hỗ trợ chuỗi 2 phím (gg)
      ui.rs                   — widget dùng chung: panel có viền/focus, selection style, centered_rect, status hint bar, HelpOverlay
      vim_list.rs             — phép toán di chuyển selection dùng chung cho mọi list (và cho row movement của editor)
  tradar-connector-api/
    src/lib.rs                — trait Connector, trait Session, struct ConnectorDescriptor,
                                  CONNECT_TIMEOUT + with_connect_timeout (giới hạn mở kết nối, mọi connector bọc qua)
  tradar-query-workbench/
    src/
      query_driver.rs         — trait QueryDriver (connect, list_schema, execute, export_curl, edit_source/edit_sql) + SchemaInfo/QueryResult/RowEdit
      query_engine.rs         — QueryEngine: nhận một chuỗi query, giao cho QueryDriver đang active, lưu lịch sử; implement Session
      components/             — QueryScreenComponent (implement Component), + query_editor.rs/results.rs/row_edit.rs/completion.rs/file_prompt.rs/file_picker.rs/history_picker.rs
                                  (struct state+draw thuần, do QueryScreenComponent compose và định tuyến phím tới, không tự implement Component)
  connectors/
    tradar-postgres/  tradar-sqlite/  tradar-elasticsearch/  tradar-redis/  tradar-mongo/
      src/lib.rs               — mỗi crate: struct driver (private, implement QueryDriver) + struct XConnector (private, implement Connector)
                                    + `pub fn connector() -> Box<dyn Connector>` (constructor export duy nhất ra ngoài crate)
  tradar-app/                 [[bin]] name = "tradar"
    src/
      main.rs                 — dựng registry (HashMap<String, Box<dyn Connector>>) từ 5 connector(); event loop:
                                    crossterm input -> Component actions -> spawn Connector::connect -> Session -> Screen
      components/
        mod.rs                — RootComponent: tabs: Vec<Tab> (mỗi Tab: ScreenSlot::ConnectionPicker | Active(Box<dyn Component>) + connection_picker riêng + title) + active_tab
        connection_picker.rs  — ConnectionPickerComponent (list + add/edit/delete)
        connection_form.rs    — ConnectionFormComponent: form 3 field cho add/edit, overlay trên picker
```

`Action`/`Component` nằm ở `tradar-core` (đóng, 6 variant: `Quit`/`OpenRequested`/`Opened`/`OpenFailed`/`BackToPicker`/`ShowHelp` — đổi tên từ `Connect*` thành `Open*` đúng theo "RootComponent và Action" ở mục kiến trúc mục tiêu bên dưới; `ShowHelp` thêm 2026-08-13, vẫn đúng quy tắc "không connector nào thêm variant" vì overlay phím tắt là việc của app shell, không của connector). `QueryDriver`/`SchemaInfo`/`QueryResult`/`QueryEngine` cùng toàn bộ UI dạng query nằm ở `tradar-query-workbench`. `Connector`/`Session`/`ConnectorDescriptor` nằm ở `tradar-connector-api`, cùng với `CONNECT_TIMEOUT`/`with_connect_timeout` — giới hạn thời gian mở kết nối mà **mọi** connector đều bọc qua, đặt chung một chỗ vì client bên dưới của mỗi backend bất đồng hoàn toàn về hành vi khi host không trả lời (sqlx có timeout riêng, `redis`/`mongodb` có default riêng, `reqwest` không có gì), mà TUI thì đứng im trong lúc connect nên treo lâu sẽ bị đọc là app hỏng. Mỗi driver cụ thể sống trong crate connector riêng của nó dưới `crates/connectors/`; `tradar-app` phụ thuộc cả 5 nhưng không chứa code driver nào.

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
}
```

`edit_source`/`edit_sql` (thêm 2026-08-14) là cặp method đứng sau việc **sửa cell / xoá dòng ngay trên bảng kết quả**. Cùng một lý do như `export_curl`: chỉ driver mới biết cú pháp của chính nó, nên `tradar-query-workbench` không bao giờ tự viết câu SQL. `edit_source(query)` trả về tên bảng mà một result đọc từ đó (`None` = không xác định được ⇒ bảng kết quả ở chế độ chỉ đọc); `edit_sql(&RowEdit)` trả về câu lệnh thực hiện thay đổi. Hai connector SQL cùng uỷ quyền cho `query_driver::single_table_source` và `query_driver::build_sql_edit` — được phép dùng chung vì cả hai đều phụ thuộc crate này, còn connector thì không được phụ thuộc lẫn nhau (đúng pattern của `returns_rows`/`SQL_KEYWORDS`/`split_sql_statements`). Mongo/Redis/Elasticsearch giữ mặc định `None`: reply của chúng không phải dòng của một bảng nào có thể địa chỉ hoá được.

`RowEdit` mô tả thay đổi theo ngôn ngữ của *bảng đang nhìn* chứ không theo cú pháp của dialect nào: `{ table, key: Vec<(String, String)>, change: SetValue { column, value } | DeleteRow }`. `key` chính là khoá chính của dòng, lấy từ `SchemaInfo` — đây là lý do `ColumnInfo` có thêm field `primary_key`. Không có khoá chính thì không có mệnh đề `WHERE` nào chỉ đúng một dòng, và một câu lệnh có thể chạm nhiều dòng thì không được tự động chạy thay người dùng.

`ping` (thêm 2026-08-15) là round trip rẻ nhất mà driver có để tự chứng minh còn sống — Postgres/SQLite `SELECT 1` qua pool, Mongo lệnh `ping`, Redis `PING`, Elasticsearch `GET /`. Mặc định `Ok(())` (coi như còn sống) là chủ đích: một driver không override thì hành xử y như trước khi method này tồn tại, không driver nào "tự nhiên" báo rớt kết nối. `QueryEngine::tick()` tự bắn `ping()` trong nền mỗi 15 giây (`PING_INTERVAL`, dùng `tokio::time::Instant` để test được bằng clock giả lập của `tokio::time::pause`), tối đa một lần gọi bay cùng lúc, và cập nhật `alive: bool` đọc qua `QueryEngine::alive()`. Đây là cách duy nhất TUI biết một connection rớt *trước khi* user chạy query vào nó và nhận lỗi — trước đây không có cách nào cả. `Component` có thêm `connection_alive() -> Option<bool>` (mặc định `None` — "không áp dụng", cho picker/overlay/mọi thứ không giữ connection riêng) để mang trạng thái đó từ `QueryEngine` (bên trong `tradar-query-workbench`) lên tới `QueryScreenComponent::draw` và tới navigator ở app shell — cùng contract "app chỉ chuyển tiếp, không hiểu nội dung" với `restore_state`/`outline`.

`export_curl` thay cho `Action::ExportCurl` cũ (một variant trong enum dùng chung mà chỉ Elasticsearch implement, buộc `main.rs` phải special-case theo `DriverKind`) — mặc định `None` ("không hỗ trợ export"), chỉ `ElasticsearchDriver` (trong `tradar-elasticsearch`) override. Curl export giờ nằm gọn trong crate của riêng Elasticsearch (`QueryScreenComponent` chỉ gọi `self.engine.export_curl(query)`, không biết gì về ES) — mức cô lập cuối cùng đã đạt được, không còn "chờ bước 4" như ghi chú trước đây.

`SchemaInfo` và `QueryResult` là các shape đã chuẩn hoá mà phần còn lại của app render ra — driver có trách nhiệm dịch kết quả gốc của database sang các kiểu này. `QueryResult` là enum, không phải một struct duy nhất, vì kết quả dạng bảng (SQL) và kết quả dạng document (MongoDB, Elasticsearch, Redis) không cùng shape:

```rust
pub enum QueryResult {
    Table { columns: Vec<String>, rows: Vec<Vec<String>> },
    Documents(Vec<serde_json::Value>),
}
```

`Table` là kết quả Postgres và SQLite trả về. `Documents` dùng chung cho ba driver còn lại: mỗi hit/response của Elasticsearch, mỗi document của MongoDB, mỗi reply của Redis thành một `serde_json::Value` trong vec. `ResultsComponent` render `Table` thành bảng text và `Documents` thành các khối JSON pretty-print.

### Phạm vi ngôn ngữ query theo từng driver

Postgres và SQLite chấp nhận SQL tuỳ ý. Ba driver còn lại chỉ chấp nhận một tập con hẹp, có chủ đích, không phải toàn bộ ngôn ngữ query gốc:

- **Elasticsearch**: mô phỏng theo Dev Tools console của Kibana, không phải client Search-only cố định. Dòng đầu là `METHOD /path` (vd `GET my-index/_search`); các dòng còn lại (nếu có) là JSON request body, gửi nguyên văn. Không có cấu hình auth/TLS client-cert, và mỗi lần chạy chỉ một request (không có script nhiều request). Toàn bộ JSON response được bọc thành một `Documents` một phần tử, không unwrap thành từng document theo hit. `Ctrl+Y` trên một kết nối Elasticsearch xuất request hiện tại thành lệnh `curl` ghi vào `./tradar-query.sh` (đường dẫn cố định, ghi đè mỗi lần export).
- **Redis**: một dòng lệnh duy nhất, tách theo khoảng trắng (không hỗ trợ quoting/escaping), gửi qua `redis::cmd`. Chuyển đổi kết quả chỉ nhận biết kiểu cho `HGETALL` (→ JSON object) và `ZRANGE`/`ZREVRANGE ... WITHSCORES` (→ mảng object `{member, score}`); mọi lệnh khác dùng chuyển đổi RESP-to-JSON tổng quát. Không có pipelining, transaction (`MULTI`/`EXEC`), pub/sub, hay xử lý riêng cho stream (`XADD`/`XRANGE`).
- **MongoDB**: một parser tối giản cho đúng shape `db.<collection>.<method>(<json-args>)` — không phải JS engine thật. Hỗ trợ `find`, `aggregate`, `insertOne`, `insertMany`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`. Không có method chaining (`.sort()`, `.limit()`), không có `$where`, không có bulk operation hay transaction; bất cứ gì ngoài shape này trả về lỗi "unsupported query".

### Keymap, theme và widget dùng chung

Thêm 2026-08-13. Ba module trong `tradar-core` mà **mọi** component phải đi qua thay vì tự làm:

- **`keymap`** — component không match `KeyCode` trực tiếp nữa; nó hỏi `keymap().resolve_in(&[context...], &mut pending, key)` và nhận về một `Command`. `Context` (`Global`/`Picker`/`QueryScreen`/`List`/`Prompt`) tồn tại vì cùng một phím mang nghĩa khác nhau tuỳ chỗ (`enter` = connect ở picker, = chèn tên schema ở query screen). `resolve_in` nhận *nhiều* context theo thứ tự ưu tiên, dùng chung một ô `pending` — đó là cách một màn hình check binding riêng của nó trước rồi mới rơi xuống điều hướng list dùng chung. `pending: Option<KeyPress>` do chính component giữ và chính là `pending_g: bool` cũ được tổng quát hoá cho chuỗi 2 phím bất kỳ, không riêng `gg`.
- **`theme`** — màu đặt tên theo *vai trò* (`border_focused`, `error`, `syntax_keyword`), không theo tên màu. Component không được viết `Color::Red` thẳng.
- **`ui`** — `panel()`/`selection_style()`/`centered_rect()`/`draw_status_bar()`/`HelpOverlay`. Nằm ở `tradar-core` vì cả `tradar-app` và `tradar-query-workbench` đều cần, mà hai crate đó không được phụ thuộc nhau.

Cả `theme` và `keymap` được nạp một lần lúc khởi động (`config::init`) vào một `OnceLock`, đọc qua hàm `theme()`/`keymap()` trả về `&'static` — nếu chưa nạp thì rơi về mặc định dựng sẵn. Chọn global thay vì truyền tham số xuyên mọi `handle_key_event`/`draw` là có chủ đích: nó tránh phải đổi signature của trait `Component` (và `Session::build_screen` ở `tradar-connector-api`) chỉ để mang theo hai thứ config bất biến suốt vòng đời process. Đánh đổi: test không thay được keymap giữa chừng, nên logic remap được test trực tiếp trên `Keymap` ở `tradar-core`, còn component thì test theo binding mặc định.

Hai bất biến của phần dispatch phím, cả hai đều có test hồi quy trong `query_screen.rs` (dễ vô tình phá khi thêm binding mới):

1. **Ký tự thường gõ trong Insert mode luôn là text, không bao giờ là lệnh.** Không có quy tắc này thì bind `?` cho help sẽ khiến không gõ được dấu `?` vào query. Chỉ phím có `CONTROL`/`ALT` (hoặc phím không phải `Char`) mới đi tới keymap khi editor đang ở Insert mode.
2. **Lệnh gắn với một pane cụ thể chỉ chạy khi pane đó đang focus** (`required_focus`: `yank` → Results, `insert-name` → Sidebar). Bấm ở chỗ khác thì phím rơi xuống editor như thể không có binding — nên `enter` vẫn xuống dòng bình thường khi đang gõ query.

### Quy tắc cách ly (isolation rule)

Đây là quy tắc giữ cho driver pluggable, và được áp dụng ở mọi nơi, không chỉ ở top level — giờ được Cargo enforce, không chỉ quy ước:

- Code trong mỗi crate connector (`crates/connectors/tradar-*`) chỉ implement `QueryDriver` (từ `tradar-query-workbench`) và `Connector`/`Session` (từ `tradar-connector-api`); struct driver/connector cụ thể không `pub` ra ngoài crate, chỉ `pub fn connector()` được export.
- Code trong `tradar-query-workbench` (components, `query_engine`) chỉ phụ thuộc trait `QueryDriver` — không bao giờ phụ thuộc một connector crate cụ thể nào (thật ra không *thể* phụ thuộc — mỗi connector crate phụ thuộc `tradar-query-workbench`, không phải ngược lại, nên một cycle sẽ chặn ngay ở compile time nếu vi phạm).
- `tradar-app/src/main.rs` là nơi duy nhất biết toàn bộ tập connector (hàm `registry()` gọi `connector()` của cả 5 crate) — không connector crate nào phụ thuộc connector crate khác hay phụ thuộc `tradar-app`.

Thêm một database mới nghĩa là: tạo một crate connector mới dưới `crates/connectors/` implement `QueryDriver` + `Connector`/`Session` + export `connector()`, thêm một dòng dependency vào `tradar-app/Cargo.toml`, và một dòng trong `registry()`. Việc này không bao giờ được yêu cầu sửa `tradar-query-workbench`, `tradar-connector-api`, `tradar-core`, hay bất kỳ connector crate nào khác.

### Trạng thái hiện tại

Walking skeleton v1 chạy được từ đầu đến cuối: `tradar` load các saved connection từ `storage`, dựng registry từ 5 connector crate, kết nối qua `Connector` tương ứng với connector id đã chọn (Postgres, SQLite, Elasticsearch, Redis, hoặc MongoDB, tất cả đều implement đầy đủ chạy được với backend thật), và chạy các query gõ vào query editor của `QueryScreenComponent` thông qua `QueryEngine`, render kết quả hoặc lỗi thật.

Những phần còn mỏng/thiếu đáng chú ý:

- ~~Chưa có màn hình "add connection" tương tác~~ — đã có từ 2026-08-14: `a`/`e`/`d` trong connection picker, form ở `crates/tradar-app/src/components/connection_form.rs`; xem `docs/backlog.md` mục 5.5.
- `QueryDriver::list_schema` đã implement và test cho cả năm driver, và đã nối vào TUI dưới dạng **navigator** (`crates/tradar-app/src/components/navigator.rs`) — một cây `connection → bảng → cột` phủ mọi connection đã lưu, không chỉ connection của tab hiện tại. Nó nằm ở app shell chứ không trong screen vì chỉ `RootComponent` biết có những connection nào khác và chúng đang mở ở tab nào; screen chỉ cung cấp dữ liệu qua `Component::outline()`. Cùng cách đó, `NavConnection.alive` (từ `Component::connection_alive()`) gắn `✗ disconnected` sau tên một connection đang mở tab mà ping nền vừa phát hiện rớt — xem `ping`/`connection_alive` ở mục "Trait `QueryDriver`" phía trên.
- MongoDB (`list_schema`) suy ra field bằng cách đọc mẫu một document mỗi collection lúc connect — xem chi tiết và đánh đổi ở `docs/backlog.md` mục "Schema đọc được sâu hơn".
- `config/` là module placeholder rỗng; cấu hình app ngoài file connections chưa tồn tại.

## Kiến trúc mục tiêu: connector pluggable

Cả năm driver hiện tại dùng chung một shape: `connect → list_schema → execute(query) -> Table | Documents`, được enforce bởi trait `QueryDriver` duy nhất và UI `QueryScreenComponent` duy nhất ở trên. Shape đó không khớp với các hệ thống Tradar dự định hỗ trợ tiếp theo — message broker (Kafka, RabbitMQ), hệ thống watch/inspect trạng thái sống (Kubernetes, Docker, Prometheus), và công cụ dạng remote-shell (SSH). Kafka/RabbitMQ không phải "gửi một chuỗi query, nhận về rows" — chúng là browse-topic/queue, tail message theo thời gian thực, publish một message. Cassandra (CQL) là ngoại lệ: nó khớp shape query nên có thể tái dùng UI hiện tại.

Phần sau định nghĩa shape mà toàn bộ 5 connector dạng query hiện có (Postgres, SQLite, Elasticsearch, Redis, MongoDB) đã được xây theo, và là shape các hệ thống không phải query (Kafka, Kubernetes, SSH, ...) sẽ được xây vào khi chúng thực sự được lên kế hoạch. Xem "Triển khai hiện tại" ở trên để biết layout thật hiện có.

### Các quyết định

- **Pluggable = static/compile-time, không phải dynamic loading.** Không load `.so`/`.wasm`, không có hệ sinh thái plugin bên thứ ba. Mỗi connector là một Rust crate được compile thẳng vào binary `tradar`.
- **Tách thành Cargo workspace, mỗi module một crate.** Spec v1 đã gác lại việc này cho tới khi có "một lý do thứ hai cụ thể"; việc thêm các connector có hình dạng khác hẳn (message queue, hệ thống watch-based) bên cạnh các connector dạng query hiện có chính là lý do đó. Workspace biến quy tắc cách ly thành một sự thật của dependency graph trong Cargo, không chỉ là quy ước ghi bằng comment.
- **Mỗi connector sở hữu Screen riêng của nó**, không dùng chung shape query-editor. Các connector *có hình dạng query* (SQL, Mongo, ES, Redis, Cassandra sau này) vẫn dùng chung một crate UI để không phải tự implement lại.
- **"Backend"/"Driver" đổi tên thành Connector** xuyên suốt (`PostgresConnector`, `KafkaConnector`, ...) — một database Postgres, một cluster Kafka, và một host SSH không phải "backend"/"driver" theo bất kỳ nghĩa chung nào mà cái tên cũ nắm bắt được.
- **Kết nối và dựng UI là hai việc khác nhau, do hai kiểu khác nhau đảm nhiệm:** một `Connector` tạo ra một `Session`; một `Session` tạo ra một `Screen`.

### Bố cục workspace

```
Cargo.toml                       [workspace]
crates/
  tradar-core/                   — Action, trait Component, Capability,
                                    storage (SavedConnection, ConnectionStore), config
  tradar-connector-api/          — trait Connector, trait Session, ConnectorDescriptor, CONNECT_TIMEOUT
                                    (SPI dành cho connector — xem "Vì sao tách khỏi tradar-core" bên dưới)
  tradar-query-workbench/        — QueryScreenComponent, ResultsComponent, QueryEditorComponent,
                                    QueryEngine (implement Session), trait QueryDriver, SchemaInfo/QueryResult
  connectors/
    tradar-postgres/  tradar-sqlite/  tradar-mongo/  tradar-elasticsearch/  tradar-redis/
    (tương lai) tradar-kafka/, tradar-rabbitmq/, tradar-cassandra/
  tradar-app/ (binary crate)     — main.rs (registry + event loop), RootComponent, ConnectionPickerComponent
```

Toàn bộ layout trên đã dựng đúng như mô tả kể từ 2026-08-10. `tradar-query-workbench` là bản chuyển gần như nguyên khối từ `components/query_screen.rs` et al. trước migration -- một vài chỗ đổi hình dạng cho khớp `Session`, xem "Sai khác khi triển khai thật" ở mục "Screen không bao giờ làm IO" bên dưới. Nhóm connector tương lai (`tradar-kafka/`, ...) vẫn chưa tồn tại — chưa có hệ thống không-phải-query nào được lên kế hoạch cụ thể.

`tradar-query-workbench` đặt tên là "workbench", không phải "ui" — nó gói cả editor, execution, và history cho các connector dạng query, không chỉ là widget.

**Vì sao tách `Connector`/`Session`/`ConnectorDescriptor` ra `tradar-connector-api` thay vì để trong `tradar-core`** (quyết định 2026-08-08, trước khi bước 3 bắt đầu nên không tốn refactor gì): `Connector`/`Session` là một SPI (service provider interface) dành riêng cho connector, không phải domain cốt lõi của ứng dụng (state, action, config, storage). Nếu để chung trong `tradar-core`, crate này sẽ dần phình thành "God crate" chứa đủ thứ không liên quan tới nhau. Tách riêng giúp `tradar-core` tập trung vào state/action/config/hạ tầng ứng dụng, còn connector chỉ cần phụ thuộc `tradar-connector-api` cộng các kiểu dùng chung cần thiết.

Hướng phụ thuộc, được Cargo enforce (không chỉ là quy ước):

- `tradar-core` không phụ thuộc gì trong workspace.
- `tradar-connector-api` phụ thuộc `tradar-core` (cho `SavedConnection`, `Action`, `Component`).
- `tradar-query-workbench` phụ thuộc `tradar-core` và `tradar-connector-api`.
- Mỗi connector crate luôn phụ thuộc `tradar-connector-api`, và phụ thuộc `tradar-query-workbench` chỉ khi nó có hình dạng query (Postgres, SQLite, Mongo, Elasticsearch, Redis hiện tại; Cassandra sau này). Kafka, RabbitMQ, và các connector không phải dạng query khác chỉ phụ thuộc `tradar-connector-api`/`tradar-core` và tự dựng `Session`/`Component` implementation riêng.
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
    /// "Screen never does IO" bên dưới.
    fn tick(&mut self);

    fn build_screen(self: Box<Self>, action_tx: UnboundedSender<Action>) -> Box<dyn Component>;
}
```

- **Connector**: factory gần như stateless. Nhận một `SavedConnection`, tạo ra một `Session`. Là giai đoạn duy nhất làm handshake ban đầu (mở kết nối TCP, xác thực, ping).
- **Session**: actor sống lâu dài. Sở hữu mọi thứ chạm vào IO hoặc tồn tại lâu hơn một khung render — connection/client handle, mọi background task nó spawn, một channel nội bộ để các task đó báo kết quả về, và cache (schema, topic metadata, mapping info). Một `KafkaSession` sở hữu consumer, producer, và offset tracking của nó; một `MongoSession` sở hữu client và collection cache của nó. **`Session` là thứ duy nhất trong pipeline này được phép spawn task hoặc sở hữu channel.**
- **Screen**: cái mà `RootComponent` thực sự giữ và route phím/draw tới — một giá trị implement `Component`. Nó đọc state của `Session` để render, và biến key event thành lời gọi command đồng bộ trên `Session` (vd `session.submit_query(text)`, `session.publish(topic, payload)`). Nó không bao giờ chạm socket, file, hay `tokio::spawn` trực tiếp.

Connector dạng query: `QueryEngine` (trong `tradar-query-workbench`) implement `Session` — `tick()` của nó rút các reply query-completion, `build_screen()` của nó trả về `QueryScreenComponent`. Đây là một cú fit không cần đổi tên; `QueryEngine` đã đóng vai trò này rồi, chỉ cần gắn thêm trait chính thức.

Không cần trait mới cho việc phân biệt Screen/Component/widget — nó chỉ đặt tên cho một pattern code đã dùng sẵn. `QueryScreenComponent` là một Screen (implement `Component`), nhưng bên trong compose `query_editor.rs`, `results.rs`, và `row_edit.rs`, không cái nào tự implement `Component` — chỉ là struct state+draw thuần mà screen sở hữu và gọi trực tiếp. Mọi connector tương lai theo cùng pattern: `KafkaScreen` implement `Component` và compose các struct thuần `TopicList`/`MessageTable`/`Header` tuỳ ý.

### Screen không bao giờ làm IO — Session là actor

Một Screen không bao giờ được gọi `tokio::spawn` hay sở hữu channel trực tiếp. Nếu làm vậy, code UI của mỗi connector sẽ bị đan xen với IO/business logic của nó — đúng kiểu coupling mà quy tắc cách ly driver tồn tại để ngăn, chỉ là bị đẩy lên một layer cao hơn.

1. Một phím bấm hoặc lời gọi `update()` trên Screen biến thành một lời gọi method **đồng bộ** trên Session của nó (vd `self.session.submit_query(text)`), trả về ngay lập tức.
2. Method đó của Session gọi `tokio::spawn` cho IO mà command cần, đưa cho task được spawn nửa `Sender` của một channel do Session sở hữu.
3. Event loop gọi `Component::tick()` trên screen đang active mỗi vòng lặp (method mới của trait, mặc định no-op); `tick()` của Screen forward tới `self.session.tick()`.
4. `Session::tick()` rút channel nội bộ của chính nó — **có giới hạn** (vd tối đa 64 message mỗi lần gọi) — cập nhật state của chính nó. Nó không bao giờ block.
5. Lời gọi `draw()` tiếp theo của Screen render bất cứ state nào Session đang giữ.

```rust
pub trait Component {
    fn handle_key_event(&mut self, code: KeyCode, modifiers: KeyModifiers) -> Option<Action>;
    fn update(&mut self, action: Action) -> Option<Action>;
    fn tick(&mut self) {}
    fn draw(&mut self, frame: &mut Frame, area: Rect);
}
```

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

Cho phép một connection picker hay một screen suy luận về việc một connector *có thể làm gì* mà không hardcode danh tính của nó. Ví dụ: Postgres/SQLite khai `[Query, Schema, Explain, Export]`; Redis khai `[Query, Streaming]`; một Kafka tương lai sẽ khai `[Streaming, Publish, Tail]`. Chưa có gì dùng `Capability` — chưa có UI nào branch theo nó — nó được định nghĩa sẵn bây giờ vì retrofit sau khi đã có vài connector sẽ gây xáo trộn hơn nhiều so với định nghĩa shape từ đầu, và để chưa dùng thì không tốn gì cả.

### Registry

**Đã dựng (2026-08-10).** `SavedConnection.driver` (`tradar-core::storage`) là một `String` connector id (vd `"postgres"`, `"sqlite"`), không còn enum đóng `DriverKind` — file `connections.toml` trên đĩa không đổi format, vì `DriverKind` vốn đã serialize thành cùng chuỗi thường này (`#[serde(rename_all = "lowercase")]`).

- Mỗi connector crate export đúng một hàm `pub fn connector() -> Box<dyn Connector>`, mọi thứ khác trong crate (struct driver, struct connector) không `pub`.
- `crates/tradar-app/src/main.rs`'s `registry()` là nơi duy nhất biết toàn bộ tập connector: gọi `connector()` của cả 5 crate, dựng `HashMap<String, Box<dyn Connector>>` (key lấy từ `descriptor().id`) một lần lúc khởi động, bọc trong `Arc` để clone rẻ vào mỗi task connect. Thêm một connector nghĩa là thêm một dòng dependency trong `tradar-app/Cargo.toml` và một dòng trong `registry()` — không đổi `tradar-core`, `tradar-connector-api`, `tradar-query-workbench`, hay bất kỳ connector crate nào khác.
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
- **`Component: Send`.** Trait hiện không (và ban đầu không thể) ràng buộc `Send` — lý do gốc là `QueryEditorComponent` giữ `edtui::EditorState`, bên trong có `Rc<RefCell<dyn ClipboardTrait>>` (xem `ConnectOutcome` trong "Triển khai hiện tại"). `edtui` đã bị bỏ (2026-08-12, xem `docs/backlog.md`) và editor tự viết hiện tại không có field non-`Send` nào rõ ràng — lý do gốc có thể không còn đúng, nhưng **chưa verify** toàn bộ implementor của `Component` (`RootComponent`, `ConnectionPickerComponent`, `QueryScreenComponent` và các sub-component của nó) thực sự `Send`, và việc bound `Send` sẽ đơn giản hoá `main.rs` (bỏ được channel `ConnectOutcome` riêng, dựng `Screen` thẳng trong task connect). Xem lại nếu channel `ConnectOutcome` này từng trở thành điểm khó bảo trì thật.
- ~~**`Workspace → Tab → Screen` thay vì một `ScreenSlot::Active(Box<dyn Component>)` duy nhất.**~~ Đã làm (2026-08-12), theo hướng thực dụng chứ không đúng nguyên văn: không có type `Workspace` mới, `RootComponent` (`crates/tradar-app/src/components/mod.rs`) đóng vai trò đó, đổi từ một `screen`/`connection_picker` sang `tabs: Vec<Tab>` + `active_tab: usize` (`Tab` gồm `screen: ScreenSlot`, `connection_picker` riêng, và `title` để tab bar có gì hiển thị). Xem chi tiết đầy đủ ở mục "Sessions/workspace state" trong `docs/backlog.md`. Split-view (nhiều tab hiển thị cùng lúc, không chỉ chuyển qua lại) vẫn chưa làm — xem lại nếu thực sự cần.
- **`ConnectorFactory` thay vì trait object `Connector` trực tiếp trong registry.** Xem lại nếu một connector cần khởi tạo non-singleton (vd instance riêng cho từng tab với config khác nhau), hiện chưa có gì cần việc này.

**Ghi nhận thêm 2026-08-08** — một buổi review kiến trúc rộng hơn đề xuất tổ chức lại toàn bộ dự án theo hướng "Engineering Workbench" thay vì theo từng database: thêm layer `Runtime` (workspace/scheduler/lifecycle/background task) tách khỏi App, tách UI thành `tradar-ui` (widget dùng chung) + `tradar-editor` (vim/textarea/cursor/completion) tách khỏi `tradar-query-workbench`, một loạt crate hạ tầng dùng chung (`tradar-table`, `tradar-tree`, `tradar-json`, `tradar-log`, `tradar-terminal`, `tradar-theme`, `tradar-keymap`, `tradar-command`, `tradar-icons`, `tradar-search`, `tradar-utils`, `tradar-plugin`), một AI service độc lập gắn vào editor, và các connector mới ngoài phạm vi hiện tại (Kubernetes, Docker, SSH). Tất cả các ý này **chưa được đưa vào shape mục tiêu ở trên** — cùng lý do như các mục phía trên: chưa có connector/tính năng cụ thể nào cần chúng, và phần lớn còn chưa nằm trong phạm vi v1/planned của `CLAUDE.md` hay `docs/backlog.md`. Ghi lại ở đây để trace, không phải để scaffold ngay:
  - **`tradar-runtime`** (workspace manager, scheduler, lifecycle, background task tách khỏi App/Session) — xem lại khi có nhu cầu thật sự ngoài event loop hiện tại của `main.rs` (vd nhiều workspace/tab chạy song song thực sự cần điều phối).
  - **`tradar-ui` / `tradar-editor` tách khỏi `tradar-query-workbench`** — xem lại khi có Screen không phải dạng query cần dùng lại editor/widget (hiện `QueryEditorComponent` — editor vim-modal tự viết từ 2026-08-12, không còn phụ thuộc `edtui` — chỉ được dùng ở đây).
  - **Các crate hạ tầng dùng chung** (`tradar-table`, `tradar-tree`, `tradar-json`, `tradar-log`, `tradar-terminal`, `tradar-theme`, `tradar-keymap`, `tradar-command`, `tradar-icons`, `tradar-search`, `tradar-utils`) — xem lại từng cái khi code tương ứng thực sự tồn tại và bị dùng lại ở ≥2 chỗ; không tạo crate rỗng trước.
  - **`tradar-plugin`** — xem lại nếu dynamic loading (`.so`/`.wasm`) thực sự được quyết định làm, trái với "Non-goals" bên dưới hiện tại.
  - **AI service gắn vào editor** — xem lại khi tính năng AI cụ thể (completion, apply-patch, ...) được scope, hiện chưa có trong `docs/backlog.md`.
  - **Connector Kubernetes/Docker/SSH** — cùng nhóm với Kafka/RabbitMQ ở trên: hợp lý về mặt shape (`Connector`/`Session`/`Screen` đã tính tới các hệ thống không phải query), nhưng chưa có trong danh sách connector v1/planned; thêm vào danh sách connector tương lai khi thực sự được lên kế hoạch.

## Non-goals của kiến trúc mục tiêu

- Implement Kafka, RabbitMQ, Cassandra, hay bất kỳ connector mới nào khác — tài liệu này chỉ định nghĩa shape để chúng được xây dựng vào.
- Dynamic plugin loading (`.so`/`.wasm`, phân phối plugin bên thứ ba).
- ~~Một UI "add connection" tương tác (vẫn sửa tay TOML).~~ Non-goal này chỉ áp dụng cho phạm vi migration connector pluggable; màn hình đó đã được làm sau, xem `docs/backlog.md` mục 5.5.
- Bất kỳ UI nào thực sự branch theo `Capability` — enum và descriptor shape được định nghĩa sẵn bây giờ; việc dùng chúng là việc của tương lai.
- ~~Bất kỳ thay đổi nào cho phần vim-modal query editor ngoài việc `QueryScreenComponent` chuyển vào `tradar-query-workbench`.~~ Không còn đúng — non-goal này chỉ áp dụng cho phạm vi migration connector pluggable (đã xong từ lâu); `QueryEditorComponent` đã được viết lại hoàn toàn (bỏ `edtui`) trong một việc riêng, xem "Đánh bóng UI tổng thể" trong `docs/backlog.md`.
