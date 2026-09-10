# Next run

Ghi chú bàn giao để tiếp tục công việc ở một session mới. File này là state
tạm/session, không phải bản ghi bền vững — `docs/roadmap.md` (việc đang mở)
và `docs/backlog/` (việc đã xong) mới là docs bền vững; cập nhật hai chỗ đó
và xoá hoặc làm rỗng file này khi nội dung đã cũ hoặc đã được xử lý.

## Trạng thái hiện tại (tính đến 2026-09-10, cuối phiên)

- Working tree sạch, đã commit/push. Nhánh reset từ `master` sau khi PR #2
  (multi-filter + row-edit Mongo/ES) merge, theo đúng quy tắc branch-đã-merge.
- Mongo: chain `sort`/`limit`/`skip`/`count()` sau `find()`, lệnh mới
  `findOne`/`countDocuments`, `find()` nhận projection (đối số thứ 2) —
  `docs/backlog/mongo-chaining.md`. Phát hiện + sửa luôn 1 bug có sẵn
  không liên quan chaining: `split_top_level_args` không bỏ qua nội dung
  chuỗi JSON khi đếm độ sâu ngoặc (giá trị string chứa `}`/`,` làm sai
  parse). `edit_source` (row-edit) vẫn nhận `find()` có chain
  sort/limit/skip, từ chối khi có `count()`.
- `make check` sạch. Test pure-logic (parser, `run_method` validate-trước-khi-chạm-mạng)
  chạy thật, không cần Docker — kể cả các case trước đây phải cần Mongo
  thật mới verify được (lợi dụng: `Client::with_uri_str` không tự connect,
  nên build `Collection` handle vào cổng không nghe được vẫn an toàn nếu
  test không bao giờ `.await` một operation thật).
- **Vẫn chưa verify tay qua tmux + `make test-docker`** cho cả row-edit
  Mongo/ES (từ phiên trước) lẫn chaining/findOne/countDocuments/projection
  (phiên này) — sandbox chưa từng có Docker daemon ở bất kỳ session nào
  tới giờ.

## Việc tiếp theo

Không có việc cụ thể nào đang dở — session trước hết quota nên dừng lại ở
mức **planning**, chưa code. User đang ưu tiên nhóm Mongo/Elasticsearch,
2 hướng đã nêu nhưng chưa chọn hẳn; phác thảo sẵn từng hướng dưới đây để
session sau (hoặc chính mình tuần sau) bắt tay vào được ngay, không phải
suy nghĩ lại từ đầu — **vẫn phải hỏi user chọn hướng nào trước khi code**,
đây chỉ là chuẩn bị sẵn phương án, không phải đã chốt.

### Hướng A: Elasticsearch ngoài `_search` (dễ hơn, chia nhỏ được)

