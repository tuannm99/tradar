# Completion sau `keyspace.`/`schema.` cho Cassandra (2026-10-11)

## Bối cảnh

Mục còn lại duy nhất rẻ trong checklist "Server headless + plugin Neovim" của `docs/roadmap.md` sau khi "Mục nhỏ" đã xong hết — người dùng chọn làm trước Cassandra row-edit (đắt hơn, cần driver nhớ kiểu cột) và gRPC (cần chốt scope trước).

Ghi nhận gốc trong roadmap: "Cassandra: tên bảng là `demo.users` nhưng từ đang gõ chỉ là `us`" — gõ `FROM demo.us` (sau khi đã gõ `demo.` để chọn keyspace) không gợi ý ra `users`.

## Vấn đề cụ thể

`SchemaInfo::name` của Cassandra luôn là `<keyspace>.<table>` (không như Postgres, chỉ gắn tiền tố khi hai schema trùng tên bảng qua `qualify_colliding_names` — xem `docs/architecture.md`). Candidate list (`CompletionSource::matches`) so khớp bằng `candidate.text_lower.starts_with(prefix)` — với `candidate.text == "demo.users"` và `prefix == "us"` (phần sau dấu `.` cuối cùng — cả `query_editor.rs`'s `word_before_cursor()` lẫn `tradar-server`'s `prefix_start` đều dừng ở ký tự không phải "từ", và `.` không tính là ký tự từ), `"demo.users".starts_with("us")` là `false` — không có gợi ý nào.

`completion_context()` (hàm quyết định ngữ cảnh completion từ text trước con trỏ) đã có logic đọc `<alias>.` — nhưng chỉ thử resolve `alias` như một table alias từ `FROM`/`JOIN` (`resolve_alias`). "demo" không phải alias của bảng nào (nó là keyspace), nên `resolve_alias` trả `None`, và trước bản sửa này `completion_context` theo đó trả `CompletionContext::None` luôn — rơi về flat list, vẫn so khớp sai như trên.

## Thay đổi

### `CompletionContext::SchemaQualifiedTable { schema }` (variant mới)

Khi `<alias>.` không resolve được qua `resolve_alias`, `completion_context()` giờ trả `SchemaQualifiedTable { schema: alias }` thay vì `None` — thử `alias` như một schema/keyspace name, không phải bỏ cuộc. Hàm này không có quyền truy cập schema thật đã connect (đúng ranh giới module: `query_driver.rs` chỉ đọc text, không biết gì về connection) nên không thể tự xác nhận "demo" có là keyspace thật hay không — phần đó là việc của `CompletionSource::matches_in_context`, nơi duy nhất có `self.schema: Vec<SchemaInfo>`.

### `matches_in_context`: lọc theo tiền tố, trả phần chưa qualify

Arm mới: lọc `self.schema` theo entry nào `name.split_once('.')` ra đúng `schema` (so không phân biệt hoa thường), rồi so phần còn lại (`table`) với prefix đang gõ. Candidate trả về chỉ là `table` (chưa qualify) — vì phần `demo.` người dùng đã gõ rồi, trả cả `demo.users` sẽ chèn thành `demo.demo.users` khi accept (cơ chế accept chỉ thay đúng "từ" đang gõ — `us` — bằng text của candidate, không thay cả dòng). Không có entry nào khớp thì rơi về `self.matches(prefix)` — đúng pattern "không mạnh hơn, rơi về flat list" `TableColumns`/`JoinTarget` đã dùng, không phải trường hợp mới.

### Tác dụng phụ (có lợi) cho Postgres

Vì lọc theo đúng hình dạng `name` có dấu `.` hay không (không phải case riêng cho Cassandra), Postgres cũng ăn được fix này miễn phí cho trường hợp hiếm hai schema cùng có bảng trùng tên — lúc đó `qualify_colliding_names` gắn `public.users`/`other.users`, gõ `other.us` trước bản sửa này cũng không gợi ý được, giờ hoạt động đúng.

## Test

Unit test mới trong `query_driver.rs`: `dot_after_a_schema_name_reports_it_for_schema_qualified_lookup` (cả có và không có text sau dấu `.`), `dot_after_a_resolvable_alias_still_wins_over_schema_qualified` (alias thật vẫn ưu tiên `TableColumns`, không bị `SchemaQualifiedTable` che mất). Một test cũ (`dot_on_an_unknown_alias_is_not_a_special_context`) đổi tên + assertion vì hành vi thay đổi có chủ đích — alias không resolve được giờ trả `SchemaQualifiedTable` thay vì `None`; một test Mongo cũ (`typing_the_collection_name_itself_is_not_the_method_context`, input `"db.ord"`) cũng đổi assertion tương tự — `"db"` gần như chắc chắn không khớp keyspace/schema thật nào nên hiệu ứng thực tế qua `matches_in_context`'s fallback giống hệt `None` cũ, chỉ khác giá trị enum.

Unit test mới trong `completion.rs`: fixture 3 entry kiểu Cassandra (`demo.users`, `demo.usage_log`, `other.users`) — gợi ý đúng tên chưa qualify trong đúng schema, không phân biệt hoa thường tên schema, rơi về flat list khi schema không khớp entry nào.

`cargo build --workspace`, `cargo test -p tradar-query-workbench` (641 pass), `cargo test -p tradar-app` (167 pass), `cargo clippy --all-targets --workspace -- -D warnings`, `cargo fmt --all -- --check` đều sạch.

## Chưa làm

- Không verify qua Cassandra thật (không cần Docker cho phần này — thuần unit test, logic không đụng driver cụ thể nào) nhưng cũng chưa chạy tay qua Neovim/TUI thật với một cluster Cassandra để xác nhận trải nghiệm gõ thật mượt.
- Cassandra: sửa/xoá dòng (driver cần nhớ kiểu cột để sinh literal CQL đúng) — mục còn lại kế tiếp trong checklist, đắt hơn mục này, chưa làm.
