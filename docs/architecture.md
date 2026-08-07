# Architecture

Tài liệu này gồm hai phần: kiến trúc đang triển khai hiện tại, và kiến trúc mục tiêu mà dự án đang migrate tới (một Cargo workspace với pipeline `Connector → Session → Screen`) để các hệ thống không có hình dạng "query" — message broker, hệ thống watch/inspect trạng thái sống, v.v. — có thể được thêm vào mà không phải đổi hình dạng core code. Migration đã bắt đầu: việc tách workspace/crate (bước 1 của kiến trúc mục tiêu, thuần cơ học) đã xong trước; `Action`, `Component`, `Driver`, và mọi thứ khác vẫn giữ nguyên hình dạng hiện tại cho tới các bước sau, mô tả ở "Target architecture" bên dưới.

## Triển khai hiện tại

Tradar là một Cargo workspace gồm hai crate, cấu trúc sao cho ranh giới giữa các layer đã có hình dạng ranh giới crate. `tradar-core` không phụ thuộc vào `tradar-app`; `tradar-app` phụ thuộc vào `tradar-core`.

```
Cargo.toml                    [workspace], default-members = ["crates/tradar-app"]
crates/
  tradar-core/
    src/
      storage/               — saved connections dạng TOML (dùng crate `directories` để lấy config path)
      config/                — chỗ dành sẵn cho việc load app config; chưa dùng
  tradar-app/                 [[bin]] name = "tradar"
    src/
      main.rs                 — event loop: crossterm input -> Component actions -> gọi query_engine/driver
      action.rs               — enum Action định nghĩa toàn bộ state transition có thể xảy ra, và trait Component
      components/             — các component ratatui; RootComponent, ConnectionPickerComponent, và QueryScreenComponent implement trait Component từ action.rs, còn query_editor.rs, results.rs, schema_sidebar.rs chỉ là struct state+draw thuần, được QueryScreenComponent compose lại chứ không tự implement Component
        mod.rs                — RootComponent compose ConnectionPickerComponent và QueryScreenComponent
        connection_picker.rs  — ConnectionPickerComponent
        query_screen.rs       — QueryScreenComponent (compose QueryEditorComponent, ResultsComponent, SchemaSidebarComponent)
        query_editor.rs       — QueryEditorComponent
        results.rs            — ResultsComponent
        schema_sidebar.rs     — SchemaSidebarComponent
      query_engine/           — nhận một chuỗi query, giao cho driver đang active, lưu lịch sử
      drivers/
        mod.rs                — trait Driver (connect, list_schema, execute, ...)
        postgres/
        sqlite/
        elasticsearch/
        redis/
        mongo/
```

`storage` và `config` là hai module duy nhất không phụ thuộc gì khác trong app, đó là lý do chúng được chuyển vào `tradar-core` trước tiên — `action.rs` vẫn kéo trực tiếp `QueryEngine` và các kiểu của driver, nên nó còn ở lại `tradar-app` cho tới khi bước 3 của kiến trúc mục tiêu (bên dưới) tách nó xuống còn `Action` đóng với 5 variant.

### Trait `Driver`

Mỗi database backend implement chung một trait, định nghĩa ở `crates/tradar-app/src/drivers/mod.rs`:

```rust
#[async_trait]
pub trait Driver: Send + Sync {
    async fn connect(&mut self) -> anyhow::Result<()>;
    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>>;
    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult>;
}
```

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

### Quy tắc cách ly (isolation rule)

Đây là quy tắc giữ cho driver pluggable, và được áp dụng ở mọi nơi, không chỉ ở top level:

- Code trong `drivers/*` chỉ implement `Driver` và không phụ thuộc gì khác trong app.
- Code trong `components/`, `action.rs`, và `query_engine` chỉ phụ thuộc trait `Driver` — không bao giờ phụ thuộc `drivers::postgres`, `drivers::sqlite`, hay bất kỳ module driver cụ thể nào khác.
- `main.rs` là nơi duy nhất tạo một driver cụ thể (trong `Action::ConnectRequested`) hoặc gọi một helper của driver cụ thể (trong `Action::ExportCurl`).

Thêm một database mới nghĩa là thêm một module mới dưới `drivers/` implement `Driver`. Việc này không bao giờ được yêu cầu sửa `components/`, `action.rs`, hay `query_engine`.

### Trạng thái hiện tại

Walking skeleton v1 chạy được từ đầu đến cuối: `tradar` load các saved connection từ `storage`, kết nối qua `Driver` được chọn (Postgres, SQLite, Elasticsearch, Redis, hoặc MongoDB, tất cả đều implement đầy đủ chạy được với backend thật), và chạy các query gõ vào query editor của `QueryScreenComponent` thông qua `query_engine`, render kết quả hoặc lỗi thật.

