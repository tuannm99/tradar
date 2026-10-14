# gRPC: 6 method mới ở tradar-server (2026-10-14)

## Bối cảnh

Sau khi chốt scope v1 (PR #32, 2026-10-12: đích `tradar-server`/Neovim, chỉ unary, không metadata) và spike xác nhận khả thi kỹ thuật dynamic-invocation (PR #33, 2026-10-12), đây là lần đầu code gRPC thật — nửa server. Nửa plugin Neovim (commands/UI gọi các method này) chưa làm, để lại thành việc kế tiếp (xem "Chưa làm").

## Vấn đề

gRPC không có ngôn ngữ query (`QueryDriver`) — khác 8 connector `tradar-server` đã phục vụ (`connect`/`execute`/`fetch`/...). Cần một con đường hoàn toàn riêng: không `Connector::connect` trả `Box<dyn Session>`, không `QueryEngine::driver()`, không `Arc<dyn QueryDriver>` trong `State::drivers`.

## Thay đổi

### Module `grpc.rs` mới (feature `grpc`, mặc định on)

Không implement `Connector`/`Session` của `tradar-connector-spi` — không có gì để implement, vì không có TUI screen nào tiêu thụ trait đó cho gRPC (xem "Chốt lại scope v1" trong `docs/architecture.md`). Thay vào đó là các hàm async/sync trần, gần như copy thẳng từ spike:

- `connect(target) -> Channel`: `target` là `host:port` trần hoặc URL đầy đủ, luôn thêm `http://` nếu chưa có scheme — plaintext only cho v1 (TLS để fast-follow, cùng non-goal Socket đã có).
- `discover_via_reflection(channel) -> DescriptorPool`: `ListServices` rồi `FileContainingSymbol` **mỗi service** (không chỉ một như spike — spike chỉ cần chứng minh khả thi cho một service, bản thật phải liệt kê hết), lọc bỏ `grpc.reflection.*` của chính reflection service.
- `load_proto(pool, path) -> Vec<String>`: fallback cho server không hỗ trợ reflection, parse qua `protox` (pure-Rust, không cần `protoc`), trả về tên service nào *mới* xuất hiện (so sánh trước/sau) để client biết vừa thêm được gì.
- `list_services`/`list_methods`/`invoke`: tra `DescriptorPool`, dùng `DynamicCodec`/`DynamicMessage` giống spike. `invoke` trả `Result<Result<Value, GrpcError>, anyhow::Error>` hai lớp — lớp ngoài là lỗi *trước khi* gọi được (request không khớp schema, channel không sẵn sàng), lớp trong là lỗi *từ* server (status code thật, cần giữ lại code số để client phân biệt).
- `GrpcError { code: i32, message: String }`, `From<Status>` — tách khỏi `anyhow::Error` để `protocol.rs` map đúng sang `RpcError::data = {grpc_code}`.

### `State::grpc: HashMap<String, GrpcConnection>` trong `server.rs`

`GrpcConnection { channel, pool }`, cạnh `State::drivers` chứ không thay nó — một connection tên `g` có thể là SQL (`drivers`) hoặc gRPC (`grpc`), không bao giờ cả hai. 6 method dispatch mới, toàn bộ `#[cfg(feature = "grpc")]` trên từng arm của `match` (không phải cả block `dispatch`) để method lạ vẫn rơi đúng về `-32601` khi build không có feature này, không phải lỗi biên dịch.

`connections.list`'s field `supported`/`connected` trước đây chỉ nhìn `self.registry` (danh sách `Connector`) — sai cho gRPC vì gRPC không có `Connector`. Thêm `Server::is_supported(driver)` kiểm cả hai đường; `connected` hợp nhất `state.drivers`/`state.grpc` qua một hàm nhỏ `connected_grpc` (có hai bản `#[cfg(feature = "grpc")]`/`#[cfg(not(...))]` để không cần `if cfg!` rải trong logic).

### `RpcError::from(GrpcError)` trong `protocol.rs`

`data: {"grpc_code": <số>}` — cùng ý tưởng `from_query_error`'s `{line, column}`: structured detail một client có thể branch theo mà không cần parse message.

## Test

6 test tích hợp mới (`tests/rpc_grpc.rs`), không mock — một server `greeter.Greeter` tầm thường thật (`tests/fixtures/greeter.proto`, build qua `build.rs` mới dùng `protox`+`tonic_build::compile_fds`, không cần `protoc`), chạy trên TCP loopback thật:

- `connect_discovers_services_via_reflection_and_invokes_unary`: connect → `grpc.services` → `grpc.methods` → `grpc.invoke`, đúng response.
- `a_grpc_status_error_carries_the_code_in_data`: server trả `INVALID_ARGUMENT`, xác nhận `error.data.grpc_code == 3`.
- `connecting_without_reflection_leaves_the_pool_empty_until_load_proto`: server tắt reflection, `grpc.services` rỗng, `grpc.load_proto` nạp từ file `.proto` thật, rồi invoke vẫn chạy đúng — xác nhận fallback cho ra một pool dùng được y như reflection.
- `disconnect_forgets_the_connection`, `connections_list_reports_the_grpc_connection_as_supported_and_connected`, `invoking_an_unknown_method_is_an_app_error_not_a_panic`.

`cargo build --workspace`, `cargo build -p tradar-server --no-default-features --features postgres,sqlite` (xác nhận cfg-gate đúng khi tắt `grpc`), `cargo clippy --all-targets --workspace -- -D warnings`, `cargo fmt --all -- --check`, `make test-unit` (167 + toàn bộ crate không-Docker, 0 lỗi) đều sạch.

## Chưa làm

- **Plugin Neovim** — commands/UI gọi `grpc.connect`/`services`/`methods`/`invoke`. Đây là phần lớn còn lại của "gRPC (tradar-server + Neovim)", chưa code dòng Lua nào.
- Repeated/nested message, `oneof`, `map` chưa có test riêng cho `DynamicMessage::deserialize`/serialize — tin vào cơ chế chung `prost-reflect` quảng cáo, chưa tự kiểm chứng từng shape phức tạp.
- Server-streaming: vẫn để fast-follow như đã chốt (PR #32) — cần kênh đẩy chưa có.
- Metadata/header request: vẫn để fast-follow như đã chốt.
- `load_proto`'s include path chỉ là thư mục cha của file — một `.proto` import file khác không nằm cạnh nó sẽ lỗi parse; chưa cho client truyền thêm include path.
