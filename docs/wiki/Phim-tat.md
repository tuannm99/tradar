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
| `rg` | n | sinh câu create/read/update/delete cho bảng dưới con trỏ (hoặc chọn bảng) |
| `rh` | n | picker lịch sử (telescope) |
| `rm` | n | tải thêm dòng của kết quả hiện tại |
| `rz` | n | phóng cửa sổ kết quả ra toàn màn hình (và nhảy vào đó); bấm lại để trả về cỡ cũ và quay lại editor |
| `re` | n | `EXPLAIN` câu dưới con trỏ (chỉ SQL; không bao giờ `ANALYZE`) |
| `K` | n | thông tin bảng/cột dưới con trỏ (không có thì rơi về LSP hover) |
| `rk` | n | cheatsheet: query mẫu của loại database này (`<CR>` chèn, `<C-r>` chèn và chạy; `:TradarCheatsheet`) |
| `gd` | n | sinh câu xem dòng của bảng dưới con trỏ vào buffer, chưa chạy (không có bảng thì rơi về LSP go-to-definition) |
| `<C-x><C-o>` | i | completion qua `omnifunc` (hoặc nguồn `tradar.blink` của `blink.cmp`, tự động sau `.`) |

## 2. Buffer kết quả (`tradar://results`)

Là buffer Neovim bình thường: `j`/`k`/`gg`/`G`, `/`, `yy`… đều dùng được. Trang kế **không** tự tải khi cuộn — bấm `rm` (đặt `auto_load_more = true` nếu muốn như cũ). Khi query đang chạy, thanh trên cùng của cửa sổ kết quả hiện `⠋ running 3.2s on <conn> — <leader>rx cancels`.

| Phím | Việc |
|---|---|
| `gyc` / `gyr` / `gyC` | copy ô / dòng / cột (`gyr` ở visual: nhiều dòng, tab-separated) |
| `gyj` / `gyv` / `gym` | copy **cả** kết quả dạng JSON / CSV / Markdown |
| `rm` | tải trang kế của kết quả (không tự tải khi cuộn) |
| `rz` | phóng to cửa sổ kết quả toàn màn hình / trả lại cỡ cũ (`:TradarResultsZoom`) |
| `rv` | đổi bảng ↔ JSON (SQL: mảng object; tài liệu: JSON in thụt lề) |
| `[r` / `]r` | tab kết quả trước / sau (mỗi bảng/collection một tab; đi vòng) |
| `[R` / `]R` | lần chạy trước / sau của tab này (giữ 5 lần, `setup{ results_history }`) |
| `rp` | ghim / bỏ ghim tab (tab ghim không bị lần chạy mới thay) |
| `rF` / `ro` | lọc các dòng đã tải (`col = v`, `col ~ text`, `col > 3`, hoặc chữ bất kỳ; `:TradarFilter`) / sắp xếp theo cột dưới con trỏ (tăng, giảm, tắt) |
| `rA` | hỏi server lọc/sắp xếp đang hiện cho **mọi** dòng (SQL; bọc câu gốc, cùng tab; bấm lại khi không còn lọc để bỏ) |
| `rD` | so sánh với lần chạy trước (hoặc tab n: `:TradarDiff [n]`): dòng thêm / mất / ô đổi theo khoá |
| `r1`…`r9` / `rx` | nhảy tới tab kết quả số n / đóng tab đang xem (`:TradarResults` để chọn) |
| `rR` / `rW` | chạy lại kết quả đang xem tại chỗ / theo dõi (lặp mỗi 5 giây, bấm lại để dừng) — chỉ với câu không ghi dữ liệu (`:TradarRefresh`, `:TradarWatch [giây\|off]`) |
| `gK` | xem cả dòng hiện tại, mỗi trường một dòng |
| `K` | xem trọn giá trị một ô (khi ô bị cắt `…`) |
| `zw` / `zW` | hiện trọn cột dưới con trỏ / mọi cột thay vì cắt ở 40 ký tự; bấm lại để thu về |
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
| `o` | sinh câu "xem dòng" của driver cho bảng dưới con trỏ vào buffer (không chạy; `rr` để chạy) |
| `r` | làm mới schema (sau khi DDL/migration ở nơi khác) |
| `/` | tìm trong cây như mọi buffer |
| `q` | đóng |

## 4. Picker telescope

