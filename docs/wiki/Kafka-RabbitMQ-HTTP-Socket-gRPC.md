# Kafka, RabbitMQ, HTTP, Socket, gRPC

Các backend này **không có ngôn ngữ query**, nên không có buffer nguồn gắn connection như SQL. Mỗi cái là một nhóm lệnh `:Tradar…`; connection vẫn khai báo trong [`connections.toml`](Ket-noi) với đúng `driver`. Lệnh bỏ trống tên connection sẽ hỏi qua picker (chỉ liệt kê connection có đúng `driver`). Trong mọi buffer kết quả của nhóm này, `q` đóng và `g?` xem phím.

## Kafka

Cần `tradar-server` build với `--features kafka` ([Cài đặt](Cai-dat)).

| Lệnh | Việc |
|---|---|
| `:TradarKafkaConnect [tên]` | kết nối (kiểm tra metadata broker ngay) |
| `:TradarKafkaTail [tên] [topic]` | tail realtime một topic |

`:TradarKafkaTail` bỏ trống phần nào thì hỏi phần đó, và luôn hỏi tail từ `latest` hay `earliest`. Kết quả là một buffer cuộn trong tab mới, **không sửa được**; message mới tự nối vào cuối — con trỏ chỉ bám theo khi đang đứng ở dòng cuối (cuộn lên đọc thì buffer đứng yên), và chỉ giữ 5000 dòng gần nhất. `q` dừng tail và đóng. `p` tạm dừng/tiếp tục (tin vẫn về, hiện khi tiếp tục), `f` lọc theo chuỗi con (không phân biệt hoa thường), `c` xoá buffer, `g?` xem phím; thanh winbar phía trên hiện connection/topic, `PAUSED`, bộ lọc đang bật. Nếu server khởi động lại/mất kết nối, tail tự đăng ký lại từ `latest` một lần (ghi chú vào transcript; tin trong lúc mất không phát lại), thất bại thì winbar báo `DISCONNECTED` và `R` thử lại. Mỗi lần tail là một consumer riêng dùng-một-lần (không join consumer group thật) — tail hai lần cùng topic cho hai luồng độc lập, không chia đôi dữ liệu. Topic nội bộ `__*` bị ẩn.

## RabbitMQ

Dùng Management HTTP API (không phải AMQP), nên mỗi thao tác là một lần gọi, không có tail realtime.

| Lệnh | Việc |
|---|---|
| `:TradarRabbitmqConnect [tên]` | kết nối |
| `:TradarRabbitmqPeek [tên] [queue]` | xem message gần nhất của queue — **không tiêu huỷ** (requeue ngay sau khi đọc) |
| `:TradarRabbitmqBindings [tên] [exchange]` | xem binding của exchange (exchange mặc định không tên bị ẩn) |
| `:TradarRabbitmqPublish [tên]` | mở buffer publish sửa được (trong buffer thì publish luôn); `<leader>rr` cũng publish |

Bỏ trống `queue`/`exchange` thì chọn qua picker (queue kèm số `ready`/`unacked`/`consumers`). Kết quả hiện trong một split dùng lại được; message `redelivered` được đánh dấu. Trong split đó `r` làm mới (peek lại / liệt kê lại bindings).

Buffer publish:

```
exchange:
routing-key: orders

{"id": 1}
```

`exchange:` bỏ trống = default exchange (khi đó `routing-key:` là tên queue); mọi thứ sau dòng trống là payload, nhiều dòng thoải mái. Mỗi connection một buffer, giữ nội dung để sửa nhẹ rồi gửi lại.

## HTTP

Client kiểu Postman: request là một **buffer sửa được**.

| Lệnh | Việc |
|---|---|
| `:TradarHttpConnect [tên]` | ghi nhận connection (không gọi ra ngoài, nên không bao giờ lỗi) |
| `:TradarHttpSend [tên]` | mở buffer request của connection (đang đứng trong buffer request thì gửi luôn); `<leader>rr` cũng gửi |

```
# comment (trước dòng trống thì bị bỏ qua)
POST /users
Content-Type: application/json

{"name": "ann"}
```

- Dòng đầu: `METHOD url` (chỉ có URL = `GET`). URL bắt đầu bằng `http://`/`https://` dùng nguyên văn; còn lại được nối vào `target` của connection làm base.
- Các dòng `Key: Value` tiếp theo là headers, đến dòng trống đầu tiên; từ đó trở xuống là body (nhiều dòng thoải mái).
- Mỗi connection có một buffer request riêng, giữ nguyên nội dung trong phiên Neovim để sửa nhẹ rồi gửi lại. Buffer sửa được nên `q` không bị chiếm — đóng bằng `:q`/`:tabclose`.
- Response (status, headers, body) hiện trong split `tradar://http`, dùng lại cho mọi lần gửi.

## Socket (raw TCP, kiểu netcat)

`:TradarSocketTail [tên]` — không có bước connect riêng, lệnh này vừa kết nối vừa mở transcript. Buffer **sửa được và hai chiều**: dòng cuối cùng là dòng nhập (như prompt REPL): gõ rồi `<CR>` (normal hoặc insert) để gửi và xoá dòng nhập; dòng gửi/nhận luôn chèn phía trên nó. Giữ 5000 dòng gần nhất. `›` đánh dấu dòng đã gửi, `‹` dòng nhận được. Nội dung nhị phân hiện hex preview. `q` đóng kết nối TCP và buffer. Gọi lại cùng connection mở một phiên TCP độc lập khác. Chưa hỗ trợ UDP hay TLS.

## gRPC

Chỉ **unary**, plaintext, chưa có metadata/header request.

| Lệnh | Việc |
|---|---|
| `:TradarGrpcConnect [tên]` | kết nối, thử server reflection |
| `:TradarGrpcServices` | liệt kê service đã biết, chọn service rồi method để mở buffer request |
| `:TradarGrpcMethods <service>` | liệt kê method của một service |
| `:TradarGrpcLoadProto <đường-dẫn>` | nạp file `.proto` khi server không bật reflection |

Chọn method mở một buffer JSON trong tab mới, nội dung `{}` — gõ request vào đó. Trong buffer request: `<leader>rr` hoặc `:TradarGrpcSend` gọi thật (response hiện ở split riêng, JSON thụt lề), `<leader>rx` hoặc `:TradarGrpcCancel` huỷ, `g?` xem phím. Buffer request không nhận `q` (đóng bằng `:q`/`:tabclose`). Chọn lại đúng connection/service/method đã mở sẽ nhảy về tab cũ. Chưa có skeleton request tự sinh theo shape message.

Nếu server không hỗ trợ reflection, `:TradarGrpcServices` trả rỗng cho tới khi bạn `:TradarGrpcLoadProto`.
