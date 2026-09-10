# Mongo: method chaining sau find(), findOne/countDocuments, projection — xong (2026-09-10)

Gap đã ghi sẵn trong `docs/architecture.md` ("Không có method chaining (`.sort()`, `.limit()`)") — user chọn ưu tiên qua `AskUserQuestion` sau khi PR row-edit merge. Chốt phạm vi qua `AskUserQuestion` (3 câu, cùng lúc): chain `sort`/`limit`/`skip` **+ `count()`**, thêm 2 lệnh top-level mới `findOne`/`countDocuments`, và `find()` nhận luôn tham số projection (đối số thứ 2) — cả ba làm chung một đợt vì cùng đụng `run_method`/parser.

**Vẫn cố tình không làm** (ngoài phạm vi đã chốt): `.forEach()`/bất kỳ chain nào cần chạy callback JS thật (parser này không phải JS engine, xem doc comment đầu file) — gặp là báo lỗi rõ ràng, không giả vờ hỗ trợ. Chain method nào ngoài 4 cái (`sort`/`limit`/`skip`/`count`) đều bị từ chối tương tự, kể cả trên `findOne`/`aggregate`/mọi method khác — `reject_chain()` áp dụng cho mọi method trừ `find`.

## Kiến trúc parser

`ParsedQuery` đổi từ `{collection, method, args}` (1 lệnh) sang `{collection, calls: Vec<MethodCall>}` — `calls[0]` (`primary()`) là lệnh chính, `calls[1..]` (`chain()`) là các lệnh chain. `parse_shell_query` giờ chạy vòng lặp: đọc một `method(args)`, rồi kiểm tra ký tự ngay sau `)` — gặp `.` thì đọc tiếp lệnh kế, gặp hết chuỗi thì dừng, gặp gì khác thì báo lỗi.

**Tìm đúng dấu `)` đóng của từng lệnh** (`find_matching_close_paren`) là phần khó nhất: lệnh cũ (1 lệnh duy nhất) chỉ cần `strip_suffix(')')` ở cuối chuỗi — không còn đúng khi có nhiều lệnh nối tiếp, vì phải biết `)` nào đóng lệnh hiện tại chứ không phải `)` cuối cùng của cả chuỗi. Viết một hàm quét đúng độ sâu `(`/`{`/`[` **và bỏ qua nội dung bên trong chuỗi JSON** (một `)`/`}` nằm trong `"..."` không được tính).

**Bug có sẵn phát hiện giữa chừng, không liên quan chaining**: `split_top_level_args` (tách các đối số cách nhau bởi dấu phẩy top-level trong một lệnh) có đúng lỗi y hệt — không bỏ qua nội dung chuỗi, nên `db.users.find({"note": "a } b"})` (giá trị string chứa ký tự `}`) sẽ đếm sai độ sâu và báo "unbalanced braces" dù JSON hoàn toàn hợp lệ. Phát hiện nhờ 1 test mới viết cho `find_matching_close_paren` (`a_brace_inside_a_json_string_argument_does_not_confuse_the_call_boundary`) test hỏng ngay cả khi logic mới đúng — hoá ra lỗi nằm ở hàm cũ `split_top_level_args`, không phải hàm mới. Sửa bằng đúng cơ chế bỏ-qua-chuỗi giống `find_matching_close_paren` (duplicate logic có chủ đích, không rút thành hàm dùng chung — hai hàm có vòng lặp khác hình dạng, gộp lại sẽ phức tạp hơn giữ nguyên).

## Chain `sort`/`limit`/`skip`/`count`

Dùng thẳng builder thật của driver Rust (`Collection::find` trả về action builder có `.sort(Document)`/`.limit(i64)`/`.skip(u64)`/`.projection(Document)`, xác nhận qua source code driver `mongodb-3.8.0`), để MongoDB tự làm sort/skip/limit ở server — không tự fetch hết rồi lọc tay ở client. `count()` xử lý riêng: **bắt buộc là lệnh cuối cùng trong chain** (`.count().limit(1)` bị từ chối, `.limit(1).count()` hợp lệ) — sau khi build xong cursor với mọi modifier trước đó, fetch documents như bình thường rồi trả `{"count": <số document>}` thay vì list document. Cách hiểu "count sau khi limit/skip" khác `mongosh` thật (bản mới của `mongosh` deprecate `.count()` cũ, khuyên dùng `countDocuments()`) nhưng là cách hiểu tự nhất quán duy nhất khi không có server round-trip riêng cho count.

## `findOne`/`countDocuments`

Hai lệnh top-level mới, không nhận chain (`reject_chain()`). `findOne` trả `Documents` rỗng khi không khớp gì (không phải lỗi) — nhất quán với "danh sách document, có thể rỗng" thay vì một marker "không tìm thấy" riêng. `countDocuments` trả `{"count": <n>}`, dùng `Collection::count_documents` thật của driver (đếm chính xác server-side, khác `estimatedDocumentCount` vốn dùng metadata collection có thể lệch).

## Projection

`find()` giờ nhận đối số thứ 2 (object projection, ví dụ `{"name": 1, "_id": 0}`) — trước đây bị từ chối thẳng (`max_args(1)`). Tác dụng phụ tốt: CRUD snippet Read đã sinh sẵn đúng shape `find({}, {name: 1})` từ trước (xem `crud_snippet`'s Read op) nhưng chạy lên sẽ lỗi — giờ chạy đúng, không phải bug mới mà là bug cũ được vá theo.

## `edit_source` cho chain

Row-edit (`docs/backlog/mongo-es-row-edit.md`) vẫn hoạt động cho `find()` có chain `sort`/`limit`/`skip` — các modifier này không đổi ý nghĩa "1 row = 1 document", khác `aggregate`. Chain có `count()` thì **không** editable (không còn document nào để sửa, chỉ còn 1 con số) — `edit_source` kiểm tra mọi lệnh trong `chain()` chỉ được là `sort`/`limit`/`skip`, có `count` (hoặc bất kỳ gì khác) thì trả `None`.

## Test

- Parser (không cần Docker): chain nhiều lệnh, `count()` đứng cuối, lỗi thiếu `)` của lệnh chain, string chứa `}` không phá vỡ ranh giới lệnh.
- `run_method` (không cần Docker — xây `Collection` qua `Client::with_uri_str` vào cổng không ai lắng nghe; driver Mongo connect lười, `with_uri_str`/`with_options` không tự gọi mạng, nên an toàn miễn test không bao giờ `.await` một operation thật): từ chối chain trên method không hỗ trợ, `count()` không phải lệnh cuối, chain method lạ, `limit`/`skip` không phải số, `sort` sai số đối số — toàn bộ lỗi validate trước khi chạm mạng nên verify được không cần Mongo thật.
- Tích hợp thật (cần Docker): projection loại đúng field, `sort+skip+limit` phân trang đúng, `count()` trả đúng số, `findOne` tìm thấy/không thấy, `countDocuments` đếm đúng.
- `edit_source`: chấp nhận chain `sort/limit/skip`, từ chối chain có `count`.

`docs/architecture.md` cập nhật đoạn mô tả MongoDB (thêm `findOne`/`countDocuments`, chain `sort`/`limit`/`skip`/`count`, projection) — README.md không có đoạn riêng nào mô tả phạm vi ngôn ngữ query từng driver (đoạn cuối "Database" tự dẫn sang `docs/architecture.md` cho việc đó), nên không cần sửa.
