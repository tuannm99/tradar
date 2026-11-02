# Connection viết thẳng tại chỗ + văn bản thuần (2026-11-02)

**Connection không cần alias.** Fence ` ```elasticsearch http://localhost:9201 ` / modeline `-- tradar: postgres://…` / `-- tradar: sqlite /tmp/x.db` mang chính địa chỉ.

- Server: tên connection `@driver:target` (`server/connections.rs`: `adhoc_connection`, `Server::find_connection` dùng cho `connect` và `cheatsheet`) mở `target` bằng connector `driver` mà không cần entry trong `connections.toml`; tên không có `@` vẫn phải được lưu. Protocol 1.18. Log che mật khẩu trong URL (`redact`). HTTP/gRPC đã có quy ước `@đích` riêng (1.16).
- Plugin (`conn.lua`): `adhoc_name(conn, lang)` đổi cách viết thành tên đó — ngôn ngữ của fence hoặc scheme (`postgres://`, `mongodb://`, `redis://`, `mysql://`, `clickhouse://`) cho driver; đường dẫn → SQLite; `host:port` cần ngôn ngữ (cassandra); `driver đích` cho modeline; một URL `http(s)://` không scheme riêng phải ghi rõ driver. `adhoc_driver`/`cached_driver` biết driver của tên này không cần hỏi server; `display_name` che mật khẩu ở statusline. Mở `.tdb` thì mở sẵn connection của các fence. ` ```http <base-url> ` / ` ```grpc <host:port> ` là connection tạm của request. Fence `elasticsearch`/`postgres`/`mysql`/`sqlite`/… được đăng ký màu JSON/SQL.
- Alias không đổi; một alias không tồn tại vẫn báo `no saved connection named`.
- Test: unit `adhoc_connection`/`redact` + `connect` không cần store (`server/tests.rs`); `adhoc_spec` (bảng ánh xạ, block `.tdb` chạy/hoàn thành, modeline, alias thiếu, HTTP base URL + curl).

**Văn bản thuần (Elasticsearch `_cat/…`).** Connector bọc body `text/plain` thành một document là chuỗi; JSON view hiện `"…\n…"` trên một dòng. `render.documents` giờ in document-là-chuỗi thành các dòng thật (bỏ `\n` cuối), không màu JSON (`render.is_text_reply`); chuỗi nằm trong field vẫn escape. Test `render_spec`, `results_spec`.
