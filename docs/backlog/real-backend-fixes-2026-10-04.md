# Lỗi tìm ra khi chạy với backend thật — 2026-10-04

Toàn bộ smoke test của plugin Neovim ban đầu chạy trên SQLite. Khi chạy tiếp với Postgres 16, ClickHouse 25.8, Dragonfly (giao thức Redis), MongoDB 7 và MySQL thật (qua Docker), phát hiện những lỗi dưới đây — phần lớn nằm sẵn trong driver, không phải trong code mới. Chưa chạy với Elasticsearch và Cassandra (image quá lớn để kéo trong lần này): phần plugin cho chúng (guard, modeline) chỉ có test đơn vị.

## Đã sửa

- **Postgres: `numeric`, mảng, enum, `inet`, `interval`, `bytea`, range... hiện `NULL`.** `stringify_column` đọc kết quả qua giao thức nhị phân của `sqlx::query()`, mỗi kiểu cần bộ giải mã riêng; kiểu không có trong danh sách (và cả `NUMERIC`, dù được liệt kê: nó bị giải mã như `f64` và luôn lỗi) bị nuốt thành `NULL`, giống hệt một giá trị rỗng thật. Sửa tận gốc: `run()` dùng `sqlx::raw_sql` (simple query protocol) nên server trả mọi giá trị đã là văn bản như `psql`. Hai chỉnh sửa trình bày: NULL thật là `NULL`, bool vẫn là `true`/`false` (không phải `t`/`f`). Hệ quả có thể thấy: `jsonb` hiện đúng cách Postgres in (`{"code": "ok"}`, có khoảng trắng) thay vì dạng gọn của `serde_json`. Test hồi quy: `numeric_arrays_and_other_exotic_types_show_their_text_not_null`; test JSON cũ được cập nhật kỳ vọng.
- **MySQL: cùng lỗi** (`DECIMAL`, `ENUM`, `SET`, `BLOB`, `BIT`, `YEAR`, hình học → `NULL`; chính comment trong code gọi đây là "known gap"). Sửa tương tự với `raw_sql` (`COM_QUERY`, text protocol); byte không phải UTF-8 (`BLOB`) hiện dạng `0x<hex>` thay vì `NULL`. `as_bytes` của sqlx-mysql là private nên đọc qua `try_get_unchecked::<&[u8]>`. Test: `decimal_enum_set_and_blob_columns_show_their_text_not_null`.
- **MySQL: `list_schema` hỏng với MySQL bản mới** (cả 3 test `list_schema_*` đã thất bại từ trước trên code gốc — kiểm chứng bằng cách chạy lại trên code chưa sửa): `information_schema` trả cột chuỗi kiểu `VARBINARY`, sqlx không giải mã được thành `String`, nên duyệt schema và completion trống hoàn toàn. Sửa: `CAST(.. AS CHAR)` cho mọi cột chuỗi trong 3 truy vấn.
- **Mongo: không nhận cú pháp mongosh.** Driver parse đối số bằng `serde_json` chặt: `{name: 'ann'}` báo `key must be a string`. Thêm `relax_json` (khoá không nháy, nháy đơn, dấu phẩy cuối) áp dụng cho mọi đối số — JSON chặt đi qua không đổi; hai hàm quét ngoặc/phẩy (`find_matching_close_paren`, `split_top_level_args`) cũng hiểu nháy đơn.
- **Mongo: sửa dòng chưa từng chạy được với MongoDB thật** (test `edit_sql_round_trips_a_real_update_and_delete_against_a_found_document` thất bại từ trước trên code gốc): bảng kết quả hiện `_id` là `ObjectId("…")`, `edit_sql` ghi đúng chuỗi đó vào filter `updateOne`/`deleteOne`, nhưng driver không parse được hàm `ObjectId(...)`. `relax_json` giờ hiểu `ObjectId`, `ISODate`, `NumberLong`, `NumberInt`, `NumberDecimal` (→ Extended JSON), nên vòng "xem → sửa → ghi lại" khép kín.
- **Server:** `insertOne`/`insertMany` của Mongo cũng làm mới completion (insert là cách một collection ra đời).
- **Plugin:** (1) dòng modeline `// tradar: x` / `# tradar: x` bị driver theo dòng (Mongo/Redis/ES) tách thành một "câu lệnh" riêng — `<leader>ra` sẽ cố chạy nó; giờ lọc các statement chỉ gồm comment; (2) `EXPLAIN` chỉ đúng với SQL — bản trước ghép `EXPLAIN ` vào cả Mongo/Redis/ES; giờ từ chối kèm lý do.

## Thêm cho Neovim (không phải lỗi)

- Mongo/Redis/Elasticsearch dùng được trong Neovim: file `.mongo`/`.redis`/`.esq` (hoặc bất kỳ file nào có dòng `-- tradar:`/`// tradar:`/`# tradar:`), guard theo từng ngôn ngữ (`drop()`, `deleteMany({})`, `FLUSHALL`, `DELETE /index`, `_delete_by_query` luôn hỏi; ghi trên connection `prod` hỏi), blink/telescope/`K`/`gd`/statusline hoạt động như với SQL.
- Kết quả dạng tài liệu (Mongo/ES) mặc định hiện **bảng** (cột phẳng `address.city`, `_id` đầu, còn lại theo thứ tự chữ cái vì Lua không giữ thứ tự khoá JSON), nên `i`/`dd`/`gyc`/export dùng lại được; `gT` đổi qua JSON. Giá trị Redis thuần (chuỗi, số) vẫn hiện dạng JSON — bảng một cột `value` không thêm gì.

## Chưa làm / biết trước

- Test với Elasticsearch và Cassandra thật.
- Mongo hiện tên cột theo thứ tự chữ cái, không theo thứ tự trường trong tài liệu.
- `gd` theo FK trong kết quả nhiều bảng (JOIN) vẫn chưa có.
