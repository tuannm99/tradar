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
````

Dòng mở fence là `` ```<dialect> [connection] ``. Thiếu `connection` thì block rơi về connection của buffer (dòng `-- tradar:`, file `.tradar` hoặc `:TradarConnect`).

## Hành vi

- `rr`, `K`, `gd`, `re` và completion áp dụng cho **block đang chứa con trỏ**, không phải cả buffer. Con trỏ đứng trên **dòng fence** (``` mở hoặc đóng) của block cũng chạy block đó; đặt con trỏ ở giữa các block (văn bản thường) thì `rr` báo rõ thay vì chạy nhầm.
- Chỉ **text của block đó** được gửi lên server — một block SQL cạnh một block Mongo trong cùng buffer không làm hỏng nhau.
- `ra` (chạy cả file), `rx` (huỷ) và `rr` ở visual mode **chưa** theo block — vẫn dùng connection của buffer.
- Tô màu dùng parser Markdown: `sql` tô đúng cú pháp, `mongo`/`esq` tô như JavaScript/JSON, `redis` hiện plain.
- Completion theo đúng dialect của block. Chưa có LSP thật theo từng block.