Những phần còn mỏng/thiếu đáng chú ý:

- Chưa có màn hình "add connection" tương tác — connection được thêm bằng cách sửa tay file TOML.
- `Driver::list_schema` đã implement và test cho cả năm driver, và đã nối vào TUI dưới dạng schema sidebar trên query screen (tự load khi connect; `Tab` để focus, `Enter` để chèn tên được chọn vào query).
- `config/` là module placeholder rỗng; cấu hình app ngoài file connections chưa tồn tại.

## Kiến trúc mục tiêu: connector pluggable

Cả năm driver hiện tại dùng chung một shape: `connect → list_schema → execute(query) -> Table | Documents`, được enforce bởi trait `Driver` duy nhất và UI `QueryScreenComponent` duy nhất ở trên. Shape đó không khớp với các hệ thống Tradar dự định hỗ trợ tiếp theo — message broker (Kafka, RabbitMQ), hệ thống watch/inspect trạng thái sống (Kubernetes, Docker, Prometheus), và công cụ dạng remote-shell (SSH). Kafka/RabbitMQ không phải "gửi một chuỗi query, nhận về rows" — chúng là browse-topic/queue, tail message theo thời gian thực, publish một message. Cassandra (CQL) là ngoại lệ: nó khớp shape query nên có thể tái dùng UI hiện tại.

Phần sau định nghĩa shape mục tiêu mà các hệ thống trên sẽ được xây dựng vào. **Đây chỉ là kiến trúc — chưa có code nào được di chuyển.** Migration là một refactor xuyên suốt (đụng tới cả năm driver hiện có, `RootComponent`, `main.rs`, và `storage`), dự định thực hiện theo từng bước — dựng workspace và các crate core trước, rồi migrate từng driver một — không phải một thay đổi lớn duy nhất.

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
  tradar-connector-api/          — trait Connector, trait Session, ConnectorDescriptor
                                    (SPI dành cho connector — xem "Vì sao tách khỏi tradar-core" bên dưới)
  tradar-query-workbench/        — QueryScreenComponent, ResultsComponent, SchemaSidebarComponent, QueryEditorComponent,
                                    QueryEngine (implement Session), trait QueryDriver, SchemaInfo/QueryResult
                                    (là components/query_screen.rs et al. hôm nay, chuyển nguyên khối — không phải code mới)
  connectors/
    tradar-postgres/  tradar-sqlite/  tradar-mongo/  tradar-elasticsearch/  tradar-redis/
    (tương lai) tradar-kafka/, tradar-rabbitmq/, tradar-cassandra/
  tradar-app/ (binary crate)     — main.rs, RootComponent, ConnectionPickerComponent, connector registry
