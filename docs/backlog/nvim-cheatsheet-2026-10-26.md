# Cheatsheet query mẫu theo driver (2026-10-26)

Yêu cầu: tradar-server có query mẫu để xem ngay trong nvim thay vì tự nhớ / đi tìm.

- **Dữ liệu ở server** (protocol 1.15): `QueryDriver::cheatsheet() -> &'static [CheatEntry]` (`category`, `title`, `text`; mặc định rỗng) + method `cheatsheet` → `{entries}`. Mỗi connector có `cheatsheet.rs` (postgres, mysql, sqlite, clickhouse, cassandra, mongo, redis, elasticsearch), 14–22 câu mỗi bộ theo nhóm. Không gọi database. Kafka/RabbitMQ/HTTP/Socket/gRPC chưa có (không phải driver truy vấn).
- **Chỗ điền là `${tên}`** — dùng lại cơ chế tham số (`params.lua`: hỏi lúc chạy, nhớ giá trị, `<Tab>` gợi ý), nên không thêm cú pháp mới và buffer giữ nguyên placeholder.
- **Xem:** `<leader>rk` / `:TradarCheatsheet` → telescope (preview, lọc theo nhóm/tên/nội dung, `<CR>` chèn, `<C-r>` chèn và chạy), rơi về `vim.ui.select` khi không có telescope (`cheatsheet_picker = 'ui'`). Chèn bằng `insert_statement` của `crud.lua` (dưới dòng con trỏ, cách bằng dòng trống).
- **Kiểm tra** (`cheatsheet_problems` + test mỗi crate): tiêu đề không trùng, `${` đóng đủ; SQLite chạy thật từng câu trên DB tạm; Postgres/MySQL/ClickHouse/Cassandra/Redis/Elasticsearch: mỗi mục đúng một statement (Elasticsearch còn kiểm JSON body, Redis kiểm lệnh đọc được); Mongo: mỗi câu parse được bằng shell của driver. Các câu của Postgres/MySQL/ClickHouse/Cassandra/Mongo/ES/Redis chưa được chạy thử trên server thật (không có Docker ở máy dev) — nếu câu nào lỗi trên phiên bản cụ thể thì sửa trong `cheatsheet.rs` của connector đó.

## Chưa làm
- Cheatsheet do người dùng thêm vào (file trong `queries_dir`/config) trộn chung với bộ có sẵn.
- Chạy thử toàn bộ bộ query trên container thật trong CI Docker.
