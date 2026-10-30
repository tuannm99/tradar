# Roadmap

Việc **đang mở/chưa scope xong** sống ở đây — ngắn, dễ quét. Việc **đã xong** nằm trong `docs/backlog/` (mục lục: `docs/backlog/README.md`). Thiết kế hệ thống ở `docs/architecture.md`. Cập nhật file này mỗi khi một mục bắt đầu/kết thúc hoặc có mục mới.

## Hướng đi hiện tại

**Giao diện chính là Neovim + `tradar-server`**, không phải TUI (`tradar-app` **deprecated**: không phát triển thêm, chỉ làm khi có người yêu cầu). Xem `docs/architecture.md`.

## Thứ tự tiếp theo

1. **Dùng thật vài ngày trên database của chính mình** (người dùng làm, không phải việc code) — mọi lỗi gặp được vào đây trước tính năng mới.
2. ~~Quyết định cuối về TUI~~ — **đã chốt: deprecated** (2026-10-31). Không phát triển thêm, không sửa chủ động (chỉ khi có người yêu cầu); vẫn nằm trong cây và phải build/test qua; không xoá.
3. **Kiểm chứng trên backend thật** những gì mới chỉ có spec SQLite hoặc stub: workbench gRPC qua một server gRPC thật (hiện `rpc.request` giả lập ở `restbench_spec`; phía server đã có test tonic thật), result tabs/lọc/diff/`rA` trên Postgres/MySQL/Mongo/ES/Redis, màu request trên terminal thật, blink.cmp thật (hiện chỉ gọi nguồn trực tiếp).
4. **Mở rộng đã đề xuất, chưa làm** (làm khi có nhu cầu): lọc/sắp xếp phía server cho Mongo và ES (như `rA` của SQL); thêm dòng mới ngay trong lưới Mongo; chạy cheatsheet trên nhiều version DB trong CI (entry cần version tối thiểu: Postgres 12+, Redis 6+); proxy theo từng connection (`?proxy=`/`?noproxy=1`, hiện chỉ biến môi trường của server, gRPC chưa đi qua proxy); cheatsheet riêng của người dùng cho HTTP/gRPC; lọc request/response phía server cho body rất lớn; biến request theo connection trong `connections.toml` (hiện `setup{ request_vars }` + `@x` trong file).

**Cố ý để sau, chưa có nhu cầu**: đóng gói `tradar-server` vào Docker (cần transport TCP + token trước — bảo mật hiện dựa vào quyền `0600` của unix socket); Windows named pipe.

**Completion trong Neovim — ý tưởng chưa scope** (ghi 2026-10-30, từ việc `.tdb` không ra gợi ý khi connection không với tới được; chưa việc nào được code):
- **Báo lỗi kết nối khi completion**: `complete.lua` (`ctx.ensure_connected(conn, go, function() cb({}) end)`) nuốt lỗi nên DB sập chỉ thấy "không có gợi ý". Hiện một cảnh báo ngắn, một lần cho mỗi connection (có thể dùng `notify`), không làm gián đoạn lúc gõ.
- **Keyword tĩnh khi chưa kết nối**: gợi ý `SELECT`/`FROM`/`WHERE`… theo dialect của block khi chưa có schema. Hiện toàn bộ gợi ý (bảng, cột, `alias.`, JOIN theo FK) lấy từ schema của connection đang mở, nên chưa kết nối = rỗng. Cần `nvim/tests/*_spec.lua` cho hành vi này, và cập nhật `docs/wiki/Truy-van-SQL.md` (mục Completion).

