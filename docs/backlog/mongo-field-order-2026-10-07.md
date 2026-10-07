# Neovim: thứ tự cột tài liệu Mongo theo thứ tự trường thật (2026-10-07)

## Bối cảnh

Tiếp theo `nvim-schema-tree-2026-10-06.md` — mục còn lại trong "Mục nhỏ" của `docs/roadmap.md`'s "Thứ tự tiếp theo" #3. Người dùng hỏi "tiếp tục xem làm gì"; agent rà roadmap, đánh giá cả 3 mục còn lại ("thứ tự cột Mongo", "`gd` theo FK cho `JOIN`", "huỷ query thật") đều cần đụng `tradar-server`/protocol hoặc nhiều driver — khác hẳn navigator cây lần trước (thuần plugin). Đào sâu hơn mới thấy mục "thứ tự cột" phức tạp hơn tên gọi: không chỉ là "server gửi đúng thứ tự" mà còn vướng việc `vim.json.decode()` của Neovim tự nó không giữ được thứ tự khi giải mã JSON object thành Lua table. Người dùng chọn làm mục này trước (phạm vi hẹp nhất trong 3 — chỉ 1 connector, không lan sang SQL driver khác).

## Vấn đề cụ thể (2 lớp, không phải 1)

1. **Lớp Rust**: `serde_json::Value::Object` mặc định là `BTreeMap` (alphabet) trừ khi bật feature `preserve_order`. BSON `Document` (kiểu dữ liệu MongoDB dùng) tự nó CÓ thứ tự field thật (theo lúc insert) — nhưng `Bson::Document(doc).into_relaxed_extjson()` (chuyển BSON → `serde_json::Value`) đã làm mất thứ tự đó **ngay trong Rust**, trước khi `QueryResult::Documents` kịp ra khỏi `tradar-connector-mongo`. Không sửa được ở tầng nào xa hơn (server, Lua) nếu thứ tự đã mất từ gốc.
2. **Lớp Lua**: dù lớp 1 được sửa (JSON text gửi qua wire đúng thứ tự), `vim.json.decode()` giải mã một object JSON thành Lua **table** (hash map) — `pairs()` trên table đó không có thứ tự xác định (tài liệu Lua không cam kết gì), nên `render.flatten`'s cách cũ (duyệt `pairs()` để "phát hiện" cột) vẫn ngẫu nhiên dù JSON input đã đúng thứ tự. Đây đúng là điều comment cũ của `render.flatten` đã nói: "Lua's JSON decode does not keep key order".

Phải sửa **cả hai lớp** — sửa một mà bỏ lớp còn lại thì không có tác dụng gì.

## Thay đổi

### Lớp Rust: `preserve_order` + `field_order` mới trong protocol

- `tradar-connector-mongo/Cargo.toml`: bật `features = ["preserve_order"]` cho `serde_json`. Cargo unify feature theo toàn workspace — **không đổi chữ ký hàm nào ở đâu cả**, chỉ đổi thứ tự lặp/serialize bên trong `serde_json::Value::Object` cho **mọi** crate đụng tới nó (không riêng Mongo) — verify bằng chạy lại toàn bộ `make test-unit`/`cargo clippy --workspace` sau khi bật, thay vì soát tay từng chỗ gọi `serde_json::to_string`/`assert_eq!` trong 17 crate (grep trước cho thấy không có assertion nào so sánh chuỗi JSON nhiều-key chính xác — equality của `Value::Object`/`Map` vốn đã so theo nội dung bất kể thứ tự nội bộ, không phải theo thứ tự — nên rủi ro thấp, nhưng vẫn chạy lại suite thật để chắc chứ không chỉ tin vào lý luận).
- `StoredResult::field_order(&self, offset, limit) -> Option<Value>` mới trong `tradar-server/src/protocol.rs` — `None` cho `Table` (đã có `columns()` nói cùng một điều, một lần cho cả kết quả thay vì từng dòng), `Some(...)` cho `Documents`: một mảng, mỗi phần tử là danh sách tên field **cấp cao nhất** của đúng document đó trong trang (`null` nếu phần tử đó không phải object — phòng hờ, không phải trường hợp thật của Mongo). Gắn vào response của cả `execute` (trang đầu) và `fetch` (trang kế) như field phụ `field_order`, **không đổi hình dạng `rows`** — mọi nơi khác đọc `rows` (JSON view, `yank`/export, `edit.source`/`edit.sql`...) không cần biết field này có tồn tại hay không, không có migration nào để làm cho các RPC method khác.

### Lớp Lua: `render.flatten` nhận `order`, dùng khi có

`nvim/lua/tradar/render.lua`'s `M.flatten(items, order)` — tham số `order` mới, optional:

