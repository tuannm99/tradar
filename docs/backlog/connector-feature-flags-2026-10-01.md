# Cargo feature flags cho từng connector (2026-10-01)

## Bối cảnh

Người dùng yêu cầu: "tách riêng binary theo từng loại... để tradar không kéo hết dependency vào, nếu người dùng chỉ muốn dùng mongo hay es". Trước khi code, trình bày 2 hướng kiến trúc khác hẳn nhau qua `AskUserQuestion`:

1. **Cargo feature flags, vẫn 1 binary `tradar`** — mỗi connector thành một feature, build chỉ compile connector được chọn.
2. **Binary riêng hẳn cho từng loại** (đúng nghĩa đen của yêu cầu) — crate binary mới cho mỗi nhóm (vd `tradar-mongo`, `tradar-es`...).

Người dùng chọn (1). Lý do được trình bày và chốt: (2) tạo N crate/binary mới, duplicate logic `main.rs` giữa chúng (hoặc phải factor lại thành lib dùng chung), và quan trọng nhất — phá mất trải nghiệm cốt lõi của app: **một connection picker/navigator duy nhất cho mọi loại connection đã lưu**, bất kể Postgres hay Mongo hay Kafka. Nếu tách binary thật, ai muốn dùng cả Postgres lẫn Mongo phải chạy 2 binary riêng, không còn "một app thống nhất" nữa — đi ngược premise cốt lõi ghi ngay đầu README/CLAUDE.md ("unified, keyboard-driven interface... without switching between multiple native CLI clients"). (1) giải quyết đúng vấn đề thật (dependency footprint) mà không đánh đổi điều đó, và khớp đúng kiến trúc Registry đã có sẵn từ trước (`docs/architecture.md`'s "Registry" section) — chỉ cần thêm feature gate vào đúng một chỗ (`tradar-app`), không đụng bất kỳ crate nào khác.

## Thiết kế

Mỗi connector trong `crates/tradar-app/Cargo.toml` đổi `optional = true`, thêm một feature cùng tên connector id (`postgres`, `sqlite`, `mongo`, `elasticsearch`, `redis`, `cassandra`, `clickhouse`, `rabbitmq`, `kafka`, `http`, `socket`) ánh xạ tới `dep:tradar-connector-<tên>`, cộng feature `full` gộp cả 11 và `default = ["full"]` — `cargo build`/`cargo run` trơn (không feature flag nào) vẫn y hệt trước đây, không ai bị bất ngờ.

`main.rs`'s `registry()` (nơi duy nhất biết toàn bộ connector, theo đúng thiết kế Registry có sẵn) — mỗi dòng `connectors.push(tradar_connector_x::connector())` bọc `#[cfg(feature = "x")]`. Không đổi gì ở `tradar-core`, `tradar-connector-spi`, `tradar-query-workbench`, hay bất kỳ connector crate nào — toàn bộ cơ chế nằm gọn trong `tradar-app`, đúng tinh thần Registry đã ghi từ đầu ("thêm connector = thêm 1 dòng dependency + 1 dòng registry, không đổi gì khác").

Thêm `#[allow(clippy::vec_init_then_push)]` trên `registry()` — clippy không hiểu các `#[cfg]` nên nghĩ `Vec::new()` + `push` liên tiếp nên gộp thành `vec![]`, nhưng `vec![]` không thể diễn tả một phần tử có điều kiện biên dịch.

## Build ra sao

- `cargo build` (mặc định) → đủ cả 11, y hệt trước đây.
- `cargo build -p tradar-app --no-default-features --features mongo,elasticsearch` → không kéo `rdkafka`/`scylla`/`sqlx`/... của 9 connector còn lại.
- `make build-slim FEATURES=mongo,elasticsearch` → lối tắt cho lệnh trên (target mới trong `Makefile`).
- Connection picker's form (`a` thêm connection) tự đồng bộ — `drivers` lấy thẳng từ `registry()` đã dựng, không cần sửa UI gì.

## Tác dụng phụ: `make test-unit` không còn cần "kafka-disable trick"

Trong suốt session trước đó, verify các thay đổi SQL/Elasticsearch/MongoDB phải dùng một "trick" thủ công: backup `tradar-app/Cargo.toml` + `main.rs` qua `cp`, xoá tạm dòng dependency/registry của kafka, chạy `cargo build/test/clippy --workspace --exclude tradar-connector-kafka ...`, rồi khôi phục qua `cp` (không dùng `git checkout`, tránh mất đổi thật) — vì `--exclude tradar-connector-kafka` ở mức workspace chỉ bỏ kafka khỏi *danh sách test*, không ngăn Cargo phải compile nó: `tradar-app` (không bị exclude) vẫn depend cứng vào kafka nên Cargo vẫn cần build nó để link.

Với feature flags, `make test-unit` (Makefile) giờ tách `tradar-app` ra khỏi lệnh `--workspace --exclude ...` lớn, chạy riêng bằng `cargo test -p tradar-app --no-default-features --features postgres,sqlite,mongo,elasticsearch,redis,cassandra,clickhouse,rabbitmq,http,socket` — không bao giờ chạm `tradar-connector-kafka` nữa. Verify trực tiếp: `make test-unit` chạy sạch từ đầu tới cuối, không cần sửa file nào bằng tay, trên chính sandbox này (không có `libcurl-dev`).

## Test

- `cargo build -p tradar-app --no-default-features --features postgres,sqlite,mongo,elasticsearch,redis,cassandra,clickhouse,rabbitmq,http,socket` (full trừ kafka): thành công.
- `cargo build -p tradar-app --no-default-features --features mongo,elasticsearch`: thành công, log compile xác nhận không có `rdkafka`/`scylla`/`sqlx`/postgres-sqlite-redis-cassandra-clickhouse-rabbitmq-http-socket crate nào.
- `cargo check -p tradar-app --no-default-features --features postgres` (chỉ 1 connector) và `--features socket` (connector nhẹ nhất): cả hai build sạch, riêng `socket` dựng xong trong 1.5s vì gần như không có thêm dependency.
- `cargo check -p tradar-app --no-default-features` (0 feature, trường hợp cực đoan không ai thật sự muốn) — compile được, chỉ cảnh báo `unused_mut` (vì không có `push` nào chạy) — không phải lỗi, không đáng sửa vì không phải tổ hợp build nào CI/người dùng thật sự cần.
- `cargo clippy -p tradar-app --all-targets --no-default-features --features <đủ trừ kafka> -- -D warnings`: sạch (sau khi thêm `#[allow(clippy::vec_init_then_push)]`).
- `cargo clippy --all-targets --workspace --exclude tradar-connector-kafka --exclude tradar-app -- -D warnings`: sạch.
- `make test-unit`: chạy sạch toàn bộ, không cần sửa file thủ công — 163 test `tradar-app` pass cộng toàn bộ test khác trong workspace (trừ 9 connector cần Docker + kafka).
- `make build-slim FEATURES=mongo,elasticsearch`: thành công.
- `cargo fmt --check`: sạch.

## Chưa làm

- Preset binary dựng sẵn qua CI release (vd `tradar-sql`, `tradar-nosql` build từ cùng crate với feature set khác nhau, đóng gói thành asset riêng trong GitHub Release) — có thể thêm sau mà không đổi gì ở thiết kế này, chỉ là thêm job CI.
- Không kiểm tra build thử từng combo trong 2^11 tổ hợp feature có thể có — chỉ verify: full trừ kafka, 2 connector (mongo+elasticsearch), 1 connector nặng (postgres), 1 connector nhẹ (socket), và 0 connector (biên, không phải use case thật).