**Mongo/Elasticsearch — ý tưởng chưa scope** (chuyển từ `docs/next-run.md` cũ, ghi 2026-09-10, chưa việc nào được code; làm khi có nhu cầu thật, mỗi mục cần hỏi lại chủ dự án trước khi code):
- **ES `GET <index>/_doc/<id>` tách thành 1 row có `_id`** (rủi ro thấp nhất): response này vẫn hiện nguyên envelope `{_index,_id,_version,found,_source}` trong khi `_search` đã được `unwrap_search_hits` (`crates/tradar-connector-elasticsearch/src/response.rs`) tách thành từng hit. Thêm hàm tương tự (gộp `_id` + field của `_source`; `found:false` → 0 row, không lỗi), gọi trong `execute()` cạnh `unwrap_search_hits`, nhận diện bằng body (`found` + `_source` ở top-level, không có `hits`).
- **Row-edit cho `GET <index>/_doc/<id>`** — `edit_source` chỉ nhận đúng 1 index tường minh; shape này chỉ có 1 document nên không lo multi-index, tách index + id thẳng từ path. **Row-edit cho multi-index/wildcard `_search`** thì khó hơn hẳn: phải mang `_index` của từng hit vào row và đổi `RowEdit`/`edit_sql` (dùng chung với SQL/Mongo) đọc `_index` từ row thay vì giả định 1 index cố định. Lưu ý row-edit hiện là tính năng của TUI đã deprecated — cân nhắc lại có đáng làm không trước khi đụng tới.
- **Mongo `bulkWrite`** — mongosh có, ta chưa hỗ trợ. Đường vào cụ thể: thêm 1 nhánh `"bulkWrite"` cạnh `insertOne`/`updateMany` trong `run_method` (`crates/tradar-connector-mongo/src/run.rs`), parse mảng operation (mỗi phần tử đúng 1 key `insertOne`/`updateOne`/`deleteOne`/...) thành `WriteModel` rồi gọi `Collection::bulk_write` thật (không tự lặp từng op — mất tính "1 write" của MongoDB); `reject_chain()` như các method khác; không editable. **Câu cần hỏi trước khi code**: có hỗ trợ option `ordered: false` ngay không, hay giữ mặc định `ordered: true` (dừng ở lỗi đầu tiên).
- **Elasticsearch**: chưa có auth (Basic/API key) hay TLS client-cert cho cluster cần bảo mật; `_bulk` chỉ forward nguyên văn, không có trợ giúp gì thêm; `_msearch` (`{responses:[...]}`) cần logic tách riêng. `_count` không có `hits` nên không có gì để tách row — đúng, không phải thiếu sót.
- **Mongo**: `startSession`/transaction vẫn cố tình ngoài phạm vi (`docs/architecture.md`); `aggregate(...)` chưa nhận chain (`.explain()` có thể hữu ích).

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
- [x] Cheatsheet query mẫu theo driver, xem bằng telescope (`rk`) — `docs/backlog/nvim-cheatsheet-2026-10-26.md`
- [x] Menu sinh câu lệnh theo driver (Mongo thêm find by _id / count / aggregate…), picker connection bằng telescope — `docs/backlog/nvim-snippet-menu-connection-picker-2026-10-26.md`
- [x] Lịch sử lần chạy + ghim tab, lọc/sắp xếp dòng đã tải, `:TradarDiff` — `docs/backlog/nvim-result-history-filter-diff-2026-10-24.md`
- [x] Tab kết quả theo bảng/collection/index/key (thanh tab, click, `r1..9`/`rx`) — `docs/backlog/nvim-result-tabs-2026-10-23.md`
- [x] HTTP + gRPC: workbench request chung (`###`, biến, response vào result tabs) — `docs/backlog/nvim-request-workbench-2026-10-29.md`
- [x] HTTP/gRPC: request viết trong `.tdb`/file `.http` (alias connection hoặc URL thẳng, protocol 1.16), tô màu, gợi ý (omnifunc + blink), xuất curl/grpcurl, cheatsheet block mẫu, lọc body — `docs/backlog/nvim-request-workbench-2026-10-29.md`
- [x] Không kiểm tra chứng chỉ TLS (`?insecure=1` cho HTTP/gRPC, `@insecure = 1`, protocol 1.17) và `setup{ server_env }` (biến môi trường/proxy cho server do plugin khởi động) — cùng backlog
- [x] Cheatsheet: xem cả bảng trong một buffer `.tdb` (`rK`, gập được, block chạy được) — `docs/backlog/nvim-cheatsheet-overview-2026-10-30.md`
- [x] `rA` hỏi server lọc/sắp xếp cho mọi dòng (SQL), `<C-d>` ngắt kết nối trong picker — `docs/backlog/nvim-server-view-disconnect-2026-10-28.md`
- [x] Màu view JSON bằng extmark (hết mất màu theo dữ liệu) — `docs/backlog/nvim-json-colours-2026-10-28.md`
- [x] Test cheatsheet chạy từng câu mẫu trên container thật (7 driver) — `docs/backlog/nvim-cheatsheet-2026-10-26.md`
- [x] HTTP: buffer request sửa được — `docs/backlog/http-request-buffer-2026-10-21.md`
- [x] RabbitMQ publish buffer + refresh — `docs/backlog/rabbitmq-publish-buffer-2026-10-21.md`
- [x] Kafka tail: tạm dừng/lọc/xoá/nối lại — `docs/backlog/kafka-tail-controls-2026-10-21.md`
- [x] Quyết định cuối: **TUI deprecated** (không xoá, chỉ làm khi có người yêu cầu) — 2026-10-31
- [x] Tách phần client dùng chung giữa connector crate và module `tradar-server` — `docs/backlog/dedupe-non-query-connectors-2026-10-22.md` (bước 3 review kiến trúc 2026-10-22)
- [x] Đường dẫn socket quá dài — `docs/backlog/small-fixes-socket-telescope-2026-10-22.md`
- [ ] Đóng gói: Windows named pipe

