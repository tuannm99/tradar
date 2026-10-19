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

Server chạy nền và giữ nguyên binary cũ cho tới khi dừng. Chạy `:TradarRestart`; lệnh kế tiếp sẽ tự khởi động bản mới. Plugin tự phát hiện: nếu binary trên đĩa đã được build lại sau khi server khởi động, lần kết nối kế tiếp báo "the running tradar-server … is older than the binary on disk" (và `:checkhealth tradar` cũng báo, kèm đường dẫn binary đang chạy).

Nếu đã restart mà vẫn cũ: plugin chọn `server_cmd`, rồi **`tradar-server` trên `PATH`**, rồi mới tới `target/release|debug` trong repo — một bản cài cũ trên `PATH` (vd. `~/.cargo/bin`) che mất bản vừa build. `:checkhealth tradar` cảnh báo khi bản trên `PATH` cũ hơn bản build trong repo; cách xử lý: copy bản mới đè lên, bỏ nó khỏi `PATH`, hoặc đặt `setup{ server_cmd = "…/target/release/tradar-server" }`.

## "cannot reach tradar-server" / "no tradar-server binary found"

Plugin tìm binary theo thứ tự: `server_cmd` → `PATH` → `<repo>/target/release` → `<repo>/target/debug`. Chưa build thì xem [Cài đặt](Cai-dat). Nếu bạn tắt `autostart`, hãy tự chạy `tradar-server`.

## "a tradar-server is already listening on …"

Đã có một server đang chạy trên socket đó — bình thường, plugin sẽ dùng lại nó. Nếu socket còn sót lại sau khi server chết đột ngột, lần khởi động kế tiếp tự dọn.

## Đường dẫn socket quá dài

Unix socket giới hạn khoảng 108 byte. Nếu `$XDG_RUNTIME_DIR` dài, `:checkhealth` sẽ cảnh báo; đặt `setup{ socket = "/tmp/tradar.sock" }` (đường dẫn ngắn hơn).

## "`tên` is not connected -- call `connect` first"

Plugin tự khôi phục trường hợp thường gặp nhất: server bị dừng/khởi động lại sau khi bạn đã kết nối — lệnh kế tiếp tự kết nối lại và lặp lại một lần, không cần gõ lại. Chỉ khi connection chưa từng được kết nối trong phiên này (hoặc kết nối lại cũng thất bại) bạn mới thấy thông báo này; khi đó mở lại file có dòng `-- tradar: tên`, hoặc `:TradarConnect tên`.

Connection chưa được kết nối. Mở lại file có dòng `-- tradar: tên` (plugin tự connect nền), hoặc `:TradarConnect tên`. Với Kafka/RabbitMQ/HTTP/gRPC dùng lệnh `…Connect` tương ứng ([trang này](Kafka-RabbitMQ-HTTP-Socket-gRPC)).

## "no saved … connection named …"

Tên connection không có trong `~/.config/tradar/connections.toml`, hoặc có nhưng `driver` khác loại bạn đang gọi (vd gọi `:TradarKafkaTail` với connection `driver = "postgres"`). Kiểm tra lại tên và `driver` ([Kết nối](Ket-noi)).

## Lệnh Kafka báo `unknown method 'kafka.…'`

`tradar-server` của bạn build không có Kafka. Build lại với `--features kafka` ([Cài đặt](Cai-dat)).

## File `.tdb` không tô màu, hoặc `rr` gửi cả dòng ``` lên server

Filetype của file không phải `tdb` (gõ `:set ft?`). Thường gặp khi plugin load muộn hơn lúc buffer được đọc: Neovim tự đoán `.tdb` là `conf` (nhất là khi dòng đầu bắt đầu bằng `#`). Plugin đã tự sửa (thư mục `ftdetect/`, và tự đặt lại filetype cho buffer đang mở khi plugin load), tự bật highlight markdown (treesitter, không có parser thì dùng syntax markdown) và nhận `.tdb` theo đuôi file kể cả khi filetype bị ghi đè — nếu vẫn thấy sai, kiểm tra `:verbose set ft?` xem plugin nào đặt nó, và dùng `lazy = false` ([Cài đặt](Cai-dat)). Tô màu đúng cho từng fence còn cần parser treesitter của `markdown` và của dialect trong fence (`sql`, `javascript`, `json`).

## Không sửa được ô / xoá được dòng trong kết quả

Kết quả chỉ sửa được khi đến từ **một bảng có khoá chính** và cột khoá có trong kết quả. Plugin sẽ nói rõ lý do cụ thể. Với kết quả `JOIN` nhiều bảng thì không sửa được.

## Kết quả bị cắt

Kết quả SQL/CQL và Mongo bị giới hạn số dòng đọc về để không nạp cả bảng khổng lồ vào RAM (Mongo `find`/`aggregate` dừng ở 10.000 document và báo "truncated"). Thêm `LIMIT`/bộ lọc, hoặc dùng `<leader>rm` để tải thêm trang.

## TUI: cấu hình không có tác dụng

`config.toml` hỏng thì TUI in cảnh báo ra stderr rồi dùng mặc định. `vim-mode` chỉ áp dụng sau khi khởi động lại.

## Vẫn không được?

Mở issue tại https://github.com/tuannm99/tradar/issues kèm kết quả `:checkhealth tradar` và phiên bản (`tradar --version`).
