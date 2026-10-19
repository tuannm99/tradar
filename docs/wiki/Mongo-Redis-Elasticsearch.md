# Mongo, Redis, Elasticsearch

Cách dùng giống [SQL](Truy-van-SQL) (cùng phím `<leader>r…`, cùng buffer kết quả); khác ở ngôn ngữ và cách gắn file.

## Gắn file

Dùng đuôi file `.mongo` (tô màu như JavaScript), `.redis`, `.esq` (Elasticsearch), hoặc **bất kỳ file nào** có dòng liên kết ở đầu:

```
// tradar: tên-connection     (Mongo)
# tradar: tên-connection      (Redis / Elasticsearch)
-- tradar: tên-connection     (SQL)
```

Hoặc gom nhiều ngôn ngữ vào một file với [`.tdb`](File-tdb).

## MongoDB

Dạng `db.<collection>.<method>(<args>)`. Hỗ trợ `find` (kèm projection, và chain `.sort()`/`.limit()`/`.skip()`/`.hint(<tên index hoặc {field: 1}>)`/`.count()`/`.explain()`), `findOne`, `countDocuments`, `aggregate`, `insertOne`, `insertMany`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`. Đây là parser tối giản chứ không phải JS engine thật: không có `$where`, bulk operation hay transaction; thứ gì ngoài các dạng trên trả lỗi "unsupported query".

- Nhận cú pháp mongosh (`{name: 'ann'}`, `ObjectId("…")`, `ISODate(…)`), không chỉ JSON chặt, và cả comment `// …` / `/* … */` ở bất kỳ đâu trong lời gọi (dòng filter đang tạm tắt không làm query hỏng).
- Dấu `;` ở cuối câu được chấp nhận như trong mongosh (`db.users.find();`).
- **Lệnh console** (không cần collection): `use <db>` đổi database đang thao tác (`target` không cần kèm `/db`); `show dbs` / `show databases`; `show collections` / `show tables`; `show users`; `show roles`; `show profile` (20 thao tác mới nhất trong `system.profile` của database hiện tại, rỗng cho tới khi bật profiler); `show profile` (20 thao tác mới nhất trong `system.profile`, rỗng cho tới khi bật profiler); `db` (tên database hiện tại). Các `show` khác báo lỗi và gợi ý danh sách trên.
- **Lời gọi cấp database**: `db.getName()`, `db.getCollectionNames()`, `db.getCollectionInfos()`, `db.stats()`, `db.version()`, `db.serverStatus()`, `db.runCommand({…})`, `db.adminCommand({…})`. Danh sách trả về là một giá trị mảng; `runCommand`/`adminCommand` trả document phản hồi của server.
- **Phương thức collection khác**: `distinct("field", {filter})` (mỗi giá trị một dòng cột `value`), `getIndexes()` (cột `name`/`key`/`unique`), `estimatedDocumentCount()`. Không dùng chain và không sửa-tại-chỗ được.
- `db.getSiblingDB("khác").…` chạy đúng một câu trên database khác mà không đổi `use` (vd `db.getSiblingDB("shop").getCollectionNames()`, `db.getSiblingDB("shop").orders.find({})`); kết quả kiểu này không sửa-tại-chỗ được.
- **Kiểm tra index**: `db.users.find({…}).sort({…}).explain()` trả plan chạy thật (`executionStats`) — tìm `IXSCAN` (dùng index) hay `COLLSCAN` (quét cả collection) và `totalDocsExamined`. `.hint(…)` ép dùng một index. `.count()` do server đếm (qua index nếu có), có tính cả `.skip()`/`.limit()`/`.hint()`; `.explain()` và `.count()` phải là lời gọi cuối chain.
- `db.getCollection("tên-lạ").find(…)` dùng cho collection có tên không phải định danh thường.
- Câu lệnh trải nhiều dòng được hiểu đúng: dòng bắt đầu bằng `db`, `db.`, `use`, `show` mở câu mới, dòng khác nối vào câu phía trên.
- `find`/`aggregate` dừng ở 10.000 document đầu và báo "truncated" nếu còn nhiều hơn; `.count()` vẫn trả số thật.
- Schema (navigator, completion) suy ra bằng cách đọc mẫu một document mỗi collection; field lồng nhau hiện dạng `address.city`. Cột trong bảng kết quả theo đúng thứ tự field của document.

