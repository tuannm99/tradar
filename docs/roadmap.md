# Roadmap

Việc **đang mở/chưa scope xong** sống ở đây — ngắn, dễ quét. Việc **đã xong** nằm trong `docs/backlog/` (mục lục: `docs/backlog/README.md`). Thiết kế hệ thống ở `docs/architecture.md`. Cập nhật file này mỗi khi một mục bắt đầu/kết thúc hoặc có mục mới.

## Hướng đi hiện tại

**Giao diện chính là Neovim + `tradar-server`**, không phải TUI (`tradar-app` đóng băng, chỉ sửa lỗi). Xem `docs/architecture.md`.

## Thứ tự tiếp theo

1. **Dùng thật vài ngày trên database của chính mình** (người dùng làm, không phải việc code) — mọi lỗi gặp được vào đây trước tính năng mới.
2. ~~Quyết định cuối về TUI~~ — **đã chốt: giữ đóng băng** (2026-10-31). Chỉ sửa lỗi, vẫn phải build/test qua; không bỏ hẳn. Hệ quả: phần client trùng lặp giữa connector crate và `tradar-server` (mục bên dưới) chỉ làm khi một lỗi phải sửa ở cả hai bản.

**Cố ý để sau, chưa có nhu cầu**: đóng gói `tradar-server` vào Docker (cần transport TCP + token trước — bảo mật hiện dựa vào quyền `0600` của unix socket); Windows named pipe.

## Checklist tổng quan

Quét nhanh — chi tiết/lý do đầy đủ nằm trong `docs/backlog/<file>.md` qua link ở mỗi dòng.

**So sánh DataGrip/DBeaver/Studio3T**

- [x] Tier 1 — Sort theo cột — `docs/backlog/sort-by-column.md`
- [x] Tier 2 — Navigator schema/database + nhóm object — `docs/backlog/navigator-schema-level.md`
- [x] Tier 3 — Autocomplete ngữ cảnh sâu + ERD — `docs/backlog/fk-autocomplete-and-erd.md`
- [x] Tier 4 — Generate SQL từ UI — `docs/backlog/crud-snippet-column-picker.md`
- [x] Tier 4 — Multi-filter kết hợp — `docs/backlog/multi-filter.md`
- [x] Tier 5 — Schema diff/compare — `docs/backlog/schema-diff.md`
- [x] Tier 5 — Table designer — `docs/backlog/table-designer.md`
- [x] Tier 5 — Migration/version-control — `docs/backlog/migrations.md`
- [ ] Tier 6 — Group-by trong grid — bỏ qua (không hợp mục đích results grid dựa trên row thật)
- [x] Tier 6 — Mở rộng edit-cell/delete-row ngoài single-table-with-PK — `docs/backlog/no-pk-row-edit.md`

**Connector mới**

- [x] MySQL / MariaDB — `docs/backlog/mysql-connector-2026-10-01.md`
- [x] ClickHouse — `docs/backlog/clickhouse-connector-2026-09-29.md`
- [x] Kafka: mode Groups (lag theo consumer group) — `docs/backlog/kafka-groups-mode-2026-10-01.md`
- [x] Socket — `docs/backlog/socket-connector-2026-09-29.md`
- [x] gRPC (server/plugin, unary) — `docs/backlog/grpc-rpc-methods-2026-10-14.md`, `docs/backlog/grpc-nvim-plugin-2026-10-14.md`
- [x] gRPC: metadata + server-/client-streaming + bidi — `docs/backlog/grpc-streaming-metadata-2026-10-31.md`

**Server headless + plugin Neovim** — xem `docs/backlog/server-headless-nvim-2026-10-04.md`

