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

## HTTP và gRPC: workbench request

HTTP và gRPC dùng **chung một workbench**: mỗi connection một buffer sửa được, chứa nhiều request cách nhau bằng dòng `###`. Đặt con trỏ trong block nào thì `<leader>rr` chạy block đó; response mở trong **result tabs** như kết quả của các DB khác (một tab cho mỗi method + path, `[R`/`]R` xem lần chạy trước, `<leader>rD` so hai lần, `<leader>rR` gửi lại).

| Lệnh | Việc |
|---|---|
| `:TradarRequests [tên]` | mở workbench của một connection HTTP hoặc gRPC (đang đứng trong workbench thì chạy block) |
| `:TradarHttpSend [tên]` / `:TradarGrpcSend [tên]` | như trên, giới hạn theo driver |
| `:TradarHttpConnect [tên]` | ghi nhận connection HTTP (không gọi ra ngoài, nên không bao giờ lỗi) |

### Cú pháp

```
@token = abc                          # biến, dùng bằng {{token}}

### list users                        # ### mở một block (tiêu đề tuỳ chọn)
GET /users?limit=10                   # METHOD url (chỉ có url = GET)
Authorization: Bearer {{token}}

### create
POST /users
Content-Type: application/json

{"name": "ann"}

### get user
GRPC users.UserService/GetUser        # GRPC service/Method
x-token: {{token}}                    # metadata
deadline: 5s                          # deadline của call, không gửi như header

{"id": "42"}
```

- **Block**: bắt đầu bằng `###`; phần trước `###` đầu tiên cũng là một block (chỗ để khai báo biến và ghi chú). Dòng `#` và `//` trước dòng request là comment.
- **HTTP**: dòng đầu `METHOD url` (`GET /x HTTP/1.1` cũng được; url `http://`/`https://` dùng nguyên văn, còn lại nối vào `target` của connection). Các dòng `Tên: giá trị` là header, đến dòng trống đầu tiên; từ đó trở xuống là body, giữ nguyên văn. Body dính ngay dòng request (thiếu dòng trống) sẽ báo lỗi kèm số dòng.
- **gRPC**: dòng đầu `GRPC service/Method`; `tên: giá trị` là metadata (tên tự chuyển chữ thường); `deadline: 5s` (`500ms`, `2m`, hoặc số trần = giây) đặt deadline; body là JSON, dòng `//` bị bỏ khi gửi (đó là gợi ý của skeleton). Không có body = message rỗng.
- **Biến**: `@tên = giá trị` ở bất kỳ đâu trong file (giá trị có thể dùng biến khai báo trước đó); `{{tên}}` được thay trong dòng request, header/metadata và body. Có sẵn `{{$uuid}}`, `{{$timestamp}}` (giây), `{{$isoTimestamp}}`, `{{$env.TÊN}}` (biến môi trường). Biến chưa khai báo thì báo rõ tên, không gửi gì.
- Workbench là scratch buffer, giữ nội dung trong phiên Neovim; `:w đường-dẫn` lưu ra file.

### Phím trong workbench

| Phím | Việc |
|---|---|
| `<leader>rr` | chạy block dưới con trỏ |
| `<leader>rx` | HTTP: ngừng chờ response; gRPC unary: huỷ call; stream: dừng |
| `<leader>rc` | client/bidi stream: đóng chiều gửi |
| `<leader>ry` | copy block thành lệnh `curl` / `grpcurl` |
| `g?` | xem phím |

### Response

- Response mở ở result tabs; tab đặt tên theo `METHOD /path` (query string không tách tab) hoặc tên method gRPC, thanh tab hiện mã trạng thái. Chạy lại cùng request thì thay tab đó và dồn lần trước vào history (`[R`/`]R`, `setup{ results_history }`).
- Body JSON được thụt lề **giữ nguyên thứ tự key và chữ số** như server trả, tô màu như kết quả JSON của DB khác; body không phải JSON hiện nguyên văn. Footer ghi status, thời gian, số byte.
- `<leader>rH` hiện/ẩn dòng trạng thái và header phía trên body (gRPC: header của response).
- `<leader>rD` so response với lần chạy trước (hoặc tab khác) bằng diff theo dòng; `<leader>rR` gửi lại đúng request đó tại chỗ. Mã 4xx/5xx vẫn là response bình thường, không phải lỗi.
- `gy…` ở tab response chép toàn bộ body; `:TradarExport` ghi ra file.

