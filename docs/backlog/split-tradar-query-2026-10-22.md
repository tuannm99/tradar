# Tách `tradar-query`: server không còn đi qua TUI (2026-10-22)

Bước 2 của review kiến trúc 2026-10-22.

**Trước:** `QueryDriver` nằm trong `tradar-query-workbench` cùng các component TUI; `tradar-core` luôn kéo ratatui/crossterm/arboard. `tradar-server` mở connection bằng `Connector::connect` của TUI — connector dựng một `QueryEngine` (Session của TUI) rồi server ép kiểu ngược `session.as_any().downcast_ref::<QueryEngine>()` để lấy driver; schema bị tải **hai lần** (một cho `QueryEngine`, một cho completion của server).

**Sau:**
- Crate mới `tradar-query`: toàn bộ `query_driver` cũ (`git mv`, giữ lịch sử) + `CrudOp` (chuyển từ `tradar-core::action`, core re-export) + phần không-UI của completion (`Candidate`, `CandidateKind`, `CompletionSource` — popup vẫn ở workbench) + `with_connect_timeout`/`CONNECT_TIMEOUT` (spi re-export) + trait mới `DriverOpener { id(), open(target) }`. Không phụ thuộc crate nào trong workspace.
- `tradar-query-workbench`: `pub use tradar_query as query_driver;` — code TUI đọc như cũ.
- `tradar-core`: feature `ui` (mặc định) bao `action`, `config`, `keymap`, `theme`, `ui`, `vim_list`; `storage`/`capability` luôn có.
- 8 connector query: `opener()` luôn có; `Connector` + `DESCRIPTOR` + `connector()` chuyển sang `tui.rs` sau feature `tui` (mặc định). Test của connector vẫn chạy đường TUI (`use tui::*` khi `cfg(test)`).
- `tradar-server`: `tradar-core` không `ui`, connector không `tui`, bỏ hẳn `tradar-connector-spi`/`tradar-query-workbench`. `registry()` trả `DriverOpener`; `connect` = `open` + `list_schema` một lần.

Kết quả đo: `cargo tree -p tradar-server -e normal` không còn crate TUI nào (trước: ratatui, crossterm, arboard, tree-sitter, workbench, spi), số crate phụ thuộc 531 → 476. `make check` + `make test-nvim` (9/9) đạt. Test cần Docker của từng connector chưa chạy được ở sandbox (đã biên dịch qua `clippy --all-targets`).

Lưu ý feature unification: `cargo build --workspace` vẫn bật `tui` cho connector (vì `tradar-app` cần), nên muốn binary server "sạch" thì build riêng `cargo build -p tradar-server`.
