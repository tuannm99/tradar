# Commit trong transaction, JSON view đọc được, hai lỗi Mongo/`.tdb`

## Commit trong một transaction

- Trước đây (`grid-edit-in-place-2026-10-22.md`) tôi ghi "pool không giữ `BEGIN` qua nhiều `execute`" — **sai**: các driver SQL (`transaction_control` + `transaction: Mutex<Option<Transaction>>`) giữ transaction trên một connection tới `COMMIT`/`ROLLBACK`, đó là cách TUI chạy migration. Nên không cần method batch mới ở `QueryDriver`.
- Plugin (`open_preview`/`apply`): với `postgres`/`mysql`/`sqlite` chạy `BEGIN`, từng câu, `COMMIT`. Câu thứ i lỗi → `ROLLBACK` → chạy lại truy vấn gốc (lưới hiện đúng DB) → mở lại xem trước với toàn bộ câu + `-- rolled back, nothing was applied; statement i of n failed: …`.
- Nếu `status.in_transaction` đã `true` (user tự `BEGIN`) thì không bọc — một `BEGIN` thứ hai là no-op và `COMMIT`/`ROLLBACK` của ta sẽ chốt nhầm công việc của họ; cảnh báo và chạy bên trong. Connector khác (Mongo/ES/Cassandra…) giữ hành vi cũ: dừng ở câu lỗi đầu tiên, xem trước chỉ còn phần chưa chạy.
- Server: `status` từng chờ `ping` qua pool nên treo đến hết acquire-timeout khi đang có transaction (connection bị transaction giữ) và báo `alive: false`; nay `in_transaction` ⇒ `alive: true` mà không ping. Test `status_answers_at_once_inside_a_held_transaction`.
- Hạn chế còn lại: khi user đang giữ transaction mở, các lời gọi đi qua pool (schema, `edit.source` lần đầu) vẫn chờ connection; lưới chỉ sửa được nếu thông tin sửa đã được cache từ trước.
- Kiểm chứng (SQLite thật qua `tradar-server`): 4 câu commit "trong một transaction"; câu 2 lỗi `NOT NULL` → câu `UPDATE` đầu bị rollback (DB không đổi), xem trước mở lại đủ 2 câu; user tự `BEGIN` → cảnh báo, transaction của họ vẫn mở sau commit, `ROLLBACK` của họ hủy thay đổi.

## JSON view cho Mongo/Elasticsearch

`<leader>rv` đã có từ trước nhưng JSON view là `vim.json.encode` một dòng mỗi document: khó đọc với document lồng nhau và **mất thứ tự trường** (bảng băm Lua). Nay `render.pretty`/`render.documents(items, field_order)`: thụt lề 2 khoảng trắng, trường cấp 1 theo `field_order` của server (cấp lồng nhau theo alphabet vì không có hint), document cách nhau một dòng trống, `syntax=json` để tô màu (không đặt `filetype` để khỏi kéo ftplugin/LSP vào buffer scratch). `gyj` dùng cùng dạng. Hạn chế: trường `null` bị `luanil` của lớp rpc bỏ khi decode (như trước).

## Lỗi: comment trong query Mongo

`db.users.find({\n  // _id: ObjectId("…")\n})` → `invalid JSON argument: key must be a string`: `relax_json` cố ý không hiểu `//`. Nay `strip_js_comments` (nhận biết chuỗi `"`/`'`, giữ xuống dòng của comment khối) chạy ở đầu `parse_shell_query` và `execute`. Test: dòng bị comment trong lời gọi, `//` trong chuỗi URL giữ nguyên, dấu `'` trong comment không mở chuỗi.

## Lỗi: `.tdb` lúc chạy lúc không

`<leader>rr` chỉ chạy khi con trỏ nằm trong **thân** block; đứng trên dòng ``` mở/đóng (rất hay gặp) báo "cursor is not inside a block" — nên trông như lúc được lúc không. `blocks.at(lines, line, loose)`/`M.block_at(buf, row, true)`: dòng fence thuộc block của nó; chọn câu lệnh đầu (fence mở) hoặc cuối (fence đóng). Văn bản giữa các block vẫn báo, thông báo nói rõ hơn. Kiểm chứng: chạy `rr` ở từng dòng của một `.tdb` hai block (SQLite thật).
