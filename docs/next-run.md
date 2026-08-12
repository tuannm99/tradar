# Next run

Ghi chú bàn giao để tiếp tục công việc ở một session mới. File này là state
tạm/session, không phải bản ghi bền vững — `docs/backlog.md` (roadmap/trạng
thái) và `docs/architecture.md` (thiết kế) mới là docs bền vững; cập nhật hai
file đó và xoá hoặc làm rỗng file này khi nội dung đã cũ hoặc đã được xử lý.

## Trạng thái hiện tại (tính đến 2026-08-12)

- Working tree sạch tính đến hết session này.
- Migration kiến trúc connector pluggable (`docs/backlog.md` mục 6) đã xong
  hoàn toàn từ 2026-08-10, cả 4 bước.
- **Sub-project 3, "Lưu/mở query ra file", đã xong** (`docs/backlog.md` mục
  3): `Ctrl+S`/`Ctrl+O` trong `QueryScreenComponent`, dựng mới
  `FilePromptComponent`. Xem chi tiết đầy đủ trong `docs/backlog.md`, không
  lặp lại ở đây.

## Việc tiếp theo

Theo đúng roadmap đã thống nhất với user (2026-08-12): tiếp tục mục 4
("Sessions/workspace state") — **nhưng bước đầu tiên là một buổi brainstorm
để scope phạm vi**, chưa phải code. Đừng tự giả định phạm vi (query history?
nhiều tab/connection cùng lúc? state sống qua các lần chạy app?) — hỏi lại
user trước khi thiết kế. Sau đó mới tới mục 5 (UI polish), gom luôn các known
issue nhỏ đang treo trong `docs/backlog.md` khi tới lượt.

## Đừng quên

- `cargo fmt --all` + `cargo clippy --all-targets --workspace -- -D warnings`
  trước khi coi bất kỳ việc gì là xong.
- Test toàn bộ: `cargo test --workspace --lib --exclude tradar-elasticsearch --exclude tradar-postgres --exclude tradar-redis --exclude tradar-mongo`
  bỏ qua 4 crate cần Docker; chạy riêng từng crate đó
  (`cargo test -p tradar-elasticsearch --lib`, v.v.) nếu cần verify chúng,
  vì tên test trong các crate này không còn tiền tố `drivers::<tên>::` như
  trước khi tách workspace (chỉ còn `tests::...`), nên filter `--skip
  drivers::postgres` kiểu cũ không còn khớp gì khi chạy `--workspace`.
- Elasticsearch có xu hướng flaky khi chạy qua `testcontainers` (từng thấy
  `StartupTimeout`/`WaitContainer(WaitLog(EndOfStream([])))` thoáng qua,
  không liên quan tới code) — đừng hoảng nếu 3-4 test đó fail một lần rồi
  pass lại khi chạy riêng.
- File `~/.config/tradar/connections.toml` thật của user có 5 connection,
  không backend nào ngoài sqlite thực sự chạy local — đừng giả định các cái
  khác reachable khi test tay.
