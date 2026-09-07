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
- `make check` (fmt-check + clippy + test-unit) sạch; verify tay qua tmux
  với một SQLite thật (filter cột-cụ-thể, AND, OR, panel F3 xoá điều kiện —
  đều đúng).

## Việc tiếp theo

Theo `docs/roadmap.md`'s checklist tổng quan, Tier 5 (lớn, chưa scope) là
mục còn lại gần nhất: #2 Table designer → #3 Schema diff/compare → #4
Migration/version-control — mỗi mục cần `AskUserQuestion` riêng trước khi
code, đừng tự chọn hướng.

## Đừng quên

- `cargo fmt --all` + `cargo clippy --all-targets --workspace -- -D
  warnings` (hoặc `make check`) trước khi coi bất kỳ việc gì là xong.
- `make test-unit` bỏ qua 8 connector cần Docker (Postgres/Redis/Mongo/
  Elasticsearch/Cassandra/RabbitMQ/Kafka/HTTP); `make test-docker` chạy
  riêng chúng nếu cần verify — môi trường sandbox này không có Docker
  daemon, chưa thử được `test-docker`.
- Container sandbox này thiếu sẵn `libcurl4-openssl-dev` (Kafka connector
  cần để build `librdkafka` qua cmake) — `apt-get install -y
  libcurl4-openssl-dev` trước khi build nếu gặp lỗi `curl/curl.h: No such
  file or directory`.