Hiện trạng: `unwrap_search_hits` (thêm ở PR #2) tách `_search` thành nhiều
row bất kể 1 hay nhiều index bị query — tức **browse** đã ổn cả với
multi-index/wildcard rồi. Chỉ riêng **row-edit** (`edit_source`) cố tình
bảo thủ, chỉ nhận đúng 1 index tên tường minh.

- **A1 — `GET <index>/_doc/<id>` cũng tách thành 1 row có `_id`** (rủi ro
  thấp nhất, làm trước): hiện response này vẫn là 1 "document" nguyên envelope
  (`{_index, _id, _version, found, _source}`), khác hẳn cách `_search` hiện
  hiển thị. Thêm một hàm `unwrap_get_doc` tương tự `unwrap_search_hits`
  (gộp `_id` + field của `_source`), gọi trong `execute()` bên cạnh
  `unwrap_search_hits`.
- **A2 — Row-edit cho shape đó**: `edit_source` nhận thêm `GET
  <index>/_doc/<id>` (path shape `<index>/_doc/<literal id>`, không phải
  `_search`) — vì chỉ có đúng 1 document, không cần lo multi-index. Có thể
  gộp cùng `edit_sql` hiện có (đã dùng `_update`/`DELETE _doc/<id>` sẵn).
- **A3 — Row-edit cho multi-index/wildcard `_search`** (rủi ro cao hơn,
  để sau nếu có nhu cầu): cần mang `_index` của từng hit vào row (thêm
  field `_index` khi unwrap, tương tự `_id`), rồi đổi `RowEdit`/`edit_sql`
  đọc `_index` từ chính row thay vì giả định `edit.table` là 1 index cố
  định cho cả kết quả — đổi kiến trúc chung `RowEdit` (dùng chung với SQL/
  Mongo), cần cân nhắc kỹ trước khi làm, không nhẹ như A1/A2.
- Không đáng làm ngay (ghi lại để không quên): `_count` không có `hits`
  nên không có gì để tách row (đúng, không phải thiếu sót); `_msearch`
  hình dạng response khác hẳn (`{responses: [...]}`), cần logic tách riêng
  nếu có nhu cầu cụ thể.

### Hướng B: Rà lại toàn bộ 2 connector tìm bất cập khác

Việc research/audit trước, không phải code ngay — mỗi gạch đầu dòng dưới
đây là một *ứng viên*, cần xác nhận với user có đáng làm không trước khi
chốt phạm vi:

- **Mongo**: `bulkWrite` (nhiều write khác loại trong 1 lệnh, mongosh thật
  có) chưa hỗ trợ; transaction/session (`startSession`) vẫn cố tình ngoài
  phạm vi (đã ghi trong `docs/architecture.md`, không đổi trừ khi user yêu
  cầu); `aggregate(...)` chưa nhận chain (`.toArray()` là no-op thật ra
  không cần, nhưng `.explain()` thì có thể có ích).
- **Elasticsearch**: không có auth (Basic/API key) hay TLS client-cert cho
  cluster cần bảo mật; mỗi lần chạy đúng 1 request, không có `_bulk` được
  xử lý đặc biệt (vẫn forward nguyên văn được, chỉ là không có trợ giúp gì
  thêm); `_msearch` như trên.
- Rà thêm nếu có thời gian: `docs/backlog/known-issues.md`/`docs/roadmap.md`
  có mục nào liên quan Mongo/ES bị bỏ sót không, đọc lại toàn bộ
  `crates/tradar-connector-mongo`/`crates/tradar-connector-elasticsearch`
  một lượt tìm `// TODO`/comment ghi rõ giới hạn chưa làm.

### Ngoài nhóm Mongo/ES

`docs/roadmap.md`'s checklist tổng quan, Tier 5 (lớn, chưa scope) — #2
Table designer → #3 Schema diff/compare → #4 Migration/version-control —
mỗi mục cần `AskUserQuestion` riêng trước khi code.

## Đừng quên

- `cargo fmt --all` + `cargo clippy --all-targets --workspace -- -D
  warnings` (hoặc `make check`) trước khi coi bất kỳ việc gì là xong.
- `make test-unit` bỏ qua 8 connector cần Docker (Postgres/Redis/Mongo/
  Elasticsearch/Cassandra/RabbitMQ/Kafka/HTTP); `make test-docker` chạy
  riêng chúng nếu cần verify — môi trường sandbox này không có Docker
  daemon, chưa thử được `test-docker` ở session nào tới giờ.
- Mẹo verify logic Mongo/ES không cần Docker khi có thể: nếu một nhánh lỗi
  trả về trước khi `.await` một operation mạng thật, `mongodb::Client::with_uri_str`
  vào một địa chỉ không ai lắng nghe vẫn xây được `Collection` handle an
  toàn (driver connect lười) — xem `unreachable_collection()` trong
  `crates/tradar-connector-mongo/src/lib.rs`'s test module làm ví dụ.
- Container sandbox này thiếu sẵn `libcurl4-openssl-dev` (Kafka connector
  cần để build `librdkafka` qua cmake) — `apt-get install -y
  libcurl4-openssl-dev` trước khi build nếu gặp lỗi `curl/curl.h: No such
  file or directory`.
