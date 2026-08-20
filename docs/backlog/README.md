# Backlog — lịch sử sub-project (đã xong)

Mục lục cho các file trong thư mục này, theo thứ tự thời gian. Mỗi file là bản ghi đầy đủ một sub-project đã hoàn thành — quyết định thiết kế, đánh đổi, bug phát hiện lúc làm, test. Việc **đang mở/chưa scope xong** không nằm ở đây, xem `docs/roadmap.md`. Thư mục này thay thế `docs/backlog.md` cũ (tách ra 2026-08-19 khi file đó dài quá 450 dòng); nội dung từng file gần như nguyên văn, chỉ những mục còn mở mới được chuyển sang `docs/roadmap.md`.

- [`roadmap-sub-project.md`](roadmap-sub-project.md) — roadmap gốc: migration Component Architecture, vim keybinding xuyên suốt, lưu/mở query, sessions/tab, dedupe keymap, viết lại query editor (bỏ `edtui`), theme/keymap tuỳ biến, migration connector pluggable.
- [`yeu-cau-2026-08-14.md`](yeu-cau-2026-08-14.md) — 8 yêu cầu sau khi dùng thử: phản hồi non-SELECT, schema sidebar xem cột, giới hạn dòng/huỷ query, hỗ trợ chuột, autocomplete, navigator nhiều connection, bảng kết quả thao tác được, rà lại keymap.
- [`query-file-multi-statement.md`](query-file-multi-statement.md) — giữ nội dung query qua session, chạy nhiều câu lệnh trong một file.
- [`quan-ly-file-query.md`](quan-ly-file-query.md) — thư mục queries mặc định, recent list, file picker qua `Ctrl+O`.
- [`file-browser-save-open.md`](file-browser-save-open.md) — `FilePickerComponent` thành trình duyệt thư mục thật.
- [`features-batch-2026-08-14.md`](features-batch-2026-08-14.md) — loạt tính năng gom từ 2026-08-14: export CSV/JSON, tìm kiếm trong kết quả, trạng thái connection, CRUD snippet tự sinh, thư viện snippet, undo/redo + Visual mode + search trong editor, schema Elasticsearch/Mongo sâu hơn, connect timeout.
- [`mockup-ui-2026-08-15.md`](mockup-ui-2026-08-15.md) — mở rộng theo mockup "Tradar TUI": code folding, preview cell, transaction control, Redis key browser, connector Kafka + RabbitMQ + Cassandra, navigator filter, kiểu cột, spinner, lỗi Postgres parse được, table⇄JSON.
- [`known-issues.md`](known-issues.md) — các bug/vấn đề từ review nhánh và báo cáo người dùng, tất cả đã fix (connect treo, race connect, lag chuột, redraw thừa, Postgres/Mongo hiện sai kiểu dữ liệu...).
- [`http-connector.md`](http-connector.md) — connector HTTP kiểu Postman (form Method/URL/Headers/Body + response pane tự vẽ).
- [`rename-connector-crates.md`](rename-connector-crates.md) — đổi tên + di chuyển `crates/connectors/tradar-*` → `crates/tradar-connector-*`.
- [`rename-connector-api-to-spi.md`](rename-connector-api-to-spi.md) — đổi tên `tradar-connector-api` → `tradar-connector-spi`.
- [`layout-zoom-mouse.md`](layout-zoom-mouse.md) — layout ngang/dọc + zoom cho query/HTTP screen, chuột trái/phải/giữa.
- [`mongo-no-default-db.md`](mongo-no-default-db.md) — MongoDB connect không cần chọn DB, lệnh `use`/`show dbs`.
- [`tab-session-management.md`](tab-session-management.md) — quản lý tab/session cho 1 connection qua connection picker, đầy đủ chuột (double-click, chuột phải, badge).
- [`mongo-es-completion-autoclose-vimconfig.md`](mongo-es-completion-autoclose-vimconfig.md) — Mongo/ES keyword completion mở rộng, bracket/quote auto-close, vim mode/normal mode chọn qua config.
- [`mouse-ux-polish.md`](mouse-ux-polish.md) — `DoubleClickTracker` dùng chung, double-click cho navigator/history/snippet picker, filter cho connection picker, badge tab nền.
- [`keymap-and-performance-2026-08-18.md`](keymap-and-performance-2026-08-18.md) — rà toàn bộ keymap (fix bug Ctrl+Left/Right HTTP chết, đổi Ctrl+G → F2) + hiệu năng (checkpoint batching, event loop không-blocking, Mongo schema song song).
- [`sort-by-column.md`](sort-by-column.md) — sort theo cột trong bảng kết quả (phím `s` + click header), client-side, cycle asc/desc/off, giá trị thiếu luôn xếp cuối.
- [`navigator-schema-level.md`](navigator-schema-level.md) — navigator thêm cấp schema/keyspace/database (Postgres/Cassandra/MongoDB) + nhóm theo loại object (Postgres: Tables/Views/Functions/Procedures), `OutlineEntry::is_object` thay `depth == 0` cho CRUD snippet.
- [`fk-autocomplete-and-erd.md`](fk-autocomplete-and-erd.md) — dữ liệu FK mới (`ColumnInfo.foreign_key`, Postgres/SQLite), autocomplete ngữ cảnh sâu (`.` sau alias, JOIN gợi ý bảng liên quan), ERD box-drawing (`F4`) cho lân cận 1 bảng.

Thiết kế hệ thống (không phải nhật ký) nằm ở `docs/architecture.md`.
