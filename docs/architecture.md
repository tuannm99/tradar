# Architecture

Mô tả **kiến trúc hiện tại** của tradar — hình dạng thật của code hôm nay, không phải nhật ký quá trình đi tới đó. Lý do/đánh đổi cho từng quyết định, kèm bug phát hiện lúc làm và test đã chạy, nằm trong `docs/backlog/` (một file mỗi sub-project, mục lục ở `docs/backlog/README.md`) — file này chỉ trỏ sang khi cần.

## Hai giao diện, một bộ connector

`tradar` có hai client dùng chung đúng một bộ connector crate:

- **Neovim + `tradar-server`** — giao diện chính. `tradar-server` là một JSON-RPC server headless giữ mọi connection đã mở; nhiều session Neovim dùng chung một tiến trình server. Xem "Server headless" bên dưới.
- **TUI (`tradar-app`)** — **deprecated** (không phát triển thêm, không sửa chủ động; chỉ làm khi có người yêu cầu). Vẫn phải build/test qua, vẫn là một client hợp lệ của cùng connector.

Cả hai không phụ thuộc lẫn nhau — chúng chỉ cùng phụ thuộc `tradar-query-workbench`/`tradar-connector-spi`/các connector crate.

## Bố cục workspace

```
Cargo.toml                    [workspace], default-members = ["crates/tradar-app"]
crates/
  tradar-core/                 — Action, trait Component, Capability, storage, config, theme, keymap, ui (widget dùng chung)
  tradar-connector-spi/        — trait Connector, trait Session, ConnectorDescriptor, CONNECT_TIMEOUT
  tradar-query-workbench/      — trait QueryDriver, QueryEngine (implement Session), QueryScreenComponent + sub-component
  tradar-connector-{postgres,mysql,sqlite,mongo,elasticsearch,redis,cassandra,clickhouse}/
                                — implement QueryDriver + Connector/Session, dùng chung QueryScreenComponent
  tradar-connector-{kafka,rabbitmq,http,socket}/
                                — không có ngôn ngữ query, tự implement Connector/Session + Screen riêng (screen.rs)
  tradar-app/   [[bin]] "tradar" — registry() + event loop (TUI)
  tradar-server/                — JSON-RPC server headless cho Neovim (xem "Server headless")
```

Hướng phụ thuộc, Cargo enforce (không chỉ quy ước):

- `tradar-core` không phụ thuộc gì trong workspace.
- `tradar-connector-spi` phụ thuộc `tradar-core`.
- `tradar-query-workbench` phụ thuộc `tradar-core` + `tradar-connector-spi`.
- Mỗi connector crate phụ thuộc `tradar-connector-spi`, và phụ thuộc `tradar-query-workbench` chỉ khi có ngôn ngữ query (8 connector dạng SQL/document). 4 connector còn lại (Kafka/RabbitMQ/HTTP/Socket) chỉ phụ thuộc `tradar-connector-spi`/`tradar-core`.
- `tradar-app` là crate duy nhất phụ thuộc mọi connector crate. Không connector nào phụ thuộc connector khác hay phụ thuộc `tradar-app`.

**Quy tắc cách ly**: code trong mỗi connector crate chỉ implement `QueryDriver`/`Connector`/`Session`, không `pub` ra struct cụ thể nào — chỉ `pub fn connector() -> Box<dyn Connector>`. Thêm một database mới = một crate connector mới implement đúng các trait này, một dòng dependency + feature trong `tradar-app/Cargo.toml` (và `tradar-server/Cargo.toml` nếu nó có `QueryDriver`), một dòng trong `registry()`. Không bao giờ cần sửa `tradar-query-workbench`, `tradar-connector-spi`, `tradar-core`, hay connector crate khác.

## `Connector` → `Session` → `Screen`

```rust
#[async_trait]
pub trait Connector: Send + Sync {
    fn descriptor(&self) -> &ConnectorDescriptor;
    async fn connect(&self, connection: SavedConnection) -> anyhow::Result<Box<dyn Session>>;
}

pub trait Session: Send + Sync {
    fn tick(&mut self) -> bool; // rút channel nội bộ có giới hạn mỗi lần gọi, trả true nếu có gì đổi
    fn build_screen(self: Box<Self>, action_tx: UnboundedSender<Action>, restore: Option<&str>) -> Box<dyn Component>;
}
```

