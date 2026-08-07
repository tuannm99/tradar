# Next run

Ghi chú bàn giao để tiếp tục công việc ở một session mới. File này là state
tạm/session, không phải bản ghi bền vững — `docs/backlog.md` (roadmap/trạng
thái) và `docs/architecture.md` (thiết kế) mới là docs bền vững; cập nhật hai
file đó và xoá hoặc làm rỗng file này khi nội dung đã cũ hoặc đã được xử lý.

## Trạng thái hiện tại (tính đến 2026-08-08)

- Working tree sạch, không còn gì pending, không còn gì chưa commit.
- **Đã bỏ workflow spec/plan của superpowers** cho dự án này (`docs/superpowers/`
  và `ARCHITECTURE_AUDIT.md` ở root đã xoá, commit `2411dff`). Đừng đề xuất
  `superpowers:brainstorming`/`writing-plans` cho tradar — thiết kế viết
  thẳng vào `docs/architecture.md`, roadmap/trạng thái vào `docs/backlog.md`.
- **Sub-project 2, "Vim keybinding xuyên suốt TUI", đã xong hoàn toàn**
  (`docs/backlog.md` mục 2): query editor (`edtui`), connection list/schema
  sidebar (`gg`/`G`/`Ctrl+d`/`Ctrl+u`), và panel results (selection +
  điều hướng + `y`-để-yank qua OSC52, không thêm dependency nặng) đều đã
  landed và được verify từ đầu tới cuối trên terminal thật (tmux), không
  chỉ unit test.
- **Đã fix một bug thật phát hiện lúc test tay**: `PostgresDriver::connect()`
  từng bị treo tới 30s không có phản hồi UI nào khi host không reachable
  (`acquire_timeout` mặc định của sqlx). Giờ giới hạn 5s
  (`PgPoolOptions::acquire_timeout`), kèm test hồi quy không cần Docker.
- **Đã tách Cargo workspace (bước 1 của migration kiến trúc connector
  pluggable) — xong (2026-08-08).** Xem mục dưới.
- **Rule mới (2026-08-08): toàn bộ docs (`docs/*.md`, `README.md`) viết
  tiếng Việt**, giữ nguyên các từ/thuật ngữ chuyên ngành (tên crate, tên
  trait, tên hàm, tên lệnh cargo, code block...) — không dịch toàn bộ vì sẽ
  mất nghĩa. Claude cũng trả lời user bằng tiếng Việt trong dự án này, cùng
  quy tắc giữ thuật ngữ. `CLAUDE.md` (file cấu hình/hướng dẫn cho Claude Code,
  không phải doc cho người dùng) tạm giữ tiếng Anh — chưa được xác nhận rõ có
  nằm trong phạm vi rule này hay không, hỏi lại user nếu cần chắc chắn. Đã áp
  dụng: dịch toàn bộ `docs/architecture.md`, `docs/backlog.md`, `README.md`,
  file này (`docs/next-run.md`) sang tiếng Việt trong session này.

## Việc tiếp theo: tiếp tục migration kiến trúc connector pluggable

User xác nhận (2026-08-08) muốn làm việc này sau khi sub-project vim
keybinding đã xong.

**Bước 1 (tách workspace thuần cơ học) đã xong, trong session này
(2026-08-08).** Có một điều chỉnh so với kế hoạch ban đầu, phát hiện trong
lúc tách: `Action` *không thể* chuyển vào `tradar-core` "nguyên trạng, không
đổi" như dự định ban đầu — nó phụ thuộc trực tiếp `QueryEngine` và
`drivers::{QueryResult, SchemaInfo}`, và signature `update()` của trait
`Component` tham chiếu `Action` theo value, nên hai cái không thể tách qua
ranh giới crate mà không tạo cycle. Chỉ `storage/` và `config/` (hai module
duy nhất không phụ thuộc gì khác) thực sự chuyển vào `tradar-core`;
`action.rs`, `components/`, `drivers/`, `query_engine/`, và `main.rs` đều
chuyển vào `tradar-app` không đổi gì khác ngoài việc giờ phụ thuộc
`tradar-core` cho `SavedConnection`/`DriverKind`/`ConnectionStore`. Việc
`Action` thực sự chuyển vào `tradar-core` xảy ra cùng lúc với bước 3 bên
dưới, một khi nó được tách khỏi `QueryEngine`/`drivers`. Mục "Triển khai
hiện tại" của `docs/architecture.md` và `README.md` đã cập nhật khớp. Đã
verify: `cargo build`, `cargo fmt --check`,
`cargo clippy --all-targets --workspace -- -D warnings`, full test suite
(94 test, cùng skip list như trước), và `cargo run` đều pass; binary vẫn
tên `tradar` (`[[bin]] name = "tradar"` trong `tradar-app/Cargo.toml`,
workspace `default-members = ["crates/tradar-app"]` nên `cargo run` trần
vẫn chạy được, không cần flag `-p`).

