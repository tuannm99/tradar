# Backlog — lịch sử sub-project (đã xong)

Mục lục cho các file trong thư mục này. Mỗi file là bản ghi đầy đủ một sub-project đã hoàn thành — quyết định thiết kế, đánh đổi, bug phát hiện lúc làm, test. Việc **đang mở/chưa scope xong** không nằm ở đây, xem `docs/roadmap.md`. Thiết kế hệ thống (không phải nhật ký) nằm ở `docs/architecture.md`.

Nhóm theo chủ đề, trong mỗi nhóm theo thứ tự thời gian. Mục mới thêm vào cuối nhóm của nó.

## Kiến trúc, server, protocol, test/CI

- [`roadmap-sub-project.md`](roadmap-sub-project.md) — migration Component Architecture, vim keybinding, sessions/tab, viết lại query editor, migration connector pluggable.
- [`known-issues.md`](known-issues.md) — bug từ review nhánh/báo cáo người dùng, đã fix hết.
- [`rename-connector-crates.md`](rename-connector-crates.md) — đổi tên + chuyển `crates/connectors/tradar-*` → `crates/tradar-connector-*`.
- [`rename-connector-api-to-spi.md`](rename-connector-api-to-spi.md) — đổi tên `tradar-connector-api` → `tradar-connector-spi`.
- [`connector-feature-flags-2026-10-01.md`](connector-feature-flags-2026-10-01.md) — mỗi connector thành một Cargo feature, `--no-default-features` build slim.
- [`server-headless-nvim-2026-10-04.md`](server-headless-nvim-2026-10-04.md) — giai đoạn 1 `tradar-server` (JSON-RPC/unix socket) + plugin Neovim.
- [`push-channel-design-2026-10-14.md`](push-channel-design-2026-10-14.md) — thiết kế kênh đẩy server→client cho Kafka/Socket tail (chưa code lúc viết).
- [`protocol-versioning-2026-10-18.md`](protocol-versioning-2026-10-18.md) — phiên bản hoá protocol JSON-RPC (`version` method, cảnh báo không chặn).
- [`server-extension-trait-2026-10-19.md`](server-extension-trait-2026-10-19.md) — tách 5 connector phi-`QueryDriver` khỏi `server.rs` bằng trait `Extension` (1426 → 658 dòng, `#[cfg]` 80 → 3), không đổi hành vi wire; kèm `Subscriptions<H, M>` dùng chung cho Kafka/Socket tail.
- [`kafka-e2e-test-2026-10-20.md`](kafka-e2e-test-2026-10-20.md) — test end-to-end thật cho Kafka tail qua server trên broker `apache/kafka` (connect/topics/tail/stop/dọn khi client ngắt kết nối), `make test-kafka-server`.
- [`stale-server-and-picker-fixes-2026-10-25.md`](stale-server-and-picker-fixes-2026-10-25.md) — phát hiện server chạy ngầm bản cũ (`version.stale`/`exe`, cảnh báo, health, `PATH` che bản build); picker tables/lịch sử trong `.tdb` dùng đúng connection và vị trí
- [`server-log-2026-10-21.md`](server-log-2026-10-21.md) — file log của `tradar-server`, `:TradarLog`, protocol 1.11; sửa socket đóng không dọn client/subscription
- [`idle-disconnect-2026-10-22.md`](idle-disconnect-2026-10-22.md) — connection không dùng quá `TRADAR_IDLE_TTL` thì server tự đóng, protocol 1.12
- [`nvim-tests-ci-2026-10-22.md`](nvim-tests-ci-2026-10-22.md) — `nvim/tests/` (spec headless + `make test-nvim`) và CI GitHub Actions
- [`split-tradar-query-2026-10-22.md`](split-tradar-query-2026-10-22.md) — crate `tradar-query`, `DriverOpener`, feature `ui`/`tui`; server không còn phụ thuộc TUI, schema tải một lần khi connect
- [`dedupe-non-query-connectors-2026-10-22.md`](dedupe-non-query-connectors-2026-10-22.md) — Kafka/RabbitMQ/HTTP/Socket: module dùng chung không UI + feature `tui`; server bỏ các bản chép
- [`unified-connection-lifecycle-2026-10-22.md`](unified-connection-lifecycle-2026-10-22.md) — `Extension::connected`/`close`: idle TTL và `disconnect` áp cho mọi loại connection; plugin tự reconnect `<driver>.*`; protocol 1.13
- [`performance-fetch-cap-2026-10-31.md`](performance-fetch-cap-2026-10-31.md) — `max_rows` (mặc định 1000) thay vì đọc 10 000 dòng, trần 64 MB mỗi kết quả, 256 MB tổng cursor, đóng cursor cũ: `select *` 600 ms → 30 ms, RSS không còn tăng không chặn

