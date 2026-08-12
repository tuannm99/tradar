# Next run

Ghi chú bàn giao để tiếp tục công việc ở một session mới. File này là state
tạm/session, không phải bản ghi bền vững — `docs/backlog.md` (roadmap/trạng
thái) và `docs/architecture.md` (thiết kế) mới là docs bền vững; cập nhật hai
file đó và xoá hoặc làm rỗng file này khi nội dung đã cũ hoặc đã được xử lý.

## Trạng thái hiện tại (tính đến 2026-08-12, cuối phiên)

- Working tree sạch.
- Sub-project 3 ("Lưu/mở query ra file") và sub-project 4 ("Sessions/
  workspace state" — query history, nhiều tab/connection, state sống qua các
  lần chạy app) đều đã xong. Xem chi tiết đầy đủ trong `docs/backlog.md`,
  không lặp lại ở đây.
- 4 commit riêng biệt cho phiên này: save/load query, query history
  (`Ctrl+R`), multi-tab (`Ctrl+T`/`Ctrl+W`/`Ctrl+Left`/`Ctrl+Right`), session
  persistence (`Ctrl+Q` + `session.toml`).
- Đã verify tay qua tmux cho cả multi-tab (2 tab, mỗi tab connect độc lập,
  không đụng nhau) và session restore (connect → `Ctrl+Q` → mở lại → tự
  reconnect đúng connection).

## Việc tiếp theo

Theo `docs/backlog.md`, chỉ còn mục 5: **"Đánh bóng UI tổng thể"** — chưa bắt
đầu, cố ý để cuối. Gom luôn các known issue nhỏ đang treo trong
`docs/backlog.md` khi tới lượt (xem mục "Vấn đề đã biết").

Một việc chưa làm được ghi lại rõ trong mục 4 của `docs/backlog.md`: session
restore hiện chỉ nhớ lại *connection* của mỗi tab, không nhớ nội dung query
editor đang gõ dở. Muốn làm thì cần đổi `Session::build_screen` (trait ở
`tradar-connector-api`) để nhận thêm một `initial_query: Option<String>`,
đụng tới ranh giới connector — chưa có nhu cầu cụ thể nào đòi hỏi ngay, đừng
tự làm nếu user không yêu cầu.

## Đừng quên

- `cargo fmt --all` + `cargo clippy --all-targets --workspace -- -D warnings`
  trước khi coi bất kỳ việc gì là xong.
- Test toàn bộ: `cargo test --workspace --lib --exclude tradar-elasticsearch --exclude tradar-postgres --exclude tradar-redis --exclude tradar-mongo`
  bỏ qua 4 crate cần Docker; chạy riêng từng crate đó
  (`cargo test -p tradar-elasticsearch --lib`, v.v.) nếu cần verify chúng —
  tên test trong các crate này không còn tiền tố `drivers::<tên>::` như
  trước khi tách workspace (chỉ còn `tests::...`).
- Elasticsearch có xu hướng flaky khi chạy qua `testcontainers` (từng thấy
  `StartupTimeout`/`WaitContainer(WaitLog(EndOfStream([])))` thoáng qua,
  không liên quan tới code) — đừng hoảng nếu 3-4 test đó fail một lần rồi
  pass lại khi chạy riêng. Container `testcontainers` cũng có thể bị bỏ lại
  ở trạng thái `Exited` nếu test bị timeout/kill giữa chừng thay vì được
  Ryuk reaper dọn sạch — kiểm tra `docker ps -a` thỉnh thoảng nếu máy có vẻ
  chậm, không phải lỗi code.
- File `~/.config/tradar/connections.toml` thật của user có 5 connection,
  không backend nào ngoài sqlite thực sự chạy local — đừng giả định các cái
  khác reachable khi test tay. `~/.config/tradar/session.toml` giờ cũng là
  file thật (session persistence) — đừng xoá nó trừ khi đang dọn dẹp sau một
  lần test tay của chính mình.
- Chỉ kill/động vào các tmux session mà chính agent này tạo ra (đặt tên rõ
  ràng, vd `tradar-verify`) — không đụng session `tradar` sẵn có của user.