**Review kiến trúc 2026-10-22** — server không còn phụ thuộc TUI, một vòng đời connection chung

- [x] Bước 1 — test plugin Neovim trong repo + CI — `docs/backlog/nvim-tests-ci-2026-10-22.md`
- [x] Bước 2 — crate `tradar-query`, server bỏ phụ thuộc TUI, schema tải một lần — `docs/backlog/split-tradar-query-2026-10-22.md`
- [x] Bước 3 — gom logic trùng Kafka/RabbitMQ/HTTP/Socket — `docs/backlog/dedupe-non-query-connectors-2026-10-22.md`
- [x] Bước 4 — idle TTL/`disconnect` cho mọi loại connection — `docs/backlog/unified-connection-lifecycle-2026-10-22.md`
- [x] CI chạy test Docker của từng connector (job `docker` trong `.github/workflows/ci.yml`) — `docs/backlog/nvim-tests-ci-2026-10-22.md`
- [ ] TTL riêng theo từng connection (field trong `connections.toml`) — chỉ khi có nhu cầu; hiện TTL chung toàn server
- [x] Kết quả nhiều cột: `zw`/`zW` hiện trọn cột — `docs/backlog/nvim-ux-batch-2026-10-22.md`

**`tradar` CLI: import/export** — chưa scope, tier thấp (gần như mỗi connector đã có tool OSS riêng làm đúng việc này — `mongoimport`/`psql \copy`/`cqlsh COPY`/`elasticdump`; giá trị thật của việc tự làm lại chưa rõ ràng). Khi quay lại cần chốt: subcommand `clap` hay binary riêng; tái dùng connection đã lưu; mapping cột/field theo từng connector; streaming cho file lớn; import-only hay cả export.

- [ ] Chốt các điểm mở rồi lên plan

**Gap nhỏ, đã xong** — xem backlog: theme preset (`docs/backlog/theme-presets.md`), remap vim trong editor (`docs/backlog/vim-remap-2026-09-24.md`), resize cột tay (`docs/backlog/column-resize-2026-09-24.md`), visual-mode search-as-motion (`docs/backlog/visual-mode-search-motion-2026-09-24.md`), `:s/pat/repl/` (`docs/backlog/command-line-substitute-2026-09-24.md`).