## Plugin Neovim và tính năng qua server (giao diện chính)

- [`nvim-schema-tree-2026-10-06.md`](nvim-schema-tree-2026-10-06.md) — panel schema plugin thành cây thật (`<leader>rs`).
- [`gd-fk-join-2026-10-07.md`](gd-fk-join-2026-10-07.md) — `gd` theo FK hoạt động trên kết quả `JOIN`.
- [`tdb-format-2026-10-09.md`](tdb-format-2026-10-09.md) — định dạng file `.tdb` (nhiều dialect/connection một file).
- [`schema-qualified-completion-2026-10-11.md`](schema-qualified-completion-2026-10-11.md) — completion sau `keyspace.`/`schema.`.
- [`grpc-rpc-methods-2026-10-14.md`](grpc-rpc-methods-2026-10-14.md) — nửa server gRPC: 6 method mới trong `grpc.rs`.
- [`grpc-nvim-plugin-2026-10-14.md`](grpc-nvim-plugin-2026-10-14.md) — nửa plugin gRPC: `nvim/lua/tradar/grpc.lua`.
- [`nvim-ux-review-2026-10-15.md`](nvim-ux-review-2026-10-15.md) — rà keymap/command plugin: `gT`→`<leader>rv`, `g?`, `K`, append tăng dần, cap cột, gRPC tab-reuse/huỷ.
- [`kafka-tail-server-2026-10-15.md`](kafka-tail-server-2026-10-15.md) — Kafka tail qua `tradar-server` (push-channel), `ClientConnection` mới.
- [`socket-tail-server-2026-10-15.md`](socket-tail-server-2026-10-15.md) — Socket tail qua server, cùng hạ tầng push-channel.
- [`rabbitmq-server-2026-10-16.md`](rabbitmq-server-2026-10-16.md) — RabbitMQ qua server (không cần push-channel).
- [`http-server-2026-10-17.md`](http-server-2026-10-17.md) — HTTP qua server (không cần push-channel).
- [`wiki-huong-dan-2026-10-19.md`](wiki-huong-dan-2026-10-19.md) — wiki hướng dẫn sử dụng trong `docs/wiki/` (cài đặt, kết nối, SQL, Mongo/Redis/ES, `.tdb`, Kafka/RabbitMQ/HTTP/Socket/gRPC, TUI, xử lý sự cố).
- [`result-buffer-views-staged-edit-2026-10-20.md`](result-buffer-views-staged-edit-2026-10-20.md) — buffer kết quả: JSON view cho SQL (`<leader>rv`), row view `gK`, `i`/`dd` xếp hàng rồi `<leader>rw` commit / `<leader>ru` bỏ
- [`nvim-connections-tail-fixes-2026-10-20.md`](nvim-connections-tail-fixes-2026-10-20.md) — `:TradarConnections` mở `connections.toml` (+ `connections.path`, protocol 1.1), sửa Socket tail không gõ được, tail buffer không giật con trỏ/không phình vô hạn; rà HTTP/RabbitMQ/Kafka còn lại
- [`http-request-buffer-2026-10-21.md`](http-request-buffer-2026-10-21.md) — `:TradarHttpSend` mở buffer request sửa được (`METHOD url` / headers / body nhiều dòng) thay chuỗi `vim.ui.input`
- [`rabbitmq-publish-buffer-2026-10-21.md`](rabbitmq-publish-buffer-2026-10-21.md) — RabbitMQ publish bằng buffer sửa được (payload nhiều dòng), `r` làm mới split peek/bindings
- [`kafka-tail-controls-2026-10-21.md`](kafka-tail-controls-2026-10-21.md) — Kafka tail: `p` tạm dừng, `f` lọc, `c` xoá, tự nối lại khi mất kết nối server (`R` thử lại), winbar trạng thái
- [`grid-edit-in-place-2026-10-22.md`](grid-edit-in-place-2026-10-22.md) — sửa thẳng trên lưới kết quả bằng lệnh vim, `:w` → xem trước `UPDATE`/`DELETE`/`INSERT` → commit một lượt (khớp dòng theo khoá; `edit.sql` insert, protocol 1.2)
- [`commit-transaction-json-view-fixes-2026-10-23.md`](commit-transaction-json-view-fixes-2026-10-23.md) — commit lưới trong một transaction (rollback khi lỗi), JSON view Mongo/ES pretty-print có thứ tự, sửa comment trong query Mongo và `rr` trên dòng fence của `.tdb`
- [`es-comments-tdb-filetype-2026-10-24.md`](es-comments-tdb-filetype-2026-10-24.md) — comment `#`/`//`/`/* */` trong body Elasticsearch; filetype `.tdb` không còn mất (ftdetect, đặt lại khi plugin load muộn, tự bật highlight markdown, nhận theo đuôi file)
- [`redis-quoting-statusline-2026-10-26.md`](redis-quoting-statusline-2026-10-26.md) — Redis nhận giá trị có dấu cách trong nháy (`SET k "a b"`); statusline đúng connection trong buffer kết quả, cờ sửa chưa commit, badge `tx` tức thì
- [`grpc-request-skeleton-2026-10-27.md`](grpc-request-skeleton-2026-10-27.md) — buffer request gRPC điền sẵn khung từ message đầu vào (`grpc.skeleton`, protocol 1.4)
- [`cancel-reconnect-fixes-2026-10-28.md`](cancel-reconnect-fixes-2026-10-28.md) — huỷ query chậm báo `cancelled` thay vì lỗi (`biased` select), plugin tự kết nối lại và lặp một lần khi server đã khởi động lại
- [`results-paging-progress-selection-2026-10-30.md`](results-paging-progress-selection-2026-10-30.md) — bỏ tự tải trang khi cuộn (`<leader>rm`, `page_size`), thanh tiến trình query bằng winbar (bớt lag statusline), Mongo khỏi `list_schema` sau mỗi kết quả, visual `<leader>rr` tách theo `;`
- [`grpc-streaming-metadata-2026-10-31.md`](grpc-streaming-metadata-2026-10-31.md) — gRPC: metadata request/response, server-/client-streaming và bidi qua `grpc.stream.*` (protocol 1.6), buffer transcript trong plugin
- [`results-window-layout-zoom-2026-10-31.md`](results-window-layout-zoom-2026-10-31.md) — `<leader>rz` phóng cửa sổ kết quả toàn màn hình, `results_layout` (bottom/right/tab), sửa cửa sổ kết quả bị bé 4 dòng sau "running…"
- [`ui-ux-perf-check-2026-10-20.md`](ui-ux-perf-check-2026-10-20.md) — rà UI/UX + hiệu năng plugin; sửa `list_schema` SQLite (N+1 `PRAGMA` → 3 truy vấn)
- [`save-query-library-2026-10-22.md`](save-query-library-2026-10-22.md) — `:TradarSave` (thư viện chung) / `:TradarSaveHere` (cạnh file) / `:TradarOpen`
- [`nvim-ux-batch-2026-10-22.md`](nvim-ux-batch-2026-10-22.md) — `zw`/`zW` hiện trọn cột, statusline `(idle)`, Tab-complete connection, thông báo một ngôn ngữ, `:Tradar <lệnh-con>`
- [`nvim-crud-generator-2026-10-22.md`](nvim-crud-generator-2026-10-22.md) — `:TradarCrud` / `<leader>rg`: sinh create/read/update/delete cho một bảng, chọn cột, chèn vào buffer
- [`small-fixes-socket-telescope-2026-10-22.md`](small-fixes-socket-telescope-2026-10-22.md) — lỗi rõ + đường dẫn mặc định ngắn khi socket quá dài; spec cho picker telescope (`deps.sh`)
- [`nvim-result-history-filter-diff-2026-10-24.md`](nvim-result-history-filter-diff-2026-10-24.md) — lịch sử lần chạy (`[R`/`]R`), ghim tab, lọc/sắp xếp (`rF`/`ro`), `:TradarDiff`, gợi ý giá trị `${}`
- [`nvim-result-tabs-2026-10-23.md`](nvim-result-tabs-2026-10-23.md) — tab kết quả theo bảng/collection/index/key, thanh tab click được, `<leader>r1..9`/`rx`
- [`nvim-ux-results-log-flash-2026-10-22.md`](nvim-ux-results-log-flash-2026-10-22.md) — (phần log kết quả đã được thay bằng tab theo đích) nháy câu vừa chạy
- [`nvim-params-refresh-addconn-2026-10-22.md`](nvim-params-refresh-addconn-2026-10-22.md) — tham số `${tên}`, làm mới/watch kết quả (`<leader>rR`, `:TradarWatch`), `:TradarAddConnection`

