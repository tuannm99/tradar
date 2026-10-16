# Wiki hướng dẫn sử dụng (2026-10-19)

Làm tiếp việc "chi tiết thì để wiki hướng dẫn sử dụng, làm sau" đã chốt khi dọn tài liệu (`docs/architecture.md` chỉ còn kiến trúc hiện tại). Wiki nằm trong repo ở `docs/wiki/` thay vì repo `tradar.wiki.git` riêng: review được qua PR cùng code, và đặt tên file/`_Sidebar.md` theo kiểu GitHub Wiki nên sau này chép sang là xong.

Trang: `Home`, `Cai-dat`, `Ket-noi`, `Truy-van-SQL`, `Mongo-Redis-Elasticsearch`, `File-tdb`, `Kafka-RabbitMQ-HTTP-Socket-gRPC`, `TUI`, `Xu-ly-su-co`.

Nguồn và cách kiểm chứng: nội dung lấy từ `nvim/README.md` và `README.md`, nhưng các điểm dễ sai được đối chiếu lại với code thật trước khi viết — danh sách lệnh `:Tradar*` (`nvim/plugin/tradar.lua`, `grpc.lua`), các tuỳ chọn `setup{}`, bảng phím mặc định của TUI (`tradar-core/src/keymap.rs`), thông báo lỗi thật (`unknown method`, `is not connected`, `no saved … connection named`, `a tradar-server is already listening`). Phát hiện nhờ đó: `README.md` gốc chưa liệt kê đủ phím chạy query của TUI (`F5`, `Ctrl+C`, `Ctrl+R`, `Ctrl+K`, `F7`, `Ctrl+Q`).

Chưa làm: không có ảnh chụp/gif minh hoạ; chưa chạy thử từng bước trên một máy sạch; chưa đẩy sang GitHub Wiki thật (cần repo `tradar.wiki.git`, nằm ngoài phạm vi truy cập của phiên này).