| Picker | `<CR>` | Phím khác |
|---|---|---|
| bảng (`rt`) | chèn tên vào buffer | `<C-o>` sinh câu xem dòng vào buffer (không chạy), `<C-g>` menu sinh câu lệnh (CRUD; Mongo thêm find by _id, count, distinct, aggregate…) |
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
| **Workbench HTTP / gRPC** | `rr` | chạy block `###` dưới con trỏ (hoặc `:TradarHttpSend` / `:TradarGrpcSend`); gRPC streaming: bắt đầu call, hoặc gửi thêm một message |
| | `rx` | HTTP: ngừng chờ response; gRPC: huỷ call hoặc dừng stream |
| | `rc` | đóng chiều gửi của client-/bidi-stream |
| | `ry` / `rk` | copy block thành lệnh `curl` / `grpcurl` / cheatsheet block mẫu |
| | `<C-x><C-o>` | gợi ý động từ, header, `{{biến}}`, service/method gRPC |
| **Response HTTP / gRPC** (result tab) | `rH` | hiện/ẩn status và header |
| | `rR` / `rD` / `[R` `]R` | gửi lại / diff với lần trước / lần chạy trước — như mọi result tab |
| | `rF` | lọc body: `.data.items[0].id` (đường dẫn kiểu jq) hoặc chuỗi để giữ các dòng khớp |

Buffer request (workbench HTTP/gRPC, RabbitMQ publish) **sửa được** nên không chiếm `q` — đóng bằng `:q`/`:tabclose`.

## 6. Lệnh `:Tradar…`

Mọi lệnh dưới đây cũng gọi được qua một lệnh chung `:Tradar <lệnh-con>` — `:Tradar kafka tail`, `:Tradar run all`, `:Tradar save tên`… (giữ range và `!`); `<Tab>` gợi ý lệnh con rồi tới tham số. Tên connection được gợi ý bằng `<Tab>` theo đúng loại (`:TradarConnect` chỉ connection SQL/Mongo/…, `:TradarKafkaTail` chỉ Kafka…), đọc từ server nếu đang chạy, không thì từ `connections.toml`.

| Nhóm | Lệnh |
|---|---|
| Connection | `:TradarConnect [tên]`, `:TradarAddConnection` (thêm bằng hỏi đáp, thử kết nối ngay), `:TradarConnections` (mở `connections.toml`), `:TradarRestart` (dừng server nền), `:TradarLog` (xem log của server) |
| Chạy | `:TradarRun` (nhận range), `:TradarRunAll`, `:TradarCancel`, `:TradarMore` |
| Điều hướng | `:TradarSchema`, `:TradarTables`, `:TradarHistory` |
| Cheatsheet | `:TradarCheatsheet` (`rk`), `:TradarCheatsheetEdit` (query của riêng bạn, `cheatsheet.toml`) |
| Sinh câu lệnh | `:TradarCrud [create\|read\|update\|delete]` (`rg`) — chèn câu lệnh mẫu của bảng vào buffer, không chạy |
| Lưu / mở query | `:TradarSave[!] [tên]` (vào thư viện, `<leader>rS`), `:TradarSaveHere[!] [đường dẫn]` (cạnh file hiện tại), `:TradarOpen` (duyệt thư viện, `<leader>rf`) |
| Kết quả | `:TradarResults` (chọn tab), `:TradarRefresh`, `:TradarWatch [giây\|off]`, `:TradarFilter [biểu thức\|off]`, `:TradarDiff [n]`, `:TradarResultsZoom` |
| Xuất | `:TradarExport csv\|json\|md\|tsv [file]` |
| Kafka | `:TradarKafkaConnect [tên]`, `:TradarKafkaTail [tên] [topic]` |
| Socket | `:TradarSocketTail [tên]` |
| RabbitMQ | `:TradarRabbitmqConnect`, `…Peek [tên] [queue]`, `…Bindings [tên] [exchange]`, `…Publish [tên]` |
| HTTP | `:TradarHttpConnect [tên]`, `:TradarHttpSend [tên]` |
| gRPC | `:TradarGrpcConnect [tên]`, `…Services`, `…Methods <service>`, `…LoadProto <đường-dẫn>`, `…Send [tên]` |
| Workbench HTTP / gRPC | `:TradarRequests [tên]`, `:TradarHttpEndpoints [tên] [đường-dẫn]` |

`:checkhealth tradar` kiểm tra binary server, phiên bản protocol, kết nối và yêu cầu Neovim ≥ 0.10.