```

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

Không cần trait mới cho việc phân biệt Screen/Component/widget — nó chỉ đặt tên cho một pattern code đã dùng sẵn. `QueryScreenComponent` là một Screen (implement `Component`), nhưng bên trong compose `query_editor.rs`, `results.rs`, và `schema_sidebar.rs`, không cái nào tự implement `Component` — chỉ là struct state+draw thuần mà screen sở hữu và gọi trực tiếp. Mọi connector tương lai theo cùng pattern: `KafkaScreen` implement `Component` và compose các struct thuần `TopicList`/`MessageTable`/`Header` tuỳ ý.

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

- `SavedConnection.driver` đổi từ enum đóng `DriverKind` thành một `String` connector id (vd `"postgres"`, `"kafka"`), match với `ConnectorDescriptor::id`. Không `tradar-core` lẫn connector crate nào cần liệt kê toàn bộ danh sách connector.
- Mỗi connector crate export một constructor, vd `pub fn connector() -> Box<dyn Connector>`.
- `main.rs` của `tradar-app` là nơi duy nhất biết toàn bộ tập connector: nó dựng một `HashMap<String, Box<dyn Connector>>` lúc khởi động bằng cách gọi constructor của từng connector crate. Thêm một connector nghĩa là thêm một dòng dependency trong `Cargo.toml` và một dòng đăng ký trong `main.rs` — không đổi `tradar-core`, `tradar-connector-api`, `tradar-query-workbench`, hay bất kỳ connector crate nào khác.
- Một `connection.driver` id không match là lỗi runtime hiển thị cho người dùng (vd `"unknown connector 'kafka': not compiled into this build"`), không phải lỗi compile-time do enum không exhaustive.

### RootComponent và Action

Field cố định `query_screen: QueryScreenComponent` của `RootComponent` trở thành:

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

**Tác dụng phụ:** `Action::ExportCurl` hiện tại là một variant trong enum dùng chung mà chỉ Elasticsearch implement, buộc `main.rs` phải special-case nó. Dưới model mới, curl export trở thành `session.export_curl(query)` — một lời gọi đồng bộ xử lý hoàn toàn bên trong Session/Screen riêng của `tradar-elasticsearch` — loại bỏ một chỗ rò rỉ logic riêng của connector vào code dùng chung vốn đã tồn tại từ trước.

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
- **`Workspace → Tab → Screen` thay vì một `ScreenSlot::Active(Box<dyn Component>)` duy nhất.** Xem lại khi multi-tab hoặc split-view (vd hai connection mở song song) trở thành một tính năng thực sự được lên kế hoạch — nó đang ở backlog dưới mục "Sessions/workspace state" nhưng chưa được scope. (Được đề xuất lại 2026-08-08 như một phần của một kiến trúc lớn hơn — xem mục dưới; vẫn giữ nguyên quyết định gác lại vì lý do cũ chưa đổi.)
- **`ConnectorFactory` thay vì trait object `Connector` trực tiếp trong registry.** Xem lại nếu một connector cần khởi tạo non-singleton (vd instance riêng cho từng tab với config khác nhau), hiện chưa có gì cần việc này.

**Ghi nhận thêm 2026-08-08** — một buổi review kiến trúc rộng hơn đề xuất tổ chức lại toàn bộ dự án theo hướng "Engineering Workbench" thay vì theo từng database: thêm layer `Runtime` (workspace/scheduler/lifecycle/background task) tách khỏi App, tách UI thành `tradar-ui` (widget dùng chung) + `tradar-editor` (vim/textarea/cursor/completion) tách khỏi `tradar-query-workbench`, một loạt crate hạ tầng dùng chung (`tradar-table`, `tradar-tree`, `tradar-json`, `tradar-log`, `tradar-terminal`, `tradar-theme`, `tradar-keymap`, `tradar-command`, `tradar-icons`, `tradar-search`, `tradar-utils`, `tradar-plugin`), một AI service độc lập gắn vào editor, và các connector mới ngoài phạm vi hiện tại (Kubernetes, Docker, SSH). Tất cả các ý này **chưa được đưa vào shape mục tiêu ở trên** — cùng lý do như các mục phía trên: chưa có connector/tính năng cụ thể nào cần chúng, và phần lớn còn chưa nằm trong phạm vi v1/planned của `CLAUDE.md` hay `docs/backlog.md`. Ghi lại ở đây để trace, không phải để scaffold ngay:
  - **`tradar-runtime`** (workspace manager, scheduler, lifecycle, background task tách khỏi App/Session) — xem lại khi có nhu cầu thật sự ngoài event loop hiện tại của `main.rs` (vd nhiều workspace/tab chạy song song thực sự cần điều phối).
  - **`tradar-ui` / `tradar-editor` tách khỏi `tradar-query-workbench`** — xem lại khi có Screen không phải dạng query cần dùng lại editor/widget (hiện `edtui` chỉ được `QueryEditorComponent` dùng).
  - **Các crate hạ tầng dùng chung** (`tradar-table`, `tradar-tree`, `tradar-json`, `tradar-log`, `tradar-terminal`, `tradar-theme`, `tradar-keymap`, `tradar-command`, `tradar-icons`, `tradar-search`, `tradar-utils`) — xem lại từng cái khi code tương ứng thực sự tồn tại và bị dùng lại ở ≥2 chỗ; không tạo crate rỗng trước.
  - **`tradar-plugin`** — xem lại nếu dynamic loading (`.so`/`.wasm`) thực sự được quyết định làm, trái với "Non-goals" bên dưới hiện tại.
  - **AI service gắn vào editor** — xem lại khi tính năng AI cụ thể (completion, apply-patch, ...) được scope, hiện chưa có trong `docs/backlog.md`.
  - **Connector Kubernetes/Docker/SSH** — cùng nhóm với Kafka/RabbitMQ ở trên: hợp lý về mặt shape (`Connector`/`Session`/`Screen` đã tính tới các hệ thống không phải query), nhưng chưa có trong danh sách connector v1/planned; thêm vào danh sách connector tương lai khi thực sự được lên kế hoạch.

## Non-goals của kiến trúc mục tiêu

- Implement Kafka, RabbitMQ, Cassandra, hay bất kỳ connector mới nào khác — tài liệu này chỉ định nghĩa shape để chúng được xây dựng vào.
- Dynamic plugin loading (`.so`/`.wasm`, phân phối plugin bên thứ ba).
- Một UI "add connection" tương tác (vẫn sửa tay TOML).
- Bất kỳ UI nào thực sự branch theo `Capability` — enum và descriptor shape được định nghĩa sẵn bây giờ; việc dùng chúng là việc của tương lai.
- Bất kỳ thay đổi nào cho phần vim-modal query editor ngoài việc `QueryScreenComponent` chuyển vào `tradar-query-workbench`.