## Connector (dùng chung cho cả hai giao diện)

- [`http-connector.md`](http-connector.md) — connector HTTP kiểu Postman.
- [`mongo-no-default-db.md`](mongo-no-default-db.md) — MongoDB connect không cần chọn DB, lệnh `use`/`show dbs`.
- [`mongo-es-row-edit.md`](mongo-es-row-edit.md) — row-edit cho MongoDB/Elasticsearch.
- [`mongo-chaining.md`](mongo-chaining.md) — Mongo: chain `sort`/`limit`/`skip`/`count()`, `findOne`/`countDocuments`, projection.
- [`no-pk-row-edit.md`](no-pk-row-edit.md) — edit-cell/delete-row cho bảng không PK khai báo.
- [`clickhouse-connector-2026-09-29.md`](clickhouse-connector-2026-09-29.md) — connector ClickHouse qua HTTP interface.
- [`socket-connector-2026-09-29.md`](socket-connector-2026-09-29.md) — connector Socket (raw TCP netcat-style).
- [`sql-es-mongo-ux-2026-09-30.md`](sql-es-mongo-ux-2026-09-30.md) — Mongo completion theo collection, ES validate JSON cục bộ, Mongo index thật, SQLite error marker.
- [`mysql-connector-2026-10-01.md`](mysql-connector-2026-10-01.md) — connector MySQL/MariaDB.
- [`correctness-gaps-2026-10-03.md`](correctness-gaps-2026-10-03.md) — 6 gap correctness: HTTP target optional, table designer chặn 2 PK, Redis/Cassandra cap `MAX_ROWS`, migrations tên trùng, export JSON `NULL`→`null`.
- [`real-backend-fixes-2026-10-04.md`](real-backend-fixes-2026-10-04.md) — lỗi khi chạy plugin với backend thật (kiểu dữ liệu, `list_schema` MySQL, mongosh), thêm Mongo/Redis/ES cho plugin.
- [`mongo-field-order-2026-10-07.md`](mongo-field-order-2026-10-07.md) — cột tài liệu Mongo theo đúng thứ tự field thật.
- [`real-cancel-2026-10-08.md`](real-cancel-2026-10-08.md) — `cancel` huỷ thật phía database cho Postgres/SQLite.
- [`cassandra-row-edit-2026-10-12.md`](cassandra-row-edit-2026-10-12.md) — sửa/xoá dòng cho Cassandra.
- [`grpc-spike-2026-10-12.md`](grpc-spike-2026-10-12.md) — spike xác nhận khả thi dynamic-invocation gRPC.
- [`update-multi-cell-2026-10-29.md`](update-multi-cell-2026-10-29.md) — các ô đổi của một dòng gộp thành một câu `UPDATE` (`RowChange::SetValues`, protocol 1.5; SQL, Cassandra, Mongo, ES)
- [`mongo-console-helpers-2026-10-31.md`](mongo-console-helpers-2026-10-31.md) — Mongo: `show collections/users/roles`, `db`, `db.getCollectionNames()/stats()/runCommand()...`, `db.getCollection("x")`, dấu `;` cuối câu

