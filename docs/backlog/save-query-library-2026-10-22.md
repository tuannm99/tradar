# Lưu query: thư viện chung vs cạnh file (2026-10-22)

User muốn hai chế độ lưu — vào một "root" của tradar để mở lại tập trung (telescope), hoặc lưu theo file/đường dẫn hiện tại — chưa chốt UX.

Chốt tạm (tách thành hai lệnh thay vì một lệnh có cờ, để mỗi lệnh có một nghĩa duy nhất):
- `:TradarSave` → thư viện = `queries_dir`, mặc định `<config>/tradar/queries` — **cùng thư mục TUI** (`default_queries_dir`) nên query hai bên thấy nhau. Luật tên giống `resolve_query_path` của TUI (tên trần → `.sql`, `/`, `~`, `./`, `../` giữ nguyên) cộng thêm: từ chối `..` giữa tên thư viện.
- `:TradarSaveHere` → cạnh file hiện tại / cwd; buffer đã có file thì chỉ `:write`.
- `:TradarOpen` → telescope (mới sửa nhất đầu, preview file, `<C-s>` split, `<C-x>` xoá có hỏi), không telescope → `vim.ui.select`.
- Không ghi đè nếu file đã có, trừ `!`. Lưu buffer đã có file vào thư viện là **bản sao** (buffer giữ file cũ); buffer chưa có tên thì thành file đó.
- Buffer đã `:TradarConnect` thì ghi `-- tradar: <conn>` (`//` cho mongo/ES, `#` cho redis) vào đầu file khi lưu, vì liên kết `vim.b.tradar_connection` mất khi mở lại; `.tdb` bỏ qua (connection nằm ở dòng fence).
- Đặt tên trùng với buffer khác đang mở file đó: wipe buffer kia nếu chưa sửa, còn đã sửa thì từ chối (không thì E95).

Chưa làm (chờ dùng thử rồi quyết): tìm theo nội dung (`live_grep` trong thư viện), thẻ/nhóm theo connection, thư viện riêng theo project (`.tradar` đã có để gắn connection).

Test: kịch bản headless 20 kiểm tra (resolve tên, folder tự tạo, không ghi đè, `!`, bản sao giữ tên buffer, modeline connection, tên cạnh file / cwd, prompt khi bỏ trống, liệt kê đệ quy, `:TradarOpen` fallback).
