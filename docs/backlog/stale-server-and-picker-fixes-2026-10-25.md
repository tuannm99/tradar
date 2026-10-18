# Server cũ chạy ngầm (comment Mongo "vẫn lỗi"), picker trong `.tdb`

## Báo cáo "comment trong query Mongo vẫn không chạy"

Query `db.users.find({ // _id: ObjectId("…") })` vẫn báo `invalid JSON argument: key must be a string at line 2 column 3` sau khi bản sửa đã merge. Server truyền nguyên văn query tới `MongoDriver::execute`, và unit test dùng đúng câu đó pass → binary đang chạy là bản cũ. Hai cơ chế khiến điều này im lặng:

1. `tradar-server` chạy nền, sống lâu hơn phiên Neovim, giữ binary cũ đến khi `:TradarRestart`.
2. `rpc.server_cmd` ưu tiên `tradar-server` trên `PATH` hơn bản build cạnh plugin, nên một bản cài cũ che bản vừa `cargo build`.

Sửa (để lần sau không phải đoán): `version` trả thêm `exe` (binary đang chạy) và `stale` (binary trên đĩa đổi sau khi server khởi động — mtime mới hơn, hoặc `/proc/self/exe` có hậu tố ` (deleted)` khi cargo thay file); protocol `1.3`. Plugin cảnh báo một lần khi `stale`; `:checkhealth tradar` hiện binary đang chạy, cảnh báo `stale`, và cảnh báo khi bản trên `PATH` cũ hơn bản build trong repo. Kiểm chứng: server chạy từ một bản copy, thay file như cargo (new inode) → `stale: false` → `true`, plugin in cảnh báo.

## Rà UX picker/completion

- Picker tables (`:TradarTables`) và `open_table`/`gd` dùng connection của *buffer*, bỏ qua connection ghi trên fence của block `.tdb` → hỏi connect picker hoặc mở nhầm connection. Nay dùng `block_connection` theo vị trí con trỏ.
- Lịch sử `<C-r>` (chạy ngay) nối câu vào *cuối file* — trong `.tdb` đó nằm ngoài mọi block nên không chạy. Nay chèn ngay dưới con trỏ, như `<CR>`.
- Completion (blink) đã theo block; không có vấn đề.

## Rà tiếp (cùng đợt)

- Panel schema: thêm `o` (mở bảng dưới con trỏ — trước đó phải thoát ra dùng `gd`), `r` (làm mới schema — panel đang mở không thấy bảng mới), winbar `schema · <connection>`; `render.schema_tree` gắn `entry` vào node bảng.
- Preview lịch sử tô theo dialect của câu (`mongo`/`esq`/`redis`/`sql`) qua `tradar.cached_driver`.
- Viết `docs/wiki/Phim-tat.md`: bảng mọi phím/lệnh theo từng loại buffer (query, kết quả, xem trước commit, schema, telescope, Kafka/Socket/RabbitMQ/HTTP/gRPC) — nguồn là các `keymap.set` trong `nvim/lua/tradar/*.lua`; `setup{ keymaps = false }` chỉ tắt phím buffer query, nay ghi rõ.

## Chưa rà

gRPC (service/method picker, response), Redis, statusline.
