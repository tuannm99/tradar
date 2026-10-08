# Huỷ query thật sự phía database, Postgres + SQLite (2026-10-08)

## Bối cảnh

Mục cuối cùng trong "Mục nhỏ" của `docs/roadmap.md`'s "Thứ tự tiếp theo" #3, sau `gd-fk-join-2026-10-07.md`. `cancel` (TUI's `QueryEngine::cancel()`, server's RPC `cancel`) từ trước tới nay chỉ abort/drop future đang `.await` chờ driver — client thôi chờ ngay, nhưng statement vẫn chạy tiếp thật phía database cho tới khi nó tự xong (hoặc lỗi). Người dùng chọn làm Postgres + SQLite trước (ưu tiên rõ nhất — hai driver được dùng nhiều nhất), để MySQL/Cassandra lại sau.

## Khảo sát trước khi code

**Postgres**: `sqlx` 0.8.6 giữ `PgConnection::process_id`/`secret_key` là field **private** (đọc source xác nhận, `#[allow(dead_code)]` ngay trên chúng — `sqlx` tự nó không dùng tới) — không có cách nào build một `CancelRequest` wire-protocol thật từ bên ngoài `sqlx-postgres`. Giải pháp không cần wire protocol: `SELECT pg_backend_pid()` (SQL thuần, Postgres cho mọi role tự biết pid của chính session mình) chạy trên đúng connection sắp chạy câu lệnh thật, lưu lại, rồi `SELECT pg_cancel_backend($1)` từ một connection khác trong cùng pool (Postgres cho một role huỷ query của chính session mình, không cần superuser).

**SQLite**: `sqlite3_interrupt()` (hàm C chuẩn để huỷ) chỉ lộ qua `SqliteConnection::lock_handle()` của sqlx — nhưng hàm đó gửi request qua đúng worker thread riêng của connection đó (sqlx-sqlite chạy mỗi connection trên một thread riêng vì SQLite không thread-safe mà không serialize), mà thread đó **đang bận block** trong lúc chạy chính câu lệnh cần huỷ — không có cách nào chen vào được đúng lúc cần. Giải pháp: lấy raw pointer `sqlite3*` qua `lock_handle()` **một lần, lúc connect()** (connection còn rảnh), giữ lại, gọi trực tiếp `sqlite3_interrupt()` qua FFI sau đó — không qua sqlx nữa, hợp lệ vì SQLite tự tài liệu hoá `sqlite3_interrupt` an toàn gọi từ thread khác bất cứ lúc nào. Cần thêm `libsqlite3-sys` trực tiếp (ghim đúng version `0.30.1` đã có sẵn trong `Cargo.lock` qua `sqlx-sqlite`, xác nhận Cargo chỉ compile một instance duy nhất) + `unsafe` để gọi FFI và giữ raw pointer qua `Send`/`Sync` thủ công — việc này đưa lên hỏi `AskUserQuestion` trước khi code vì đổi hẳn mức rủi ro (FFI/unsafe, dependency mới) so với mọi việc trước đó trong phiên này; chốt "làm cả hai luôn".

## Thay đổi

### `QueryDriver::cancel_query` (method mới, mặc định no-op)

```rust
async fn cancel_query(&self) -> anyhow::Result<()> { Ok(()) }
```

