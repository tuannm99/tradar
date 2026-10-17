# Kết nối

Mọi connection nằm trong `~/.config/tradar/connections.toml` — **cùng một file** cho TUI và Neovim. Mỗi connection là một bảng `[[connections]]` với ba trường:

```toml
[[connections]]
name = "local postgres"          # tên bạn đặt, dùng để gắn file / chọn trong picker
driver = "postgres"              # id của connector (bảng dưới)
target = "postgres://user:password@localhost:5432/mydb"
```

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
| `clickhouse` | `http://[user[:password]@]host[:port][/database]` |
| `kafka` | bootstrap servers: `host:port[,host:port...]` |
| `rabbitmq` | URL tới Management API kèm credentials: `http://user:pass@host:15672/%2f` (`%2f` là vhost mặc định `/`) |
| `http` | base URL tuỳ chọn (có thể để trống nếu luôn gõ URL đầy đủ) |
| `socket` | `host:port` trần |
| `grpc` | `host:port` trần hoặc URL đầy đủ, plaintext (chưa hỗ trợ TLS) |

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
3. **`:TradarConnect [tên]`** — gắn riêng buffer hiện tại.

Mở file là plugin tự connect nền, không cần gõ lệnh.