- **Connector**: factory gần như stateless — nhận `SavedConnection`, tạo `Session`. Nơi duy nhất làm handshake ban đầu.
- **Session**: actor sống lâu dài, sở hữu mọi IO — connection/client handle, background task, channel nội bộ, cache. **Chỉ Session được spawn task hay giữ channel** — Screen không bao giờ làm IO trực tiếp. Một phím bấm → lời gọi method đồng bộ trên Session (vd `session.submit_query(text)`) → Session spawn task cho IO thật → `tick()` rút kết quả có giới hạn mỗi lần gọi (firehose-safe: một Kafka consumer hàng nghìn msg/giây không được phép làm đói render).
- **Screen**: implement `Component`, cái `RootComponent` thực sự giữ và route phím/draw tới. Đọc state của Session để render.

Connector dạng query: `QueryEngine` (trong `tradar-query-workbench`) implement `Session`, `build_screen()` trả `QueryScreenComponent`. Bốn connector phi-query (Kafka/RabbitMQ/HTTP/Socket): mỗi cái tự implement `Session` + `Screen` riêng (`KafkaScreen`/`RabbitScreen`/`HttpScreen`/`SocketScreen`), không sidebar chung — mỗi connector có "entity cấp 1" hình dạng khác (topic/queue/request/raw byte), không ép vào cây navigator chung.

`Component` (`tradar-core`) có nhiều method với default — chỉ `handle_key_event`/`update`/`draw` bắt buộc. Một Screen mới chỉ cần 3 cái đó cộng `tick` (forward `Session::tick()`) và override default nào nó thực sự cần (`outline`, `connection_alive`, `status_hints`, ...).

## Trait `QueryDriver`

Tám connector dạng query (Postgres, MySQL, SQLite, MongoDB, Elasticsearch, Redis, Cassandra, ClickHouse) implement chung một trait (`crates/tradar-query-workbench/src/query_driver.rs`; kiểu dữ liệu, `fetch`, `edit`, `snippet`, `completion`... nằm trong các module con cùng thư mục, re-export qua `query_driver::*`):

```rust
#[async_trait]
pub trait QueryDriver: Send + Sync {
    async fn connect(&mut self) -> anyhow::Result<()>;
    async fn list_schema(&self) -> anyhow::Result<Vec<SchemaInfo>>;
    async fn execute(&self, query: &str) -> anyhow::Result<QueryResult>;
    fn export_curl(&self, _query: &str) -> Option<String> { None }
    fn edit_source(&self, _query: &str) -> Option<String> { None }
    fn edit_sql(&self, _edit: &RowEdit) -> Option<String> { None }
    fn insert_sql(&self, _table: &str, _values: &[(String, String)]) -> Option<String> { None }
    async fn ping(&self) -> anyhow::Result<()> { Ok(()) }
    fn in_transaction(&self) -> bool { false }
    fn cancel_query(&self) -> anyhow::Result<()> { Ok(()) }
    fn column_sources(&self, _query: &str, _columns: &[String]) -> Option<Vec<Option<String>>> { None }
    fn edit_key_columns(&self) -> Option<&[String]> { None }
}
```

Mọi method ngoài 3 cái đầu đều có default — driver nào không override thì hành xử y như trước khi method đó tồn tại. Điểm chính:

- `QueryResult` là enum `Table { columns, rows, truncated }` (SQL/ClickHouse) hay `Documents { items, truncated }` (Mongo/ES/Redis) — `truncated` báo kết quả bị cắt ở `MAX_ROWS`, không phải toàn bộ.
- `edit_source`/`edit_sql`/`edit_key_columns` là cặp method đứng sau sửa cell/xoá dòng ngay trên bảng kết quả — chỉ driver mới biết cú pháp của chính nó, `tradar-query-workbench` không tự viết câu lệnh sửa. `column_sources` giải quyết cùng câu hỏi cho kết quả `JOIN` nhiều bảng (ai trong `columns` đến từ bảng nào).
- `ping`/`in_transaction` phục vụ badge UI (connection rớt, đang trong transaction) — round-trip rẻ nhất driver có.
- `cancel_query` là huỷ **thật** phía database (khác `cancel()` chỉ bỏ future đang chờ) — chỉ Postgres (`pg_cancel_backend`) và SQLite (`sqlite3_interrupt` qua FFI) override.

Phạm vi ngôn ngữ: Postgres/MySQL/SQLite/ClickHouse/Cassandra nhận SQL/CQL tuỳ ý (`SQL_KEYWORDS`/`split_sql_statements` dùng chung). Elasticsearch (Dev Tools console style: `METHOD /path` + JSON body), Redis (một dòng lệnh, tách khoảng trắng), MongoDB (`db.<collection>.<method>(...)`, chain `find().sort().limit()`) chỉ chấp nhận một tập con có chủ đích, không phải toàn bộ ngôn ngữ gốc.