- **Có `order`**: field cấp cao nhất của document `i` đi theo đúng `order[i]` (danh sách tên field server gửi), không còn tự "phát hiện" qua `pairs()`. Field nào một document sau giới thiệu mà document trước không có trong `order` của nó thì nối vào sau, theo đúng vị trí trong `order` của document sau — không mất field nào, chỉ không chắc thứ tự toàn cục 100% khi kết quả có document không đồng nhất field (hiếm, Mongo schemaless).
- **Không có `order`** (server cũ, hoặc driver khác không gửi field này): rơi về đúng hành vi cũ — alphabet, `_id` đầu.
- **Field lồng nhau** (dotted path kiểu `address.city`): `order` chỉ nói về cấp cao nhất, không có thông tin gì cho cấp lồng — vẫn alphabet như cũ, nhưng giờ xếp thành **một khối sau** mọi field cấp cao nhất, thay vì interleave alphabet lẫn với field cấp cao nhất như cách cũ (một thay đổi nhỏ, có chủ đích: nhóm field "thật" trước field lồng sâu đọc hợp lý hơn trộn ngẫu nhiên theo alphabet).
- Bug nhỏ bắt được khi viết: `order[i]` có thể là `vim.NIL` (không phải Lua `nil` thật) khi document đó không phải object phía server — `rpc.lua`'s decode chỉ thay `null` thành `nil` cho giá trị field **trong object**, không áp dụng cho phần tử **trong array** (xoá sẽ làm lệch index mảng) — `vim.NIL` lọt vào code mà không guard sẽ gọi `ipairs(vim.NIL)` và lỗi. Thêm check `if keys == vim.NIL then keys = nil end`.
- `init.lua`: `state.result.field_order` lưu song song với `rows` (3 chỗ dùng: `show_result` khởi tạo, `M.more` nối thêm mỗi lần tải trang kế — `vim.list_extend` cả hai mảng cùng lúc để không bao giờ lệch index, `paint_results`/`export_text` truyền `r.field_order` vào `render.flatten`).

## Test

Không có `nvim`/Lua interpreter sẵn trong sandbox này (giống lần trước) — cài `lua5.3` qua `apt`, viết harness độc lập chạy `render.flatten` thật (không qua Neovim), 7 nhóm kịch bản: có `order` thì cột theo đúng thứ tự cho sẵn; không có `order` thì rơi về alphabet/`_id`-đầu như cũ; document sau giới thiệu field mới `order` trước không nói tới; field lồng nhau vẫn alphabet nhưng thành khối riêng sau field cấp cao nhất; `order[i] == vim.NIL` không crash; document rỗng/scalar. Tất cả pass, không hồi quy với harness `schema_tree` cũ (lần trước) chạy lại.

Phía Rust: 4 test đơn vị mới cho `StoredResult::field_order` (`tradar-server`, không cần Docker — trực tiếp dựng `StoredResult::Documents` bằng tay) + 1 test tích hợp mới trong `tradar-connector-mongo` (cần Docker, không chạy được trong sandbox này — insert `{"z":1,"a":2,"m":3}`, xác nhận `find()` trả về đúng thứ tự `_id, z, a, m`, không bị resort alphabet). `cargo build --workspace`/`--tests`, `cargo clippy --all-targets --workspace -- -D warnings`, `cargo fmt --all -- --check`, `make test-unit` đều sạch sau khi bật `preserve_order` — không phát hiện assertion nào trong toàn bộ 17 crate phụ thuộc vào thứ tự `BTreeMap` cũ.

## Chưa làm

- Field lồng nhau (dotted path) vẫn chưa có thứ tự thật — `field_order` chỉ nói cấp cao nhất. Làm đầy đủ (đệ quy xuống từng cấp) cần `field_order` lồng nhau tương ứng (cây, không phải danh sách phẳng) và `render.flatten`'s walk đệ quy phải tiêu thụ đúng hình cây đó — phức tạp hơn hẳn phạm vi mục này, để sau nếu thật sự cần (field lồng nhau không phải trường hợp phổ biến nhất trong dữ liệu Mongo thường gặp).
- Elasticsearch/Redis's `Documents` cũng được hưởng `field_order` miễn phí (cùng cơ chế chung trong `tradar-server`, không phân biệt theo connector) nhưng KHÔNG có bật `preserve_order` riêng cho chúng — nếu chúng xây `Value::Object` theo cách tự nhiên giữ thứ tự sẵn (ví dụ `reqwest`'s `.json::<Value>()` deserialize trực tiếp từ response text) thì vẫn lợi, nếu không thì `field_order` của chúng chỉ phản ánh lại đúng thứ tự `BTreeMap` cũ (không sai, chỉ không có ích thêm) — chưa verify riêng cho hai driver này, ngoài phạm vi "Mongo field order" ban đầu.
