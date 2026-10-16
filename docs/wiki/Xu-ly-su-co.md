# Xử lý sự cố

Bước đầu tiên luôn là:

```vim
:checkhealth tradar
```

Nó cho biết binary `tradar-server` có tìm thấy không, server có đang chạy không, **phiên bản protocol có khớp plugin không**, bao nhiêu connection đã lưu, và các plugin tuỳ chọn có sẵn không.

## Plugin cảnh báo "tradar-server speaks protocol vN, this plugin expects vM"

Plugin và `tradar-server` đang đến từ hai bản khác nhau (thường là một `tradar-server` cũ nằm trên `PATH`, trong khi plugin trỏ vào checkout mới). Build lại và khởi động lại:

```bash
cargo build --release -p tradar-server
```
```vim
:TradarRestart
```

Cảnh báo này chỉ nhắc, không chặn bạn dùng tiếp. Muốn chắc plugin dùng đúng binary, đặt `setup{ server_cmd = "/đường/dẫn/tradar-server" }`.

## Build lại server rồi mà hành vi vẫn cũ

Server chạy nền và giữ nguyên binary cũ cho tới khi dừng. Chạy `:TradarRestart`; lệnh kế tiếp sẽ tự khởi động bản mới.

## "cannot reach tradar-server" / "no tradar-server binary found"

Plugin tìm binary theo thứ tự: `server_cmd` → `PATH` → `<repo>/target/release` → `<repo>/target/debug`. Chưa build thì xem [Cài đặt](Cai-dat). Nếu bạn tắt `autostart`, hãy tự chạy `tradar-server`.

## "a tradar-server is already listening on …"

Đã có một server đang chạy trên socket đó — bình thường, plugin sẽ dùng lại nó. Nếu socket còn sót lại sau khi server chết đột ngột, lần khởi động kế tiếp tự dọn.

## Đường dẫn socket quá dài

Unix socket giới hạn khoảng 108 byte. Nếu `$XDG_RUNTIME_DIR` dài, `:checkhealth` sẽ cảnh báo; đặt `setup{ socket = "/tmp/tradar.sock" }` (đường dẫn ngắn hơn).

## "`tên` is not connected -- call `connect` first"

Connection chưa được kết nối. Mở lại file có dòng `-- tradar: tên` (plugin tự connect nền), hoặc `:TradarConnect tên`. Với Kafka/RabbitMQ/HTTP/gRPC dùng lệnh `…Connect` tương ứng ([trang này](Kafka-RabbitMQ-HTTP-Socket-gRPC)).

## "no saved … connection named …"

Tên connection không có trong `~/.config/tradar/connections.toml`, hoặc có nhưng `driver` khác loại bạn đang gọi (vd gọi `:TradarKafkaTail` với connection `driver = "postgres"`). Kiểm tra lại tên và `driver` ([Kết nối](Ket-noi)).

## Lệnh Kafka báo `unknown method 'kafka.…'`

`tradar-server` của bạn build không có Kafka. Build lại với `--features kafka` ([Cài đặt](Cai-dat)).

## Không sửa được ô / xoá được dòng trong kết quả

Kết quả chỉ sửa được khi đến từ **một bảng có khoá chính** và cột khoá có trong kết quả. Plugin sẽ nói rõ lý do cụ thể. Với kết quả `JOIN` nhiều bảng thì không sửa được.

## Kết quả bị cắt

Kết quả SQL/CQL và Mongo bị giới hạn số dòng đọc về để không nạp cả bảng khổng lồ vào RAM (Mongo `find`/`aggregate` dừng ở 10.000 document và báo "truncated"). Thêm `LIMIT`/bộ lọc, hoặc dùng `<leader>rm` để tải thêm trang.

## TUI: cấu hình không có tác dụng

`config.toml` hỏng thì TUI in cảnh báo ra stderr rồi dùng mặc định. `vim-mode` chỉ áp dụng sau khi khởi động lại.

## Vẫn không được?

Mở issue tại https://github.com/tuannm99/tradar/issues kèm kết quả `:checkhealth tradar` và phiên bản (`tradar --version`).