## `ConnectorDescriptor` và `Capability`

```rust
pub enum Capability { Query, Schema, Streaming, Publish, Tail, Explain, Export }
pub struct ConnectorDescriptor { pub id: &'static str, pub display_name: &'static str, pub icon: &'static str, pub capabilities: &'static [Capability] }
```

Khai báo một connector *có thể làm gì* mà không hardcode danh tính — chưa có UI nào branch theo field này.

## Registry + feature flags

`registry()` (`tradar-app/src/main.rs`, và tương tự trong `tradar-server`) là nơi duy nhất biết toàn bộ tập connector: gọi `connector()` của mỗi crate đã compile vào, dựng `HashMap<String, Box<dyn Connector>>` key theo `descriptor().id`. Mỗi connector nằm sau một Cargo feature cùng tên id (`postgres`, `kafka`, ...), `default` gộp tất cả. Một `connection.driver` không match trong registry là lỗi runtime (`"unknown connector '{id}': not compiled into this build"`), không phải lỗi compile-time.

## Server headless (`tradar-server`) + plugin Neovim

```
nvim (nvim/lua/tradar)  ──JSON-RPC, từng dòng, unix socket──>  tradar-server  ──>  QueryDriver  ──>  database
```

`tradar-server` phụ thuộc `tradar-core`/`tradar-connector-spi`/`tradar-query-workbench` + các connector crate có `QueryDriver` qua feature (cùng tên feature `tradar-app` dùng). `gRPC`/`Kafka`/`Socket`/`RabbitMQ`/`HTTP` không đi qua `registry`/`QueryDriver` (không có ngôn ngữ query) — mỗi cái sống trong module riêng (`grpc.rs` + thư mục `grpc/`, `kafka.rs`/`socket.rs`/`rabbitmq.rs`/`http.rs`), **không tái dùng connector crate TUI tương ứng** (shape `Session` của TUI dựng quanh `tick()` cho vòng lặp vẽ, không hợp một server JSON-RPC thuần), tự nói thẳng với client library hoặc (RabbitMQ/HTTP) tự gọi `reqwest`.

Mỗi module implement trait `Extension` (`extension.rs`): tự giữ state của mình, trả lời các method `<driver>.*`, báo `is_connected` cho `connections.list`, tự dọn khi client ngắt kết nối (`client_disconnected`) và tự nhận `cancel` cho call đang chạy. `server.rs` (cùng `server/connections|query|editing.rs`) chỉ route theo tiền tố method tới extension tương ứng — không có field, nhánh dispatch hay feature gate riêng cho từng connector; danh sách extension (và feature gate của chúng) nằm đúng một chỗ, `extension::all()`. Thêm một connector kiểu này = một module mới + một dòng trong `extension::all()`, không sửa `server.rs`.

**Transport.** Mỗi dòng là một JSON-RPC 2.0 request/response (không batch). Request trên cùng kết nối chạy đồng thời (mỗi cái một task), reply có thể tới không theo thứ tự gửi — client ghép theo `id`. Socket mặc định `$XDG_RUNTIME_DIR/tradar/server.sock`, thư mục `0700`, socket `0600`.

**Method cốt lõi** (connector có `QueryDriver`):

| Method | Params | Kết quả |
|---|---|---|
| `version` | — | `{major, minor, exe, stale}` — `exe` là binary đang chạy, `stale` = binary trên đĩa đã được build lại sau khi server khởi động (cần `:TradarRestart`); xem "Phiên bản hoá protocol" dưới |
| `connectors.list` / `connections.list` | — | id connector build vào server / connection đã lưu + `connected`/`supported` (file được đọc lại mỗi lần gọi) |
| `connections.path` | — | `{path}` của `connections.toml` server đang dùng (`null` nếu không có store) — để plugin mở đúng file |
| `connect` / `disconnect` | `connection` | trạng thái |
| `status` | `connection` | `alive`, `in_transaction` |
| `schema` | `connection` | các entry (`name`, `schema`, `object_kind`, `columns[]`) |
| `execute` | `connection`, `query`, `page_size?`, `query_id?` | `kind` = `table`/`documents`/`affected`, trang đầu `rows` |
| `fetch` | `cursor`, `offset?`, `limit?` | trang tiếp theo |
| `cancel` | `query_id` | huỷ (client ngừng chờ + `cancel_query` phía DB nếu có) |
| `edit.source` / `edit.sql` | tuỳ method | bảng nguồn + cột khoá / câu lệnh sửa dòng (chỉ sinh, không chạy); `edit.sql` nhận `insert: {cột: giá trị}` (không `key`/`change`) để sinh `INSERT` — `sql: null` nếu connector không thêm dòng được |
| `complete` / `snippet` / `split` / `keywords` | tuỳ method | completion / CRUD snippet / tách statement / từ vựng |
| `shutdown` | — | server dừng sau khi reply (dùng để thay binary không cần `kill`) |