**Quyết định 2026-08-08 (chưa implement, ghi lại để trace):** `Connector`/
`Session`/`ConnectorDescriptor` sẽ vào một crate riêng `tradar-connector-api`
thay vì `tradar-core`, để tránh `tradar-core` phình thành "god crate" chứa
đủ thứ không liên quan. Quyết định này rẻ vì chưa có code Connector/Session
nào tồn tại (bước 3 chưa bắt đầu). `docs/architecture.md` mục "Bố cục
workspace" đã cập nhật theo hướng này.

Cùng lúc, một buổi review kiến trúc rộng hơn (2026-08-08) đề xuất tổ chức
lại toàn dự án theo hướng "Engineering Workbench" — thêm `tradar-runtime`,
tách `tradar-ui`/`tradar-editor` khỏi `tradar-query-workbench`, ~15 crate hạ
tầng dùng chung (`tradar-theme`, `tradar-keymap`, `tradar-table`, ...), layer
`Workspace → Tab → Screen` ngay từ đầu, AI service, và connector cho
Kubernetes/Docker/SSH. **Đã quyết định KHÔNG áp dụng ngay** — phần lớn chưa
có code, nhiều thứ chưa nằm trong phạm vi v1/planned của `CLAUDE.md`, và
`Workspace → Tab` đã từng được cân nhắc và cố tình gác lại trong
`docs/architecture.md` với đúng lý do "multi-tab chưa được scope". Toàn bộ
ý này đã ghi vào mục "Đã cân nhắc và gác lại" của `docs/architecture.md`
kèm điều kiện xem lại cho từng cái — không tạo crate rỗng, không dựng
Workspace/Tab layer bây giờ.

Các bước còn lại (đã thống nhất với user, tránh một diff khổng lồ khó review):

2. **Chuyển UI dạng query vào `tradar-query-workbench`**: `QueryScreenComponent`
   + `QueryEditorComponent`/`ResultsComponent`/`SchemaSidebarComponent`/
   `QueryEngine`, chỉ phụ thuộc `tradar-core` (và `tradar-connector-api` sau
   khi crate đó tồn tại).
3. **Đưa `Connector`/`Session`/`ConnectorDescriptor`/`Capability` vào**
   `tradar-connector-api` (không phải `tradar-core` — xem quyết định ở trên);
   cho `QueryEngine` implement `Session`. Đây là bước mà `Action` thực sự
   thu hẹp còn enum đóng 5 variant (`Quit`/`OpenRequested`/`Opened`/
   `OpenFailed`/`BackToPicker`) và chuyển vào `tradar-core`, còn mọi variant
   `Action` hiện tại khác (`SchemaMove*`, `ResultsMove*`, `Yank`,
   `ExportCurl`, `SubmitQuery`, `QueryCompleted`, ...) chuyển thành một kiểu
   message riêng bên trong `tradar-query-workbench`, theo quy tắc "Screen
   never does IO" trong `docs/architecture.md`.
4. **Migrate từng driver một** vào `connectors/` (postgres → sqlite →
   mongo → elasticsearch → redis), mỗi cái thành một crate riêng implement
   `Connector`.

Chưa quyết định: bắt đầu bước 2 luôn ở session tới, hay review ranh giới
module `tradar-query-workbench` cụ thể trước. Hỏi lại user, đừng tự giả định.

## Đừng quên

- Tiếp tục dùng `cargo fmt --all` + `cargo clippy --all-targets --workspace -- -D warnings` +
  `cargo test --workspace --lib -- --skip drivers::postgres::tests::connect_succeeds_for_a_running_postgres --skip drivers::postgres::tests::list_schema_returns_created_tables --skip drivers::postgres::tests::execute_returns_columns_and_rows_for_a_select --skip drivers::mongo --skip drivers::redis --skip drivers::elasticsearch`
  (hoặc chỉ `--skip drivers::postgres` nếu Docker chưa chạy) trước khi coi
  bất kỳ việc gì là xong. Lưu ý các lệnh này giờ cần `--workspace` vì repo
  đã là Cargo workspace hai crate (`tradar-core`, `tradar-app`).
- File `~/.config/tradar/connections.toml` thật của user có 5 connection,
  không backend nào ngoài sqlite (postgres/elasticsearch/redis/mongo) thực
  sự chạy local — chỉ `local sqlite` connect thành công để test tay. Đừng
  giả định các cái khác reachable.
- Chỉ kill/động vào các tmux session mà chính agent này tạo ra. Một session
  trước đã lỡ kill session tmux `metap` của chính user trong lúc dọn debug
  session — đừng lặp lại.