- [x] Giai đoạn 1 + 2: server JSON-RPC, completion/diagnostics/cancel qua protocol, plugin Neovim (gắn connection theo file, async, navigator, schema hover/`gd`, sửa dòng, bảo vệ query nguy hiểm, `.tdb`) — xem backlog theo từng mục con, mục lục ở `docs/backlog/README.md`
- [x] Push-channel (thiết kế + Kafka/Socket tail) — `docs/backlog/push-channel-design-2026-10-14.md`, `docs/backlog/kafka-tail-server-2026-10-15.md`, `docs/backlog/socket-tail-server-2026-10-15.md`
- [x] RabbitMQ qua server — `docs/backlog/rabbitmq-server-2026-10-16.md`
- [x] HTTP qua server — `docs/backlog/http-server-2026-10-17.md`
- [x] Phiên bản hoá protocol (`version` method, cảnh báo không chặn) — `docs/backlog/protocol-versioning-2026-10-18.md`
- [x] Rà soát UI/UX plugin Neovim — `docs/backlog/nvim-ux-review-2026-10-15.md`
- [x] Test end-to-end thật cho Kafka tail (connect/topics/tail từ earliest + live/stop/dọn khi client ngắt kết nối, trên broker `apache/kafka` thật) — `docs/backlog/kafka-e2e-test-2026-10-20.md`; chạy bằng `make test-kafka-server`
- [x] Buffer kết quả: JSON view cho SQL, row view `gK`, sửa staged + commit — `docs/backlog/result-buffer-views-staged-edit-2026-10-20.md`
- [x] `:TradarConnections` + sửa Socket tail/tail buffer — `docs/backlog/nvim-connections-tail-fixes-2026-10-20.md`
- [x] Sửa trực tiếp trên lưới kết quả + INSERT, commit một lượt bằng `:w` — `docs/backlog/grid-edit-in-place-2026-10-22.md`
- [x] HTTP: buffer request sửa được — `docs/backlog/http-request-buffer-2026-10-21.md`
- [x] RabbitMQ publish buffer + refresh — `docs/backlog/rabbitmq-publish-buffer-2026-10-21.md`
- [x] Kafka tail: tạm dừng/lọc/xoá/nối lại — `docs/backlog/kafka-tail-controls-2026-10-21.md`
- [x] Quyết định cuối: **giữ đóng băng TUI** (không bỏ hẳn) — 2026-10-31
- [ ] Tách phần client dùng chung giữa connector crate (TUI) và module `tradar-server` — **chủ đích chưa làm** (rà 2026-10-19; TUI đã chốt giữ đóng băng 2026-10-31, nên vẫn chỉ làm khi một lỗi phải sửa ở cả hai bản). Đang bị chép: RabbitMQ `parse_target`/`RabbitTarget`/`request`/`PEEK_COUNT`/thân request peek; HTTP `parse_headers`/`resolve_url`; Kafka `ephemeral_group_id`/probe metadata/`list_topics`; Socket `hex_preview`/`chunk_text` và hằng số 500 của ring buffer. Chỉ là các hàm thuần và hằng số (vài trăm dòng, ít đổi) — phần khác nhau mới là phần lớn (TUI bọc trong `tick()`/event queue, server bọc trong notification push), nên không có `Session` hay `Extension` nào để gộp. Để server dùng chung thì chỉ có hai đường: phụ thuộc thẳng `tradar-connector-*` (kéo `ratatui`/UI TUI vào server) hoặc một crate chung mới (một crate cho cả bốn backend, không phải mỗi backend một crate). **Chỉ làm khi**: quyết định *giữ* TUI lâu dài, hoặc một lỗi phải sửa ở cả hai bản. Nếu quyết định *bỏ* TUI thì phần trùng lặp tự biến mất — tách trước sẽ là công cốc.
- [ ] Đóng gói: đường dẫn socket dài hơn ~108 byte, Windows named pipe

**`tradar` CLI: import/export** — chưa scope, tier thấp (gần như mỗi connector đã có tool OSS riêng làm đúng việc này — `mongoimport`/`psql \copy`/`cqlsh COPY`/`elasticdump`; giá trị thật của việc tự làm lại chưa rõ ràng). Khi quay lại cần chốt: subcommand `clap` hay binary riêng; tái dùng connection đã lưu; mapping cột/field theo từng connector; streaming cho file lớn; import-only hay cả export.

- [ ] Chốt các điểm mở rồi lên plan

**Gap nhỏ, đã xong** — xem backlog: theme preset (`docs/backlog/theme-presets.md`), remap vim trong editor (`docs/backlog/vim-remap-2026-09-24.md`), resize cột tay (`docs/backlog/column-resize-2026-09-24.md`), visual-mode search-as-motion (`docs/backlog/visual-mode-search-motion-2026-09-24.md`), `:s/pat/repl/` (`docs/backlog/command-line-substitute-2026-09-24.md`).
