# File `.tdb`

Thay vì chia theo đuôi file (`.sql`/`.mongo`/`.redis`/`.esq`), một file `.tdb` chứa nhiều block kiểu fenced code của Markdown. Mỗi block tự chọn **dialect** và (tuỳ chọn) **connection** riêng — tiện để ghi chú và so sánh nhiều truy vấn khác loại cạnh nhau.

````
# ghi chú thoải mái ở ngoài block, không chạy được

```sql pg-local
SELECT * FROM users LIMIT 10;
```

```mongo atlas-dev
db.users.find({active: true})
```

```redis
GET session:123
```

```http web
GET /users?limit=10
Authorization: Bearer {{token}}
```
````

Dòng mở fence là `` ```<dialect> [connection] ``. Thiếu `connection` thì block rơi về connection của buffer (dòng `-- tradar:`, file `.tradar` hoặc `:TradarConnect`).

## Hành vi

- `rr`, `K`, `gd`, `re` và completion áp dụng cho **block đang chứa con trỏ**, không phải cả buffer. Con trỏ đứng trên **dòng fence** (``` mở hoặc đóng) của block cũng chạy block đó; đặt con trỏ ở giữa các block (văn bản thường) thì `rr` báo rõ thay vì chạy nhầm.
- Chỉ **text của block đó** được gửi lên server — một block SQL cạnh một block Mongo trong cùng buffer không làm hỏng nhau.
- `ra` (chạy cả file), `rx` (huỷ) và `rr` ở visual mode **chưa** theo block — vẫn dùng connection của buffer.
- Tô màu dùng parser Markdown: `sql` tô đúng cú pháp, `mongo`/`esq` tô như JavaScript/JSON, `redis` hiện plain.
- Completion theo đúng dialect của block. Chưa có LSP thật theo từng block.

## Request HTTP / gRPC trong `.tdb`

Block ` ```http ` và ` ```grpc ` chứa request thay vì câu truy vấn; cú pháp, biến và phím giống [workbench request](Kafka-RabbitMQ-HTTP-Socket-gRPC). Connection chọn như các block khác:

- **Alias trong fence**: ` ```http web ` — `web` là tên một connection `driver = "http"` trong `connections.toml`; url tương đối (`/users`) được nối vào `target` của nó.
- **Alias của cả buffer**: dòng `# tradar: web` ở đầu file (cùng cú pháp `-- tradar:` của file query); block không ghi connection dùng nó. Khác block SQL, block request **không** rơi về "connection chọn gần nhất" (thường là một database).
- **URL thẳng**: không cần connection nào nếu viết URL đầy đủ — ` ```http ` rồi `GET https://api.example.com/users`. Với gRPC: `GRPC localhost:50051 pkg.Service/Method` (`https://host:port` bật TLS).

`rr` gửi request trong block đang đứng (một block có thể chứa nhiều request cách nhau bằng `###`; request nào chứa con trỏ thì chạy), `rx` huỷ/ngừng chờ, `ry` (hoặc `:TradarCopyCurl`) copy thành `curl`/`grpcurl`, `rk` thêm block mẫu vào trong block đang đứng, `<C-x><C-o>` gợi ý header/biến. Biến `@tên = giá trị` đặt ở **bất kỳ đâu** trong file (kể cả ở phần ghi chú ngoài block) dùng chung cho mọi request của file. Response mở ở result tabs.

Cùng cách này dùng được cho file `.http` (một file chỉ toàn request): đuôi `.http` tự nhận filetype `http`, `# tradar: web` ở đầu file là alias (không có thì chỉ chạy được request có URL đầy đủ).
