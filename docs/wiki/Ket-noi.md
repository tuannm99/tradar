# Kết nối

Mọi connection nằm trong `~/.config/tradar/connections.toml` — **cùng một file** cho TUI và Neovim. Mỗi connection là một bảng `[[connections]]` với ba trường:

```toml
[[connections]]
name = "local postgres"          # tên bạn đặt, dùng để gắn file / chọn trong picker
driver = "postgres"              # id của connector (bảng dưới)
target = "postgres://user:password@localhost:5432/mydb"
```

**Trong Neovim, cách dễ nhất:** `:TradarAddConnection` (hoặc chọn `+ add a connection…` trong `:TradarConnect`; chạy `:TradarConnect` lần đầu khi chưa có connection nào thì tự vào đây). Plugin hỏi lần lượt loại database → tên → target (điền sẵn mẫu đúng dạng của loại đó, kèm lưu ý như mã hoá phần trăm mật khẩu), ghi thêm một bảng `[[connections]]` vào cuối file (không đụng vào comment hay connection khác) rồi **thử kết nối ngay**. Không kết nối được thì hỏi: *Edit the target* (sửa lại, điền sẵn target vừa nhập), *Keep it anyway* hoặc *Remove it* (trả file về đúng như cũ). Socket không được thử (nó chỉ nối khi bạn tail).

Thêm/sửa/xoá được ngay trong connection picker của TUI (`a`/`e`/`d`), hoặc sửa tay file này — trong Neovim gõ `:TradarConnections` (hoặc chọn dòng `+ edit connections file…` cuối picker của `:TradarConnect`) để mở đúng file mà server đang đọc; file mới thì có sẵn template comment. Server đọc lại file mỗi lần dùng nên chỉ cần `:w`, và mỗi lần lưu plugin kiểm tra ngay: đếm số connection, hoặc báo lỗi TOML kèm dòng/cột. Lưu ý: khi lưu từ TUI, file bị ghi lại theo định dạng TOML chuẩn, nên comment và thứ tự key bạn viết tay sẽ mất.

## `driver` và `target`

| `driver` | `target` |
|---|---|
| `postgres` | `postgres://user:password@host:5432/db` |
| `mysql` (cả MariaDB) | `mysql://user:password@host:3306/db` |
| `sqlite` | đường dẫn file, vd `test.db` |
| `mongo` | `mongodb://host:27017/db` — phần `/db` không bắt buộc (mặc định `test`, như `mongosh`) |
| `elasticsearch` | `http://host:9200` |
| `redis` | `redis://host:6379/0` |
| `cassandra` | `host:port` trần, vd `127.0.0.1:9042` (không phải URI) |
| `clickhouse` | `http://[user[:password]@]host[:port][/database]` — **cổng HTTP** (8123, hoặc `https://…:8443` với ClickHouse Cloud), không phải cổng native 9000; ký tự đặc biệt trong user/password phải mã hoá phần trăm (`@`→`%40`, `:`→`%3A`, `/`→`%2F`, `#`→`%23`, khoảng trắng→`%20`, `%`→`%25`) |
| `kafka` | bootstrap servers: `host:port[,host:port...]` |
| `rabbitmq` | URL tới Management API kèm credentials: `http://user:pass@host:15672/%2f` (`%2f` là vhost mặc định `/`) |
| `http` | base URL tuỳ chọn (có thể để trống nếu luôn gõ URL đầy đủ); thêm `?insecure=1` để **không kiểm tra chứng chỉ TLS** (server tự ký) |
| `socket` | `host:port` trần |
| `grpc` | `host:port` trần hoặc URL đầy đủ, `https://…` bật TLS (root CA webpki; thêm `?ca=…&cert=…&key=…&domain=…` cho CA riêng/mTLS, hoặc `?insecure=1` để không kiểm tra chứng chỉ — xem trang gRPC), còn lại plaintext |

Chỉ những `driver` mà bản `tradar-server`/`tradar` của bạn đã compile vào mới dùng được — xem [Cài đặt](Cai-dat).

## Gắn một file với connection (Neovim)

Có ba cách, theo thứ tự bạn sẽ dùng:

1. **Dòng đầu file** (trong 10 dòng đầu), dấu comment tuỳ ngôn ngữ:
   ```
   -- tradar: tên-connection     (SQL)
   // tradar: tên-connection     (Mongo)
   # tradar: tên-connection      (Redis / Elasticsearch)
   ```
2. **File `.tradar`** (một dòng, chỉ chứa tên connection) ở thư mục cha — áp dụng cho mọi file bên dưới.
3. **`:TradarConnect [tên]`** — gắn riêng buffer hiện tại. Bỏ trống tên thì mở **picker telescope** (có telescope): mỗi connection một dòng (`●` đang kết nối, `○` chưa, `✗` server không phục vụ loại này) kèm khung xem trước — loại, target (mật khẩu thay bằng `***`), trạng thái (đang kết nối, idle, đang trong transaction…). `<CR>` chọn, `<C-a>` thêm connection, `<C-d>` ngắt kết nối đang chọn (hỏi lại; kết quả đã tải vẫn giữ, lần dùng sau tự kết nối lại để giải phóng pool và bộ nhớ), `<C-e>` mở file. Không có telescope, hoặc `setup{ connection_picker = 'ui' }`, thì dùng danh sách `vim.ui.select`.

Mở file là plugin tự connect nền, không cần gõ lệnh.
