# Quản lý file query (user hỏi 2026-08-14) — xong

Trạng thái trước khi làm: `Ctrl+S`/`Ctrl+O` có rồi nhưng **gõ tay đường dẫn**, `last_path` chỉ nằm trong RAM của một screen (mất khi thoát, không chia sẻ giữa tab). Không có thư mục mặc định, không liệt kê được file đã lưu, không có recent list.

Đã làm đúng như hướng chốt:

- **Thư mục mặc định** `~/.config/tradar/queries/` (dùng chung chỗ với `connections.toml`/`session.toml`/`config.toml`) — `storage::default_queries_dir()`. `storage::resolve_query_path()` quyết định: tên trần (`report`) → `queries/report.sql` (tự thêm `.sql` khi chưa có extension); chuỗi có `/`, `\` hoặc bắt đầu bằng `~` → dùng nguyên văn, để người quen đường dẫn tuyệt đối không bị chặn.
- **Recent list** lưu ra `~/.config/tradar/recent.toml` (`RecentFiles`/`RecentStore`), most-recent-first, dedupe khi mở lại (đẩy lên đầu chứ không nhân đôi), cắt ở `MAX_RECENT = 20`; cập nhật cả khi save lẫn khi open. Recent là **toàn cục**, không thuộc session, nên không nhét vào `session.toml`.
- **`Ctrl+O` mở picker** (`components/file_picker.rs`): recent trước (đánh dấu `●`), rồi các file còn lại trong thư mục queries theo thứ tự tên. Gõ để lọc (không phân biệt hoa thường); không khớp gì thì `Enter` mở luôn chuỗi vừa gõ như một đường dẫn — một widget lo cả hai kiểu dùng. File trong recent mà **không còn tồn tại thì bị loại khỏi danh sách** thay vì hiện ra rồi báo lỗi khi mở. File ngoài thư mục queries hiện đủ đường dẫn (chỉ tên file thì không biết là file nào). Mở lỗi thì **giữ overlay** và hiện lỗi, đóng đi thì không còn dấu vết gì báo là mở hụt.
- **Chỗ đặt state**: `queries_dir` + recent list là **global của process** (`storage::init_query_files` / `storage::query_files`), cùng kiểu với `theme()`/`keymap()`. Lý do: screen được dựng sâu bên trong connector (`Session::build_screen`), luồn xuống tận nơi đồng nghĩa với việc nhét "file để ở đâu" vào SPI connector — thứ chẳng liên quan gì tới việc kết nối database. `main.rs` init một lần lúc khởi động; test không init nên `query_files()` trả `None` và code rơi về nhánh gõ đường dẫn tay như cũ.
- Prompt `Ctrl+S` prefill bằng **tên trần** khi file cuối nằm trong thư mục queries (trước đó prefill cả đường dẫn dài, muốn lưu tên khác thì phải xoá hết).

Còn để lại: chưa có xoá/đổi tên file ngay trong picker, chưa đánh dấu buffer "đã sửa chưa lưu", chưa có `Ctrl+S` ghi đè thẳng không hỏi khi đã biết `last_path`.

