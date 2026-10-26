# Cheatsheet query mẫu theo driver (2026-10-26)

Yêu cầu: tradar-server có query mẫu để xem ngay trong nvim thay vì tự nhớ / đi tìm.

- **Dữ liệu ở server** (protocol 1.15): `QueryDriver::cheatsheet() -> &'static [CheatEntry]` (`category`, `title`, `text`; mặc định rỗng) + method `cheatsheet` → `{entries}`. Mỗi connector có `cheatsheet.rs` (postgres, mysql, sqlite, clickhouse, cassandra, mongo, redis, elasticsearch), 14–22 câu mỗi bộ theo nhóm. Không gọi database. Kafka/RabbitMQ/HTTP/Socket/gRPC chưa có (không phải driver truy vấn).
- **Chỗ điền là `${tên}`** — dùng lại cơ chế tham số (`params.lua`: hỏi lúc chạy, nhớ giá trị, `<Tab>` gợi ý), nên không thêm cú pháp mới và buffer giữ nguyên placeholder.
- **Xem:** `<leader>rk` / `:TradarCheatsheet` → telescope (preview, lọc theo nhóm/tên/nội dung, `<CR>` chèn, `<C-r>` chèn và chạy), rơi về `vim.ui.select` khi không có telescope (`cheatsheet_picker = 'ui'`). Chèn bằng `insert_statement` của `crud.lua` (dưới dòng con trỏ, cách bằng dòng trống).
- **Kiểm tra** (`cheatsheet_problems` + test mỗi crate): tiêu đề không trùng, `${` đóng đủ; SQLite chạy thật từng câu trên DB tạm; Postgres/MySQL/ClickHouse/Cassandra/Redis/Elasticsearch: mỗi mục đúng một statement (Elasticsearch còn kiểm JSON body, Redis kiểm lệnh đọc được); Mongo: mỗi câu parse được bằng shell của driver. Các câu của Postgres/MySQL/ClickHouse/Cassandra/Mongo/ES/Redis chưa được chạy thử trên server thật (không có Docker ở máy dev) — nếu câu nào lỗi trên phiên bản cụ thể thì sửa trong `cheatsheet.rs` của connector đó.

## Mở rộng (cùng ngày)

Phản hồi: "chưa đủ" -> thêm nhóm (Write, Schema, Admin, Transaction, SQL syntax) và mở rộng mỗi bộ lên 53–95 câu; thêm query của người dùng.

- **Bộ có sẵn:** sinh từ dữ liệu theo nhóm (Postgres 89, MySQL 87, ClickHouse 82, Mongo 82, Elasticsearch 79, Redis 95, SQLite 70, Cassandra 53). Có câu ghi (`INSERT`/upsert/`UPDATE`/`DELETE`, `SET`/`EXPIRE`, `_update_by_query`...), DDL và cú pháp hay phải tra. Chỗ điền đặt tên thống nhất (`${table}`, `${column}`, `${key}`, `${value}`...); chỗ cần số thì viết số cụ thể thay vì placeholder (để test parse được).
- **Của người dùng:** `cheatsheet.toml` cạnh `connections.toml`, `[[entry]]` với `driver`/`connection` (tuỳ chọn), `category` (mặc định `Mine`), `title`, `text`. Server đọc lại mỗi lần gọi `cheatsheet` (không cần restart), trả `user: true`, `warnings` (TOML sai, mục thiếu `title`/`text`) và `path`; plugin hiện `★`, báo cảnh báo, `:TradarCheatsheetEdit` / `<C-e>` mở file với template. Tests: `server/cheatsheet.rs` (parse, lọc, bỏ mục hỏng) và `tests/rpc.rs` (hợp nhất với SQLite thật, đọc lại sau khi sửa file, file hỏng).
- **Kiểm tra mạnh hơn:** SQLite chạy thật từng câu trên database tạm mới cho mỗi câu (bắt được `ADD COLUMN` trùng cột, unique index trên dữ liệu trùng...); Transaction chỉ kiểm tách câu.

## Chưa làm
- Chạy thử toàn bộ bộ query trên container thật trong CI Docker (Postgres, MySQL, ClickHouse, Cassandra, Redis, Elasticsearch, Mongo mới được kiểm cú pháp).
- Tìm trong cheatsheet theo driver khác (xem bộ của driver khác mà không đổi connection).
