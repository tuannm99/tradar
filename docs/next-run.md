# Next run

Ghi chú bàn giao để tiếp tục công việc ở một session mới. File này là state
tạm/session, không phải bản ghi bền vững — `docs/roadmap.md` (việc đang mở)
và `docs/backlog/` (việc đã xong) mới là docs bền vững; cập nhật hai chỗ đó
và xoá hoặc làm rỗng file này khi nội dung đã cũ hoặc đã được xử lý.

## Trạng thái hiện tại (tính đến 2026-09-07, cuối phiên)

- Working tree sạch, đã commit/push.
- Roadmap Tier 4 xong hết: #10 Multi-filter kết hợp (`cột:giá_trị` +
  `AND`/`OR` trong ô filter `/`, panel `F3` xem/xoá từng điều kiện) —
  `docs/backlog/multi-filter.md`.
- Row-edit mở cho MongoDB (`find()` đơn giản, khoá `_id`) và Elasticsearch
  (`_search` 1 index, khoá `_id`, `execute()` giờ tách hit thành từng
  document thay vì cả response gộp 1 document) — `docs/backlog/mongo-es-row-edit.md`.
  User yêu cầu ngoài roadmap, không phải mục đã liệt kê sẵn.
- `make check` (fmt-check + clippy + test-unit) sạch cho cả hai việc trên.
- **Chưa verify tay qua tmux cho row-edit Mongo/ES** — môi trường sandbox
  session này không có Docker daemon, không dựng được Mongo/ES thật để
  thao tác UI trực tiếp. Độ tin cậy hiện dựa vào unit test (không cần
  Docker) + integration test qua testcontainers (viết đủ, nhưng chưa chạy
  được ở đây) — chạy `make test-docker` khi có Docker để verify thật, và
  nếu tiện thì làm luôn phần verify tay qua tmux còn thiếu (kết nối
  Mongo/ES thật, sửa 1 cell, xoá 1 row, xem lại panel F3 filter không bị
  ảnh hưởng).

## Việc tiếp theo

Không có việc cụ thể nào đang dở — theo `docs/roadmap.md`'s checklist tổng
quan, Tier 5 (lớn, chưa scope) là mục còn lại gần nhất trong roadmap: #2
Table designer → #3 Schema diff/compare → #4 Migration/version-control —
mỗi mục cần `AskUserQuestion` riêng trước khi code, đừng tự chọn hướng.

## Đừng quên

- `cargo fmt --all` + `cargo clippy --all-targets --workspace -- -D
  warnings` (hoặc `make check`) trước khi coi bất kỳ việc gì là xong.
- `make test-unit` bỏ qua 8 connector cần Docker (Postgres/Redis/Mongo/
  Elasticsearch/Cassandra/RabbitMQ/Kafka/HTTP); `make test-docker` chạy
  riêng chúng nếu cần verify — môi trường sandbox này không có Docker
  daemon, chưa thử được `test-docker` ở session nào tới giờ.
- Container sandbox này thiếu sẵn `libcurl4-openssl-dev` (Kafka connector
  cần để build `librdkafka` qua cmake) — `apt-get install -y
  libcurl4-openssl-dev` trước khi build nếu gặp lỗi `curl/curl.h: No such
  file or directory`.
