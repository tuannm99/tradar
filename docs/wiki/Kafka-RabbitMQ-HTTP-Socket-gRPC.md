# Kafka, RabbitMQ, HTTP, Socket, gRPC

Các backend này **không có ngôn ngữ query**, nên không có buffer nguồn gắn connection như SQL. Mỗi cái là một nhóm lệnh `:Tradar…`; connection vẫn khai báo trong [`connections.toml`](Ket-noi) với đúng `driver`. Lệnh bỏ trống tên connection sẽ hỏi qua picker (chỉ liệt kê connection có đúng `driver`). Trong mọi buffer kết quả của nhóm này, `q` đóng và `g?` xem phím.

## Kafka

Cần `tradar-server` build với `--features kafka` ([Cài đặt](Cai-dat)).

| Lệnh | Việc |
|---|---|
| `:TradarKafkaConnect [tên]` | kết nối (kiểm tra metadata broker ngay) |
| `:TradarKafkaTail [tên] [topic]` | tail realtime một topic |

`:TradarKafkaTail` bỏ trống phần nào thì hỏi phần đó, và luôn hỏi tail từ `latest` hay `earliest`. Kết quả là một buffer cuộn trong tab mới, **không sửa được**; message mới tự nối vào cuối — con trỏ chỉ bám theo khi đang đứng ở dòng cuối (cuộn lên đọc thì buffer đứng yên), và chỉ giữ 5000 dòng gần nhất. `q` dừng tail và đóng. Mỗi lần tail là một consumer riêng dùng-một-lần (không join consumer group thật) — tail hai lần cùng topic cho hai luồng độc lập, không chia đôi dữ liệu. Topic nội bộ `__*` bị ẩn.

## RabbitMQ

Dùng Management HTTP API (không phải AMQP), nên mỗi thao tác là một lần gọi, không có tail realtime.

| Lệnh | Việc |
|---|---|
| `:TradarRabbitmqConnect [tên]` | kết nối |
| `:TradarRabbitmqPeek [tên] [queue]` | xem message gần nhất của queue — **không tiêu huỷ** (requeue ngay sau khi đọc) |
| `:TradarRabbitmqBindings [tên] [exchange]` | xem binding của exchange (exchange mặc định không tên bị ẩn) |
| `:TradarRabbitmqPublish [tên]` | hỏi lần lượt exchange (bỏ trống = default exchange), routing key, payload |

Bỏ trống `queue`/`exchange` thì chọn qua picker (queue kèm số `ready`/`unacked`/`consumers`). Kết quả hiện trong một split dùng lại được; message `redelivered` được đánh dấu.

## HTTP

Client kiểu Postman, từng bước một.

| Lệnh | Việc |
|---|---|
| `:TradarHttpConnect [tên]` | ghi nhận connection (không gọi ra ngoài, nên không bao giờ lỗi) |
| `:TradarHttpSend [tên]` | hỏi lần lượt method, URL, headers, body rồi gửi |

- URL bắt đầu bằng `http://` hoặc `https://` dùng nguyên văn; còn lại được nối vào `target` của connection làm base.
- Headers gõ trên **một dòng**, phân tách bằng `;`: `Authorization: Bearer abc; Accept: application/json`. Bỏ trống = không có header.
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

Chọn method mở một buffer JSON trong tab mới, nội dung `{}` — gõ request vào đó. Trong buffer request: `<leader>rr` hoặc `:TradarGrpcSend` gọi thật (response hiện ở split riêng), `<leader>rx` hoặc `:TradarGrpcCancel` huỷ, `g?` xem phím. Buffer request không nhận `q` (đóng bằng `:q`/`:tabclose`). Chọn lại đúng connection/service/method đã mở sẽ nhảy về tab cũ. Chưa có skeleton request tự sinh theo shape message.

Nếu server không hỗ trợ reflection, `:TradarGrpcServices` trả rỗng cho tới khi bạn `:TradarGrpcLoadProto`.
