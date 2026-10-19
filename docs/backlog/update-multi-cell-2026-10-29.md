# Gộp các ô đổi của một dòng thành một câu `UPDATE`

Lưới sửa trực tiếp (`grid-edit-in-place-2026-10-22.md`) sinh một `UPDATE` cho **mỗi ô** đổi. Nay mỗi dòng đổi một câu.

- `RowChange::SetValues(Vec<(column, value)>)` (thêm cạnh `SetValue`, giữ nguyên cho TUI). Mỗi nơi khớp enum được thêm nhánh: `build_sql_edit` (`UPDATE t SET a = …, b = … WHERE …`, dùng chung Postgres/MySQL/SQLite), Cassandra (CQL literal), Mongo (`$set: {a: …, b: …}`), Elasticsearch (một `doc` duy nhất; `merge_json` gộp các đường dẫn chấm cùng cha, vd. `customer.name` + `customer.vip` vào chung object `customer`).
- Protocol `1.5`: `edit.sql` nhận `change.set` là một `{column, value}` (như cũ) hoặc danh sách; danh sách rỗng / phần tử thiếu trường → lỗi.
- Plugin: một ô đổi vẫn gửi dạng đơn (tương thích server cũ), nhiều ô gửi danh sách. Đếm trong xem trước là số câu (1 dòng = 1 update). Lợi ích: một dòng đổi nguyên tử, ít câu hơn trong xem trước, ít vòng gọi `edit.sql`.
- Server cũ + plugin mới + dòng nhiều ô: server cũ báo lỗi `change.set needs string column and value` — build lại server.
- Kiểm chứng: unit test cho từng nơi (SQL kèm `NULL` và dấu nháy đơn, CQL, mongo, ES dotted-path merge), test RPC (danh sách, rỗng, thiếu trường), smoke test headless Neovim + server thật (lưới: sửa 2 ô một dòng → xem trước 1 câu → commit đúng).
