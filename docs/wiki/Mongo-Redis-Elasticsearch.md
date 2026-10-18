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

Dạng `db.<collection>.<method>(<args>)`. Hỗ trợ `find` (kèm projection, và chain `.sort()`/`.limit()`/`.skip()`/`.count()`), `findOne`, `countDocuments`, `aggregate`, `insertOne`, `insertMany`, `updateOne`, `updateMany`, `deleteOne`, `deleteMany`. Đây là parser tối giản chứ không phải JS engine thật: không có `$where`, bulk operation hay transaction; thứ gì ngoài các dạng trên trả lỗi "unsupported query".

- Nhận cú pháp mongosh (`{name: 'ann'}`, `ObjectId("…")`, `ISODate(…)`), không chỉ JSON chặt, và cả comment `// …` / `/* … */` ở bất kỳ đâu trong lời gọi (dòng filter đang tạm tắt không làm query hỏng).
- `use <db>` đổi database đang thao tác, `show dbs` liệt kê database. `target` không cần kèm `/db`.
- Câu lệnh trải nhiều dòng được hiểu đúng: dòng bắt đầu bằng `db.`/`use`/`show dbs` mở câu mới, dòng khác nối vào câu phía trên.
- `find`/`aggregate` dừng ở 10.000 document đầu và báo "truncated" nếu còn nhiều hơn; `.count()` vẫn trả số thật.
- Schema (navigator, completion) suy ra bằng cách đọc mẫu một document mỗi collection; field lồng nhau hiện dạng `address.city`. Cột trong bảng kết quả theo đúng thứ tự field của document.

## Redis

Một dòng lệnh mỗi lần chạy, tách theo khoảng trắng (không hỗ trợ quoting). `HGETALL` và `ZRANGE`/`ZREVRANGE … WITHSCORES` được format nhận biết kiểu; các lệnh khác dùng chuyển đổi tổng quát sang JSON. Không có pipelining, `MULTI`/`EXEC`, pub/sub hay xử lý riêng cho stream.

## Elasticsearch

Kiểu Kibana Dev Tools: dòng đầu `METHOD /path`, các dòng sau là JSON body:

```
GET /my-index/_search
{ "query": { "match_all": {} } }
```

- Body được kiểm tra JSON ngay trên máy trước khi gửi; chấp nhận cả NDJSON (một JSON mỗi dòng) cho `_bulk`/`_msearch`.
- Response không phải 2xx hiện như lỗi thật, kèm body lỗi của Elasticsearch.
- Mỗi lần chạy chỉ một request; chưa có cấu hình auth/TLS client-cert riêng.

## Kết quả dạng tài liệu

Kết quả mặc định hiện **bảng** (cột phẳng `address.city`) để sửa trực tiếp như SQL (sửa ô/xoá dòng rồi `:w` → xem trước → commit; chưa có thêm dòng mới cho Mongo/ES) và dùng `gyc`/export; `<leader>rv` đổi sang JSON (mỗi document in thụt lề, cách nhau một dòng trống, trường cấp 1 theo thứ tự lưu trong DB, cấp lồng nhau theo alphabet, tô màu cú pháp JSON; `gyj` copy cả kết quả cùng dạng này). Câu trả lời đơn giản (một chuỗi Redis, danh sách số) vẫn hiện dạng JSON. Ép một kiểu bằng `setup{ documents_view = "table" | "json" }`.

Sửa/xoá dòng hoạt động cho Mongo (`find()` đơn giản, khoá `_id`) và Elasticsearch (`_search` trên một index, khoá `_id`); Redis chỉ đọc. `<leader>re` (EXPLAIN) chỉ có cho SQL.

## An toàn

Có luật hỏi xác nhận riêng cho từng ngôn ngữ — xem [An toàn](Truy-van-SQL#an-toàn).
