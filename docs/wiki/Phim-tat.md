# Phím tắt

Mọi phím riêng của tradar, theo từng loại buffer. Trong bất kỳ buffer nào của tradar, **`g?`** mở bảng phím của chính buffer đó.

Quy ước: `<leader>r` là **tiền tố** mặc định, đổi bằng `setup{ prefix = "…" }` ([Cài đặt](Cai-dat#tuỳ-chọn-setup)); bên dưới viết tắt là `r` + phím (ví dụ `rr` = `<leader>rr`). `setup{ keymaps = false }` chỉ tắt phím của **buffer query** (mục 1); phím của các buffer kết quả/panel luôn có. Mọi phím là buffer-local nên không đè phím toàn cục của bạn.

## 1. Buffer query (SQL, `.mongo`, `.redis`, `.esq`, `.tdb`)

| Phím | Chế độ | Việc |
|---|---|---|
| `rr` | n | chạy câu lệnh dưới con trỏ (trong `.tdb`: của block đang đứng — kể cả khi con trỏ trên dòng fence) |
| `rr` | x | chạy các dòng đang chọn — tách theo dấu `;` (SQL) và chạy lần lượt như `ra` |
| `ra` | n | chạy tất cả câu lệnh (dừng ở lỗi đầu tiên) |
| `rx` | n | huỷ câu đang chạy |
| `rc` | n | chọn connection (cũng có dòng `+ edit connections file…`) |
| `rs` | n | mở panel schema |
| `rt` | n | picker bảng (telescope) |
| `rh` | n | picker lịch sử (telescope) |
| `rm` | n | tải thêm dòng của kết quả hiện tại |
| `re` | n | `EXPLAIN` câu dưới con trỏ (chỉ SQL; không bao giờ `ANALYZE`) |
| `K` | n | thông tin bảng/cột dưới con trỏ (không có thì rơi về LSP hover) |
| `gd` | n | mở bảng dưới con trỏ (không có thì rơi về LSP go-to-definition) |
| `<C-x><C-o>` | i | completion qua `omnifunc` (hoặc nguồn `tradar.blink` của `blink.cmp`, tự động sau `.`) |

## 2. Buffer kết quả (`tradar://results`)

Là buffer Neovim bình thường: `j`/`k`/`gg`/`G`, `/`, `yy`… đều dùng được. Trang kế **không** tự tải khi cuộn — bấm `rm` (đặt `auto_load_more = true` nếu muốn như cũ). Khi query đang chạy, thanh trên cùng của cửa sổ kết quả hiện `⠋ running 3.2s on <conn> — <leader>rx cancels`.

| Phím | Việc |
|---|---|
| `gyc` / `gyr` / `gyC` | copy ô / dòng / cột (`gyr` ở visual: nhiều dòng, tab-separated) |
| `gyj` / `gyv` / `gym` | copy **cả** kết quả dạng JSON / CSV / Markdown |
| `rm` | tải trang kế của kết quả (không tự tải khi cuộn) |
| `rv` | đổi bảng ↔ JSON (SQL: mảng object; tài liệu: JSON in thụt lề) |
| `gK` | xem cả dòng hiện tại, mỗi trường một dòng |
| `K` | xem trọn giá trị một ô (khi ô bị cắt `…`) |
| `gd` | trên ô cột khoá ngoại: chạy `SELECT` dòng được tham chiếu |
| `rw` | commit các sửa trên lưới (giống `:w`) |
| `ru` | bỏ các sửa chưa commit, vẽ lại lưới |
| `q` | đóng |

**Sửa trực tiếp** (khi kết quả sửa được — xem [Truy vấn SQL](Truy-van-SQL#sửa-trực-tiếp)): mọi lệnh vim thường — `cw`, `dd`, `o`, `yyp`, `:s`, `:g`, `u`… Khi kết quả không sửa được, các phím sửa (`i a o O dd cc cw x D C s S p P`) báo lý do thay vì E21.

Lệnh: `:TradarExport csv|json|md|tsv [file]` ghi cả kết quả ra file; `:TradarMore` = `rm`.

### Buffer xem trước commit (`tradar://commit`)

| Phím | Việc |
|---|---|
| `:w` hoặc `<CR>` | chạy các câu lệnh (một transaction nếu driver hỗ trợ) |
| `q` | huỷ |

## 3. Panel schema (`tradar://schema`, mở bằng `rs`)

| Phím | Việc |
|---|---|
| `<CR>` | chèn tên bảng/cột vào buffer query; trên thư mục schema thì gập/mở |
| `<Tab>` | gập/mở schema hoặc bảng dưới con trỏ |
| `o` | mở bảng dưới con trỏ (chạy câu "xem dòng" của driver) |
| `r` | làm mới schema (sau khi DDL/migration ở nơi khác) |
| `/` | tìm trong cây như mọi buffer |
| `q` | đóng |

## 4. Picker telescope

| Picker | `<CR>` | Phím khác |
|---|---|---|
| bảng (`rt`) | chèn tên vào buffer | `<C-o>` mở bảng (chạy ngay) |
| lịch sử (`rh`) | dán câu xuống dưới con trỏ | `<C-r>` dán rồi chạy ngay |

Cả hai theo connection của block/buffer đang đứng; preview tô màu theo dialect của câu.

## 5. Kafka, Socket, RabbitMQ, HTTP, gRPC

| Buffer | Phím | Việc |
|---|---|---|
| **Kafka tail** | `p` | tạm dừng / tiếp tục (tin vẫn về, hiện khi tiếp tục) |
| | `f` | lọc theo chuỗi con (không phân biệt hoa thường; rỗng = bỏ lọc) |
| | `c` | xoá transcript |
| | `R` | đăng ký lại từ `latest` khi mất kết nối server |
| | `q` | dừng tail và đóng |
| **Socket tail** | `<CR>` (n, i) | gửi dòng nhập (dòng cuối buffer) rồi xoá nó |
| | `q` | dừng và đóng |
| **RabbitMQ peek/bindings** | `r` | làm mới (peek lại / liệt kê lại) |
| | `q` | đóng |
| **RabbitMQ publish** | `rr` | publish (hoặc `:TradarRabbitmqPublish` khi đang trong buffer) |
| **HTTP request** | `rr` | gửi (hoặc `:TradarHttpSend` khi đang trong buffer) |
| **HTTP response** | `q` | đóng |
| **gRPC request** | `rr` / `rx` | gửi / huỷ |
| **gRPC response** | `q` | đóng |

Buffer request (HTTP, RabbitMQ publish, gRPC) **sửa được** nên không chiếm `q` — đóng bằng `:q`/`:tabclose`.

## 6. Lệnh `:Tradar…`

| Nhóm | Lệnh |
|---|---|
| Connection | `:TradarConnect [tên]`, `:TradarConnections` (mở `connections.toml`), `:TradarRestart` (dừng server nền) |
| Chạy | `:TradarRun` (nhận range), `:TradarRunAll`, `:TradarCancel`, `:TradarMore` |
| Điều hướng | `:TradarSchema`, `:TradarTables`, `:TradarHistory` |
| Xuất | `:TradarExport csv\|json\|md\|tsv [file]` |
| Kafka | `:TradarKafkaConnect [tên]`, `:TradarKafkaTail [tên] [topic]` |
| Socket | `:TradarSocketTail [tên]` |
| RabbitMQ | `:TradarRabbitmqConnect`, `…Peek [tên] [queue]`, `…Bindings [tên] [exchange]`, `…Publish [tên]` |
| HTTP | `:TradarHttpConnect [tên]`, `:TradarHttpSend [tên]` |
| gRPC | `:TradarGrpcConnect [tên]`, `…Services`, `…Methods <service>`, `…LoadProto <đường-dẫn>` |

`:checkhealth tradar` kiểm tra binary server, phiên bản protocol, kết nối và yêu cầu Neovim ≥ 0.10.