## Redis

Một dòng lệnh mỗi lần chạy, tách theo khoảng trắng như `redis-cli`: giá trị có dấu cách đặt trong nháy — `SET greeting "hello world"`, `SET k 'it\'s'`. Trong `"…"` hiểu `\n \r \t \b \a \\ \" \xHH`; trong `'…'` chỉ `\'`; nháy chỉ có nghĩa khi mở đầu một token (`SET k it's` vẫn chạy), nháy không đóng hoặc ký tự dính sau nháy đóng báo lỗi. Snippet do navigator sinh tự đặt nháy cho key có dấu cách. `HGETALL` và `ZRANGE`/`ZREVRANGE … WITHSCORES` được format nhận biết kiểu; các lệnh khác dùng chuyển đổi tổng quát sang JSON. Không có pipelining, `MULTI`/`EXEC`, pub/sub hay xử lý riêng cho stream.

## Elasticsearch

Kiểu Kibana Dev Tools: dòng đầu `METHOD /path`, các dòng sau là JSON body:

```
GET /my-index/_search
{ "query": { "match_all": {} } }
```

- Comment kiểu Kibana được bỏ trước khi gửi: dòng bắt đầu bằng `#` ở bất kỳ đâu, `// …` và `/* … */` trong body (không nhầm với `//` trong đường dẫn hay trong chuỗi `"http://…"`).
- Body được kiểm tra JSON ngay trên máy trước khi gửi; chấp nhận cả NDJSON (một JSON mỗi dòng) cho `_bulk`/`_msearch`.
- `_search` thường hiện mỗi hit là một dòng (`_id` + các field của `_source`). Nếu response có `aggregations` hoặc `suggest` (thường kèm `"size": 0` nên không có hit) thì hiện **nguyên response** dạng JSON, vì phần đó mới là câu trả lời. Một document lồng nhiều tầng (response aggregation, `serverStatus`) tự mở ở dạng JSON thay vì bảng một dòng rất rộng; `<leader>rv` đổi qua lại, `setup{ documents_view }` ép một kiểu.
- Response không phải 2xx hiện như lỗi thật, kèm body lỗi của Elasticsearch.
- Mỗi lần chạy chỉ một request; chưa có cấu hình auth/TLS client-cert riêng.

## Kết quả dạng tài liệu

Kết quả mặc định hiện **bảng** (cột phẳng `address.city`) để sửa trực tiếp như SQL (sửa ô/xoá dòng rồi `:w` → xem trước → commit; chưa có thêm dòng mới cho Mongo/ES) và dùng `gyc`/export; `<leader>rv` đổi sang JSON (mỗi document in thụt lề, cách nhau một dòng trống, trường cấp 1 theo thứ tự lưu trong DB, cấp lồng nhau theo alphabet, tô màu cú pháp JSON; `gyj` copy cả kết quả cùng dạng này). Câu trả lời đơn giản (một chuỗi Redis, danh sách số) vẫn hiện dạng JSON. Ép một kiểu bằng `setup{ documents_view = "table" | "json" }`.

Sửa/xoá dòng hoạt động cho Mongo (`find()` đơn giản, khoá `_id`) và Elasticsearch (`_search` trên một index, khoá `_id`); Redis chỉ đọc. `<leader>re` (EXPLAIN) chỉ có cho SQL.

## An toàn

Có luật hỏi xác nhận riêng cho từng ngôn ngữ — xem [An toàn](Truy-van-SQL#an-toàn).