Chỉ Postgres/SQLite override. Scope theo "bất cứ thứ gì đang chạy trên driver instance này ngay lúc này" chứ không theo một call `execute()` cụ thể — đúng với cách mọi caller thật trong codebase dùng (TUI/Neovim: một `execute()` bay một lúc trên một connection; chỉ `tradar-server`'s protocol về lý thuyết cho phép chạy đồng thời, nhưng không có client thật nào trong repo khai thác điều đó cho cùng một connection) — không đảm bảo đúng cho trường hợp thật có nhiều `execute()` đồng thời trên cùng driver, ghi rõ trong doc comment.

### Postgres: `run_tracked` bọc ngoài `run`, không đổi `run`

`PostgresDriver` thêm `current: std::sync::Mutex<Option<(u64, i32)>>` (token + backend pid của call đang chạy) và `next_token: AtomicU64`. `run_tracked(&self, conn: &mut PgConnection, query)`:

1. Sinh `token` mới.
2. `SELECT pg_backend_pid()` trên đúng `conn` (connection sắp chạy câu lệnh thật — bắt buộc phải là connection *này*, không phải một connection khác lấy ngẫu nhiên từ pool, nếu không `pid` sẽ sai).
3. Lưu `(token, pid)` vào `current`.
4. Chạy `run(conn, query)` như cũ, không đổi gì bên trong `run`.
5. Dọn `current` — chỉ xoá nếu nó vẫn còn đúng `token` của chính call này (so sánh trước khi xoá), để một call chậm xong sau không xoá nhầm entry của một call khác đã bắt đầu muộn hơn.

`execute()` đổi từ chạy thẳng trên `pool`/`&mut **tx` sang gọi `run_tracked` — cho cả nhánh pool (giờ `pool.acquire()` lấy connection tường minh, thay vì để `run()` tự acquire-và-trả ngầm) và nhánh transaction đang mở (connection transaction giữ sẵn, deref coercion `&mut Transaction -> &mut PgConnection` tự động, không cần `&mut **tx` tường minh nữa — clippy chỉ ra `explicit_auto_deref`).

`cancel_query()`: đọc `current`, không có gì thì no-op; có thì `SELECT pg_cancel_backend($1)` qua `self.pool` (một connection *khác*, vì connection đang chạy câu lệnh chậm không rảnh để chạy thêm gì).

### SQLite: `RawHandle` + ép `max_connections(1)`

`connect()` đổi từ `SqlitePool::connect_with` sang `SqlitePoolOptions::new().max_connections(1).connect_with(...)` — **bắt buộc đúng 1 connection**, vì raw handle chỉ cache được từ một connection cụ thể; có nhiều connection trong pool thì cache đúng một trong số đó không đảm bảo mọi query sau này chạy trên chính nó. Ngay sau khi pool mở, `acquire()` một connection (duy nhất), `lock_handle()` (an toàn lúc này — connection còn rảnh, chưa chạy gì), lấy raw pointer qua `as_raw_handle()`, lưu vào field mới `handle: Option<RawHandle>` trên driver.

`RawHandle(*mut libsqlite3_sys::sqlite3)` — `unsafe impl Send + Sync` thủ công (con trỏ thô không tự `Send`/`Sync`), hợp lệ vì pointer chỉ dùng để gọi `sqlite3_interrupt` (SQLite tự tài liệu hoá an toàn đa luồng cho hàm này), không bao giờ dereference trực tiếp.

`cancel_query()`: nếu có `handle`, gọi `unsafe { libsqlite3_sys::sqlite3_interrupt(handle.0) }`. Không cần biết có đang chạy gì hay không — gọi vào lúc idle là no-op an toàn theo tài liệu SQLite.

### Caller: `QueryEngine::cancel()` (TUI) + server's `cancel` RPC

Cả hai giờ gọi thêm `driver.cancel_query()` bên cạnh việc abort/drop future như trước:

- `QueryEngine::cancel()` là hàm **sync** — fire-and-forget qua `tokio::spawn` (không đổi chữ ký hàm, không ai chờ cancel_query xong mới coi là "đã cancel" — đúng tinh thần best-effort).
- Server's `cancel` RPC đổi `fn cancel` thành `async fn cancel`, `await` trực tiếp `cancel_query()` trước khi trả response — đã là async context sẵn (dispatch qua `.await`), không có lý do fire-and-forget ở đây. `State.running` đổi giá trị từ `Arc<Notify>` sang `(Arc<Notify>, Arc<dyn QueryDriver>)` để `cancel` có driver đúng của connection đó mà không cần tra lại bằng tên connection (params của `cancel` chỉ có `query_id`, không có `connection`).

## Test

Phía Rust:

- Postgres: test tích hợp mới (Docker, không chạy được trong sandbox này) `cancel_query_really_stops_a_running_statement` — `SELECT pg_sleep(30)` chạy nền, `cancel_query()` sau 300ms, xác nhận `execute()` trả lỗi có chữ "cancel" (đúng thông báo Postgres báo khi một statement bị `pg_cancel_backend`) trong vòng 10s, không phải chờ hết 30s.
- SQLite: test tích hợp mới (không cần Docker, chạy thật trong sandbox này) `cancel_query_really_stops_a_running_statement` — recursive CTE không `LIMIT` cộng `count(*)` ở ngoài (buộc SQLite chạy toàn bộ trong **một** `sqlite3_step()` block, không có điểm dừng giữa dòng nào — đúng trường hợp khó nhất, test không được "ăn may" nhờ cơ hội dừng giữa các dòng). Chạy thật: **pass trong 0.21s**. Để chắc đây không phải false positive, viết thêm (rồi xoá sau khi xác nhận) một test "sanity check" chạy đúng câu lệnh đó **không** gọi cancel, bọc `timeout(2s)` — xác nhận nó **thật sự còn chạy** ở mốc 2s (timeout đúng là lỗi do hết giờ, không phải lỗi khác) — rồi xoá test đó khỏi bộ thường trực vì nó để lại một worker thread SQLite kẹt chạy mãi tới hết process (drop future không dừng được lời gọi FFI blocking phía dưới), rủi ro làm các test khác trong cùng binary chạy chậm/không ổn định do tranh CPU.
- `tradar-server`: test tích hợp mới `cancel_really_stops_the_statement_db_side_not_just_the_clients_wait` — khác biệt với test `cancel` cũ (chỉ xác nhận client thôi chờ): lợi dụng đúng việc pool SQLite giờ chỉ có 1 connection — nếu recursive CTE còn chạy thật, một `execute()` thứ hai trên cùng connection sẽ bị kẹt chờ pool nhả connection (mãi không nhả vì CTE không bao giờ tự xong); nếu `cancel_query` hoạt động đúng, connection nhả ngay, query thứ hai (`SELECT 1`) chạy xong trong vài trăm ms. Chạy thật: pass trong 0.21s (cả hai test cancel trong file này).
- `cargo build --workspace`, `cargo clippy --all-targets --workspace -- -D warnings`, `cargo fmt --all -- --check`, `make test-unit` đều sạch.

## Chưa làm

- MySQL/MariaDB, Cassandra, ClickHouse không có `cancel_query` thật — vẫn chỉ "client thôi chờ" như trước. MySQL có `KILL QUERY <id>` (cần `CONNECTION_ID()` giống hệt mẫu `pg_backend_pid()` của Postgres — cùng độ phức tạp, để sau vì không phải driver được ưu tiên lần này); Cassandra/ClickHouse chưa khảo sát.
- `cancel_query` không đảm bảo đúng khi có nhiều `execute()` thật sự đồng thời trên cùng một driver instance (chỉ `tradar-server`'s protocol lý thuyết cho phép, không có client nào trong repo khai thác) — `current`'s token-guard chỉ ngăn một call chậm xoá nhầm entry của call mới hơn, không đảm bảo `cancel_query()` nhắm đúng call nào trong số nhiều call bay cùng lúc. Ghi rõ trong doc comment của trait method, không giả vờ đã giải quyết.
- SQLite's pool giờ cứng `max_connections(1)` — đổi từ default (nhiều connection) xuống một, đổi nhẹ đặc tính đồng thời của driver này (hai request cùng lúc trên cùng connection giờ chạy tuần tự thay vì có thể song song) để đổi lấy real-cancel đáng tin cậy. Chưa thấy test nào trong repo phụ thuộc vào việc SQLite có nhiều connection, nhưng chưa benchmark tác động thật trên database lớn.
