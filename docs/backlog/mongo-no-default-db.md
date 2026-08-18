# MongoDB: connect không cần chọn DB, lệnh `use`/`show dbs` (user yêu cầu 2026-08-18) — xong

User yêu cầu: connect được vào MongoDB mà không cần URI chỉ định sẵn database, dùng được lệnh `use <db>` để đổi database, tự động chọn database mặc định nếu URI có, và xem được danh sách nhiều database.

Thực hiện trong `crates/tradar-connector-mongo/src/lib.rs`:

- `MongoDriver` có thêm field `current_db: Mutex<Option<String>>` — trạng thái session duy nhất driver này cần mà `QueryDriver::execute(&self, ...)` (không phải `&mut self`) không tự cho phép. `current_db_name()` fallback về `"test"` khi chưa có gì được chọn, đúng default của `mongosh` khi không truyền db.
- `connect()` không còn bắt buộc `client.default_database()` — ping chạy trên database `admin` (lệnh admin, trả lời bất kể URI có tên db hay không) để verify connectivity. Nếu URI có tên database (`mongodb://host/mydb`) thì vẫn tự chọn vào nó như cũ.
- `execute()` bắt hai shell helper trước khi chạy qua `parse_shell_query`: `use <db>` đổi `current_db` (trả về `{"switched to db": "<db>"}` để hiển thị xác nhận), `show dbs`/`show databases` gọi `Client::list_databases()` trả về `Documents` gồm `name`/`sizeOnDisk`/`empty` cho từng database.
- **Giới hạn đã biết**: navigator schema tree (`list_schema`) chỉ fetch một lần lúc connect (kiến trúc `QueryEngine` hiện chưa có cơ chế refresh sống — xem `crates/tradar-query-workbench/src/query_engine.rs`), nên cây collection trong navigator vẫn đứng yên ở database lúc connect (URI's default hoặc `"test"`) dù sau đó chạy `use` đổi db trong query editor. `show dbs` là cách xem các database khác trong lúc chờ; nếu cần navigator tự refresh theo `use`, đó là việc riêng, chưa được scope ở đây.
- Test mới trong `tradar-connector-mongo`: `connect_succeeds_when_the_uri_names_no_database`, `connect_auto_selects_the_uri_s_default_database`, `use_switches_which_database_subsequent_queries_target`, `show_dbs_lists_a_database_that_has_data_in_it` — cả 32 test của crate (Docker/testcontainers) pass, `cargo clippy -p tradar-connector-mongo --all-targets -- -D warnings` sạch.