### gRPC

`target` dạng `host:port` hoặc `http://…` là plaintext; `https://host:port` bật **TLS** (chứng chỉ server được kiểm tra theo bộ root CA webpki đi kèm). Tuỳ chọn TLS đặt ngay trong query của `target`: `?ca=/đường/dẫn/ca.pem` thêm CA riêng, `&cert=…&key=…` gửi client certificate (mTLS, phải đi cùng nhau), `&domain=tên` đổi tên dùng để kiểm tra chứng chỉ (hữu ích khi dial theo IP); ví dụ `https://10.0.0.5:443?ca=/etc/ca.pem&cert=/me.pem&key=/me.key&domain=api.internal`. Các tham số này chỉ dùng được với `https://`. Hỗ trợ **unary, server-streaming, client-streaming và bidi**, metadata và deadline.

| Lệnh | Việc |
|---|---|
| `:TradarGrpcConnect [tên]` | kết nối, thử server reflection |
| `:TradarGrpcServices` | liệt kê service đã biết, chọn service rồi method để **thêm block** vào workbench |
| `:TradarGrpcMethods <service>` | liệt kê method của một service |
| `:TradarGrpcLoadProto <đường-dẫn>` | nạp file `.proto` khi server không bật reflection |

Chọn một method thì block của nó được thêm vào cuối workbench, đã **điền sẵn khung request** sinh từ message đầu vào (mọi trường ở giá trị mặc định: `""`, `0`, `false`, giá trị enum đầu tiên, `[]`, `{}`, message lồng nhau được điền; `oneof` chỉ có nhánh đầu — đổi sang nhánh bạn muốn; message tự tham chiếu dừng ở tầng hai) và tiêu đề ghi `[server stream]`/`[client stream]`/`[bidi stream]` nếu có. Chọn lại method đã có thì nhảy tới block cũ thay vì thêm bản sao. Nếu server không hỗ trợ reflection, `:TradarGrpcServices` trả rỗng cho tới khi bạn `:TradarGrpcLoadProto`.

**Gợi ý trong khung request.** Khung sinh ra có các dòng `// …` phía trên những trường cần chú thích: comment trong `.proto` (chỉ có khi nạp bằng `:TradarGrpcLoadProto`; server reflection không mang comment), các giá trị của enum (`// enum Level: LEVEL_UNSET | LEVEL_HIGH`), các nhánh của `oneof`. Dòng nào bắt đầu bằng `//` đều bị bỏ khi gửi.

**Streaming.** Response là một transcript chỉ-đọc trong result tab (`‹` reply, `›` message đã gửi, `#` header/trailer/trạng thái), tự cuộn theo khi con trỏ ở cuối, giữ 5000 dòng gần nhất; footer ghi `streaming` cho đến khi stream kết thúc:

| Loại | `rr` | `rc` | `rx` |
|---|---|---|---|
| server-streaming | bắt đầu call với JSON trong block; replies đổ về tới khi server kết thúc | — | dừng |
| client-streaming | lần đầu bắt đầu call và gửi JSON làm message đầu; mỗi lần `rr` tiếp theo gửi thêm một message (sửa JSON trước) | đóng chiều gửi, server trả reply duy nhất | dừng |
| bidi | như client-streaming, nhưng reply về ngay khi có | đóng chiều gửi (reply còn lại vẫn về) | dừng |

Đóng workbench (wipe buffer) sẽ dừng các stream đang mở. Ghi chú: key metadata `-bin` (giá trị nhị phân) chưa hỗ trợ.

## Socket (raw TCP, kiểu netcat)

`:TradarSocketTail [tên]` — không có bước connect riêng, lệnh này vừa kết nối vừa mở transcript. Buffer **sửa được và hai chiều**: dòng cuối cùng là dòng nhập (như prompt REPL): gõ rồi `<CR>` (normal hoặc insert) để gửi và xoá dòng nhập; dòng gửi/nhận luôn chèn phía trên nó. Giữ 5000 dòng gần nhất. `›` đánh dấu dòng đã gửi, `‹` dòng nhận được. Nội dung nhị phân hiện hex preview. `q` đóng kết nối TCP và buffer. Gọi lại cùng connection mở một phiên TCP độc lập khác. Chưa hỗ trợ UDP hay TLS.