## TUI `tradar-app` (deprecated)

- [`yeu-cau-2026-08-14.md`](yeu-cau-2026-08-14.md) — 8 yêu cầu sau dùng thử: non-SELECT, schema sidebar, giới hạn/huỷ query, chuột, autocomplete, navigator đa connection, bảng kết quả thao tác được.
- [`query-file-multi-statement.md`](query-file-multi-statement.md) — giữ nội dung query qua session, nhiều statement một file.
- [`quan-ly-file-query.md`](quan-ly-file-query.md) — thư mục queries, recent list, file picker.
- [`file-browser-save-open.md`](file-browser-save-open.md) — file picker thành trình duyệt thư mục thật.
- [`features-batch-2026-08-14.md`](features-batch-2026-08-14.md) — export CSV/JSON, search kết quả, trạng thái connection, CRUD snippet, snippet library, undo/redo + Visual mode, schema ES/Mongo sâu hơn, connect timeout.
- [`mockup-ui-2026-08-15.md`](mockup-ui-2026-08-15.md) — code folding, preview cell, transaction control, Redis key browser, connector Kafka/RabbitMQ/Cassandra, navigator filter, table⇄JSON.
- [`layout-zoom-mouse.md`](layout-zoom-mouse.md) — layout ngang/dọc + zoom, chuột trái/phải/giữa cho query/HTTP screen.
- [`tab-session-management.md`](tab-session-management.md) — quản lý tab/session, chuột đầy đủ.
- [`mongo-es-completion-autoclose-vimconfig.md`](mongo-es-completion-autoclose-vimconfig.md) — Mongo/ES completion mở rộng, bracket/quote auto-close, vim mode qua config.
- [`mouse-ux-polish.md`](mouse-ux-polish.md) — double-click dùng chung, filter connection picker, badge tab nền.
- [`keymap-and-performance-2026-08-18.md`](keymap-and-performance-2026-08-18.md) — rà keymap + hiệu năng (checkpoint batching, event loop, Mongo schema song song).
- [`sort-by-column.md`](sort-by-column.md) — sort theo cột, client-side.
- [`navigator-schema-level.md`](navigator-schema-level.md) — navigator thêm cấp schema/keyspace/database + nhóm theo loại object.
- [`fk-autocomplete-and-erd.md`](fk-autocomplete-and-erd.md) — `ColumnInfo.foreign_key`, autocomplete ngữ cảnh sâu, ERD box-drawing.
- [`crud-snippet-column-picker.md`](crud-snippet-column-picker.md) — CRUD snippet mở column picker trước khi insert.
- [`multi-filter.md`](multi-filter.md) — filter kết quả mở rộng `cột:giá_trị` + `AND`/`OR`, panel xem/xoá điều kiện.
- [`schema-diff.md`](schema-diff.md) — so schema 2 connection đã mở, dựa trên `Component::outline()`.
- [`table-designer.md`](table-designer.md) — thêm/xoá cột, đổi tên bảng, tạo bảng mới (chỉ Postgres).
- [`migrations.md`](migrations.md) — panel migration kiểu Flyway/Alembic, chỉ Postgres.
- [`theme-presets.md`](theme-presets.md) — preset Dracula/Nord.
- [`keymap-tab-nav-2026-09-24.md`](keymap-tab-nav-2026-09-24.md) — đổi default navigator/tab-nav key.
- [`json-view-line-scroll-2026-09-24.md`](json-view-line-scroll-2026-09-24.md) — JSON view cuộn từng dòng thay vì theo document.
- [`yank-curl-2026-09-24.md`](yank-curl-2026-09-24.md) — yank curl thẳng clipboard.
- [`column-resize-2026-09-24.md`](column-resize-2026-09-24.md) — `<`/`>` resize cột tay trong results grid.
- [`visual-mode-search-motion-2026-09-24.md`](visual-mode-search-motion-2026-09-24.md) — `/`/`n`/`N` như motion thật trong Visual mode.
- [`command-line-substitute-2026-09-24.md`](command-line-substitute-2026-09-24.md) — `:s/pat/repl/` trong query editor.
- [`vim-remap-2026-09-24.md`](vim-remap-2026-09-24.md) — phím vim bên trong editor remap được qua keymap.
- [`results-grid-memoization-2026-09-29.md`](results-grid-memoization-2026-09-29.md) — cache `visible_items()`/`json_lines()` trong results grid.
- [`kafka-rabbitmq-sidebar-filter-2026-09-30.md`](kafka-rabbitmq-sidebar-filter-2026-09-30.md) — `/`-filter cho sidebar Kafka/RabbitMQ.
- [`kafka-groups-mode-2026-10-01.md`](kafka-groups-mode-2026-10-01.md) — Kafka mode Groups (lag theo consumer group).
- [`connector-ux-gaps-2026-10-02.md`](connector-ux-gaps-2026-10-02.md) — 6 gap UX: highlighting ClickHouse/Cassandra, filter Redis/HTTP, error marker, chuột Kafka/RabbitMQ/Socket.
- [`vim-operations-gap-2026-10-02.md`](vim-operations-gap-2026-10-02.md) — word motion/operator/text object/`.` repeat trong query editor, `Focus`+scroll cho bảng lag Kafka/RabbitMQ.
- [`app-wide-ux-gaps-2026-10-02.md`](app-wide-ux-gaps-2026-10-02.md) — `--help`/`--version`, màu lỗi nhất quán, `/`-filter history/snippet, HTTP response search+wrap, scroll migration panel, Mongo `MAX_ROWS`+`truncated`.