Lỗi: `-32601` method lạ, `-32602` params sai, `-32000` lỗi driver/server (kèm `data.line`/`data.column` nếu driver báo được vị trí), `-32700` JSON hỏng.

**Connector phi-`QueryDriver`**, mỗi cái một nhóm method riêng, không `ClientConnection`/push trừ khi ghi chú:

- **gRPC** (`grpc.*`) — unary qua `grpc.invoke`, server-/client-streaming và bidi qua `grpc.stream.start/send/close/stop` (kênh đẩy `grpc.stream.message` `{subscription, kind: headers|message|trailers|sent|end|error}`, cùng khuôn với Socket tail); `metadata` (ASCII) và `timeout_ms` (deadline) nhận ở `invoke`/`stream.start`, target `https://` bật TLS (root webpki; `?ca=`/`cert=`/`key=`/`domain=` trong query của target cho CA riêng, mTLS, đổi tên kiểm tra), response trả `headers` (và `trailers` cho stream); `grpc.methods` báo `client_streaming`/`server_streaming`; server reflection + fallback `.proto` (`protox`), invoke động qua `prost-reflect::DynamicMessage` (không codegen Rust type nào); `grpc.skeleton` sinh khung request từ descriptor của message đầu vào (mọi trường ở giá trị mặc định, đúng thứ tự khai báo, một nhánh `oneof`, message tự tham chiếu dừng ở tầng hai).
- **Kafka tail** (`kafka.*`) — **dùng push-channel** (xem dưới): `kafka.tail.start` trả `subscription` ngay, message thật tới sau qua notification `kafka.tail.message`.
- **Socket tail** (`socket.*`) — cũng push-channel; một "connection" Socket là chính phiên TCP, `socket.tail.start` dial + tail trong một lời gọi.
- **RabbitMQ** (`rabbitmq.*`) — không cần push (Management HTTP API không có endpoint stream), request/response thuần.
- **HTTP** (`http.*`) — không cần push, không liveness probe (`http.connect` không gọi gì ra ngoài).

**Push-channel (notification server→client).** Kafka tail/Socket cần server tự gửi dữ liệu mới bất cứ lúc nào, không đợi client hỏi. Cơ chế: mỗi connection có sẵn một channel riêng (`ClientConnection`, id + `outbox`); push là một JSON-RPC notification thường (có `method`/`params`, **không có** `id`) gửi qua cùng dòng chảy — không transport mới. Mỗi subscription một task nền riêng, không share/dedup giữa nhiều subscriber. Bounded cap 500, drop-oldest khi client đọc chậm. Dọn theo connection: client đóng socket thì mọi subscription nó chưa tự dừng bị abort theo (`client_disconnected`). Phía plugin, `rpc.lua`'s `on_line` phân biệt reply (có `id`) với notification (không `id`, route theo `method` qua `rpc.on(method, cb)`).

**Log.** Server ghi file log (mặc định `$XDG_STATE_HOME/tradar/server.log`, hoặc `~/.local/state/tradar/server.log`; `TRADAR_LOG_FILE` đổi đường dẫn, `TRADAR_LOG=off|error|warn|info|debug|trace`, mặc định `info`): khởi động/tắt, client vào/ra, mỗi request (`method [connection] ok|failed <ms>`; `status`/`complete`/`fetch`… chỉ ở `debug`). Không ghi câu lệnh hay mật khẩu; lỗi chỉ ghi dòng đầu (database có thể trích một đoạn câu lệnh). Quá 5 MB thì lúc khởi động đổi tên thành `server.log.1`. `:TradarLog` mở và bám theo file này.

