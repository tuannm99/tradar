# Next run

Ghi chú bàn giao để tiếp tục công việc ở một session mới. File này là state
tạm/session, không phải bản ghi bền vững — `docs/roadmap.md` (việc đang mở)
và `docs/backlog/` (việc đã xong) mới là docs bền vững; cập nhật hai chỗ đó
và xoá hoặc làm rỗng file này khi nội dung đã cũ hoặc đã được xử lý.

## Trạng thái hiện tại (tính đến 2026-09-10, cuối phiên)

- Working tree sạch, đã commit/push. Nhánh reset từ `master` sau khi PR #2
  (multi-filter + row-edit Mongo/ES) merge, theo đúng quy tắc branch-đã-merge.
- Mongo: chain `sort`/`limit`/`skip`/`count()` sau `find()`, lệnh mới
  `findOne`/`countDocuments`, `find()` nhận projection (đối số thứ 2) —
  `docs/backlog/mongo-chaining.md`. Phát hiện + sửa luôn 1 bug có sẵn
  không liên quan chaining: `split_top_level_args` không bỏ qua nội dung
  chuỗi JSON khi đếm độ sâu ngoặc (giá trị string chứa `}`/`,` làm sai
  parse). `edit_source` (row-edit) vẫn nhận `find()` có chain
  sort/limit/skip, từ chối khi có `count()`.
- `make check` sạch. Test pure-logic (parser, `run_method` validate-trước-khi-chạm-mạng)
  chạy thật, không cần Docker — kể cả các case trước đây phải cần Mongo
  thật mới verify được (lợi dụng: `Client::with_uri_str` không tự connect,
  nên build `Collection` handle vào cổng không nghe được vẫn an toàn nếu
  test không bao giờ `.await` một operation thật).
- **Vẫn chưa verify tay qua tmux + `make test-docker`** cho cả row-edit
  Mongo/ES (từ phiên trước) lẫn chaining/findOne/countDocuments/projection
  (phiên này) — sandbox chưa từng có Docker daemon ở bất kỳ session nào
  tới giờ.

## Việc tiếp theo

Không có việc cụ thể nào đang dở. User đang ưu tiên nhóm Mongo/Elasticsearch
— hai hướng còn lại đã hỏi nhưng chưa chọn (xem lịch sử hội thoại):
mở rộng Elasticsearch ngoài `_search` (GET .../_doc/<id>, multi-index,
_count/_msearch), hoặc rà lại toàn bộ 2 connector tìm bất cập khác. Hỏi lại
user trước khi tự chọn hướng nếu không có chỉ định mới.

Ngoài nhóm Mongo/ES: `docs/roadmap.md`'s checklist tổng quan, Tier 5 (lớn,
chưa scope) — #2 Table designer → #3 Schema diff/compare → #4
Migration/version-control — mỗi mục cần `AskUserQuestion` riêng trước khi
code.

## Đừng quên

- `cargo fmt --all` + `cargo clippy --all-targets --workspace -- -D
  warnings` (hoặc `make check`) trước khi coi bất kỳ việc gì là xong.
- `make test-unit` bỏ qua 8 connector cần Docker (Postgres/Redis/Mongo/
  Elasticsearch/Cassandra/RabbitMQ/Kafka/HTTP); `make test-docker` chạy
  riêng chúng nếu cần verify — môi trường sandbox này không có Docker
  daemon, chưa thử được `test-docker` ở session nào tới giờ.
- Mẹo verify logic Mongo/ES không cần Docker khi có thể: nếu một nhánh lỗi
  trả về trước khi `.await` một operation mạng thật, `mongodb::Client::with_uri_str`
  vào một địa chỉ không ai lắng nghe vẫn xây được `Collection` handle an
  toàn (driver connect lười) — xem `unreachable_collection()` trong
  `crates/tradar-connector-mongo/src/lib.rs`'s test module làm ví dụ.
- Container sandbox này thiếu sẵn `libcurl4-openssl-dev` (Kafka connector
  cần để build `librdkafka` qua cmake) — `apt-get install -y
  libcurl4-openssl-dev` trước khi build nếu gặp lỗi `curl/curl.h: No such
  file or directory`.