**Connection nhàn rỗi.** Connection không có lệnh nào gọi tới trong `TRADAR_IDLE_TTL` (mặc định 30 phút; số giây hoặc `10m`/`2h`; `0`/`off` tắt) thì server đóng để trả RAM (pool, buffer của driver). Tính theo lần cuối có request nêu `connection` đó — `status` (poll nền của plugin) không tính. Không đóng khi đang trong transaction hoặc đang có query chạy. Lần gọi sau chỉ báo `is not connected`, plugin tự `connect` lại rồi thử lại. Chỉ áp cho connection truy vấn (`state.drivers`); Kafka/RabbitMQ/HTTP/Socket/gRPC có session riêng chưa tính. Ghi log `idle-disconnect [tên]`.

**Phiên bản hoá protocol.** `protocol::PROTOCOL_VERSION: (u32, u32)` (bắt đầu `1.0`, hiện `1.12`: `connections.path`, `edit.sql` với `insert`, `version.exe`/`version.stale`, `grpc.skeleton` (1.4), `edit.sql` nhận `change.set` là danh sách `{column, value}` — nhiều ô của một dòng trong một câu (1.5), `grpc.stream.*` + `metadata`/`headers` + cờ streaming ở `grpc.methods` (1.6), `grpc.skeleton` nhận `comments` — dòng `//` gợi ý từ comment `.proto`/enum/oneof (1.7), `edit.sql` nhận `insert_columns` — thứ tự cột của `INSERT` (1.8), `execute` trả `elapsed_ms` — thời gian database chạy câu lệnh (1.9), `execute` nhận `max_rows` — số dòng tối đa đọc từ database, mặc định 1000 (1.10), `version` trả thêm `log_path` — file log của server (1.11), `version` trả thêm `idle_ttl_secs` — số giây không dùng thì server đóng connection, `null` = tắt (1.12)) — bump major khi một client cũ có thể hiểu sai (method xoá, param bắt buộc đổi, field kết quả đổi kiểu); bump minor khi thêm thuần tuý, an toàn bỏ qua. Method `version` không sau feature gate nào. Chỉ cảnh báo (`vim.notify`/`:checkhealth`), không chặn lời gọi — công cụ một người dùng cục bộ, server/plugin thường cùng commit.

**Plugin (`nvim/`).** Mã Lua chia theo việc: `init.lua` (nối các phần + `setup`/`attach`), `ctx.lua` (bảng `M`, `opts`, `state` dùng chung), `conn` (kết nối, statusline), `results`/`resultskeys` (buffer kết quả), `running` (chạy câu lệnh), `rowedit` (sửa dòng, preview, EXPLAIN), `nav` (schema, hover, `gd`), `complete`, `history`, `connfile`; mỗi module gắn hàm vào cùng bảng `M` và gọi hàm của module khác qua `ctx.<tên>`. Còn lại: Cài qua `lazy.nvim`. Gắn connection theo file (dòng `-- tradar: <tên>`, file `.tradar`, hoặc `:TradarConnect`). `<leader>rr`/`<leader>ra`/`<leader>rx` chạy/chạy hết/huỷ — async hoàn toàn. Kết quả trong buffer thường; trang kế tải bằng `<leader>rm` (mặc định server đọc tối đa `max_rows` = 1000 dòng). Completion qua `blink.cmp`/omnifunc. `<leader>rt`/`<leader>rh`/`<leader>rs`/`<leader>rc` điều hướng bảng/lịch sử/schema/connection. Lưới kết quả sửa trực tiếp bằng lệnh vim; `:w` mở xem trước các câu `UPDATE`/`DELETE`/`INSERT` rồi commit một transaction. Bảo vệ: `UPDATE`/`DELETE` thiếu `WHERE`, `DROP`, `TRUNCATE`, connection `protected` luôn hỏi. Định dạng `.tdb` gộp nhiều dialect/connection trong một file (block fenced-code kiểu Markdown). Mỗi connector phi-`QueryDriver` có module Lua riêng (`kafka.lua`/`socket.lua`/`rabbitmq.lua`/`http.lua`/`grpc.lua`) và command riêng — xem `nvim/README.md` để biết chi tiết dùng.

## Non-goals

- Dynamic plugin loading (`.so`/`.wasm`, hệ sinh thái plugin bên thứ ba) — connector là Rust crate compile tĩnh vào binary.
- Kubernetes/SSH/Docker connector — shape `Connector`/`Session`/`Screen` đã tính tới các hệ thống không phải query, nhưng chưa có kế hoạch cụ thể.
- Bất kỳ UI nào thực sự branch theo `Capability` — enum/descriptor đã định nghĩa sẵn, dùng nó là việc tương lai.
