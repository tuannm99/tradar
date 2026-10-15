# Rà soát UI/UX plugin Neovim (2026-10-15)

## Bối cảnh

Dùng Neovim + `tradar-server` được một thời gian (gRPC vừa xong, roadmap tạm dừng ở việc thiết kế push-channel chưa code) — người dùng yêu cầu rà lại toàn bộ keymap/command của plugin Neovim, ghi nhận thao tác hiện có, gợi ý tối ưu, rồi làm hết và đẩy PR. Không phải một tính năng mới — một đợt dọn UX dựa trên đọc lại code, không phải báo lỗi cụ thể từ việc dùng thật.

## Phát hiện và sửa

### `gT` đè lên phím mặc định của Neovim

`gT`/`gt` vốn là tab trước/sau. Buffer kết quả remap `gT` thành "đổi view bảng ↔ JSON" — một xung đột thật, bị bỏ qua lâu vì ít ai dùng nhiều tab, nhưng gRPC (xem dưới) vừa làm việc này phổ biến hơn hẳn (mỗi request một tab). Đổi sang `<leader>rv`, cùng tiền tố mọi hành động khác trong buffer SQL đã dùng.

### Thiếu cách khám phá phím ở buffer kết quả / panel schema

`M.setup`'s `wk.add` (which-key) chỉ đăng ký nhóm `<leader>r*`; các phím `g`-trần (`gyc`/`gyr`/`gyC`/`gyj`/`gyv`/`gym`, `i`/`dd`, `<CR>`/`<Tab>` của panel schema) không ai thấy trừ đọc README. Thêm `g?` ở ba chỗ (buffer SQL, buffer kết quả, panel schema) — mở một float liệt kê đúng phím của buffer đó, qua `vim.lsp.util.open_floating_preview` (hàm `show_help` mới, `init.lua`, cùng cơ chế `K`/hover đã dùng).

### Buffer kết quả: không cap độ rộng cột, không có cách xem trọn ô dài

`render.lua`'s `M.table` cũ đo độ rộng mỗi cột = giá trị dài nhất, không giới hạn — một ô chứa blob/JSON dài kéo cả cột rộng theo, đẩy mọi cột sau ra xa (phải cuộn ngang nhiều mới thấy cột kế). TUI đã tự cap auto-width ở 40 ký tự từ lâu (`docs/backlog/column-resize-2026-09-24.md`), Neovim chưa port.

Thêm `render.MAX_COLUMN_WIDTH = 40`; ô vượt cap bị cắt kèm `…` (`pad_or_truncate`, cắt theo số ký tự qua `strcharpart` — không wcwidth-chính xác tuyệt đối cho text nhiều ký tự rộng gấp đôi như CJK, chấp nhận được vì không chỗ nào khác trong codebase này làm chặt hơn). Thêm `K` trong buffer kết quả (trước đó chưa dùng ở buffer này — `K` của buffer SQL là hover schema, khác buffer) xem trọn giá trị ô dưới con trỏ qua float, làm lối thoát cho việc cap.

Chủ đích **không** làm resize tay kiểu TUI's `<`/`>` — `K` đã đủ cho nhu cầu "xem trọn", resize thêm việc mà chưa thấy cần.

### Buffer kết quả: vẽ lại toàn bộ mỗi lần tải thêm trang

`paint_results` cũ gọi `render.table(r.columns, r.rows)` trên **toàn bộ** số dòng đã tải mỗi khi cuộn tới cuối/`<leader>rm` tải thêm một trang — tốn dần theo cấp số cộng số trang đã tải (trang N đo+dựng lại cả N trang, không chỉ trang mới), dù tầng network đã đúng nguyên tắc "virtual scrolling" (cursor + `fetch` theo trang server) từ trước.

Tách `render.lua`'s `M.table` thành ba phần dùng riêng được: `M.column_widths(columns, rows, prev)` (đo, nhận `prev` để seed thay vì luôn bắt đầu từ đầu), `M.header_lines`/`M.row_lines` (dựng, không đo). `paint_results(new_from)` (`init.lua`) giờ: nếu kết quả là `table` (SQL — cột cố định từ câu query, không đổi giữa các trang) và đã có `r.widths` từ lần vẽ trước, chỉ đo độ rộng của **trang mới** (seed bằng `r.widths` cũ); nếu không cột nào rộng thêm, chỉ dựng dòng của trang mới rồi `nvim_buf_set_lines` thay đúng 2 dòng chân trang (dòng trống + tiêu đề) bằng (dòng-mới + chân-trang-mới) — không đụng tới bất kỳ dòng nào đã vẽ trước đó. Nếu có cột rộng thêm (hoặc lần vẽ đầu tiên), rơi về vẽ lại toàn bộ như cũ — không có regression, chỉ thêm một đường tắt cho trường hợp phổ biến (hầu hết trang mới không làm đổi độ rộng cột nào).

**Không áp dụng cho `documents` hiện dạng bảng** (Mongo/Elasticsearch, qua `render.flatten`) — tập cột của nó có thể đổi giữa các trang (document sau có field cấp cao nhất mà document trước không có), nên luôn vẽ lại toàn bộ như trước, không tối ưu lần này. Phạm vi cố ý giới hạn ở trường hợp phổ biến nhất (kết quả SQL).

### gRPC: ba điểm chưa nhất quán so với phần còn lại của plugin

- **Thiếu `q`** đóng buffer response — mọi buffer nổi khác không-sửa-được trong plugin (kết quả, panel schema) đều có. Thêm `q` ở response split (chỉ đóng cửa sổ đó, giữ request/tab — cùng vai trò `q` của buffer kết quả SQL). **Cố ý không** thêm `q` ở buffer **request**: khác response split, buffer này sửa được thật (JSON người dùng gõ tay) — bản nháp đầu của PR này từng thêm `q` ở cả đó, nhưng bị bắt lại ngay (phản hồi trong lúc review PR): đè `q` ghi macro của vim trên một buffer sửa được là đúng cùng loại lỗi vừa sửa cho `gT`/`gt` ở trên — mất một phím thật sự hữu ích chỉ để đổi lấy một tiện lợi nhỏ. `:q`/`:tabclose` mặc định đã đủ (buffer luôn nằm riêng một tab).
- **Luôn mở tab mới**, kể cả khi chọn lại đúng method vừa xem — duyệt qua nhiều method liên tiếp (rất hay gặp khi khám phá một service lạ) chất chồng tab không dọn. Thêm registry `connection/service/method -> buffer` (`open_buffers`): chọn lại đúng bộ ba đó nhảy về buffer/tab cũ (buffer tự dọn nhờ `bufhidden = 'wipe'` nên registry không cần dọn tay khi tab bị đóng).
- **Không có cách huỷ** một `invoke` đang chạy lâu — SQL có `<leader>rx`/`:TradarCancel` từ đầu. Thêm `query_id` tuỳ chọn cho `grpc.invoke` (server), `cancel` RPC method dùng chung được (tra thêm `State::grpc_running: HashMap<String, Arc<Notify>>` — map riêng vì gRPC không có `Arc<dyn QueryDriver>` để giữ cùng `Notify` như `running` của SQL đã làm); `<leader>rx`/`:TradarGrpcCancel` buffer-local, best-effort giống `cancel` của SQL (chỉ bỏ future đang chờ phía client, gRPC không có "stop processing" thật phía nó để gọi).

Cũng tiện sửa: `services()`/`methods()` không truyền tên connection giờ ưu tiên connection của **buffer request đang đứng** (nếu có) trước khi rơi về "connection gRPC dùng gần nhất" toàn cục — tránh thao tác nhầm connection khi có từ hai connection gRPC trở lên cùng lúc (ví dụ so sánh staging/prod).

### Chủ đích ghi lại, không sửa gì

Buffer kết quả Neovim không có sort/filter theo cột (TUI có cả hai, Tier 1 #9 / Tier 4 #10 trong checklist so sánh) — chủ đích: Neovim có sẵn editor SQL thật, sửa `ORDER BY`/`WHERE` rồi `<leader>rr` chạy lại rẻ hơn một UI tương tác riêng phải xây lại từ đầu. Ghi vào `docs/architecture.md` để đây không còn là một khoảng trống ngầm hiểu.

## Test

Không có hạ tầng test Lua tự động trong repo (như mọi lần trước với `nvim/`) — xác nhận bằng hai script headless Neovim thật, chạy tay:

- `render_test.lua` (11 kiểm tra, thuần `require('tradar.render')`, không cần server): `column_widths` chọn đúng giá trị rộng nhất, cap đúng `MAX_COLUMN_WIDTH`, kết quả đo tăng dần (`prev` + trang mới) khớp đúng kết quả đo lại từ đầu khi không có gì rộng thêm, vẫn tăng đúng khi trang sau rộng hơn; ô vượt cap có `…` và không vượt độ rộng cột; `M.table` ghép đúng ba hàm con mới; `row_lines` không kèm header.
- `smoke2.lua` (20 kiểm tra, `tradar-server` thật qua sqlite + một toy gRPC server thật, không mock trừ `vim.ui.select`): tạo bảng 450 dòng (dòng 300 cố ý dài 80 ký tự) qua một `INSERT` nhiều hàng, `SELECT` rồi cuộn/`<leader>rm` liên tục tới khi tải hết — xác nhận số dòng cuối cùng đúng, ô dài bị cắt kèm `…`, con trỏ không văng khỏi buffer qua nhiều lần vẽ lại (tăng dần lẫn vẽ lại toàn bộ khi gặp ô dài ở trang giữa), `g?`/`K` không lỗi và `K` cho đúng giá trị chưa cắt; gRPC: mở buffer theo method, chọn lại đúng method không mở tab thứ hai, gửi+huỷ đua nhau không treo máy, `q` đóng đúng tab.

`cargo test -p tradar-server --features grpc` (30 test, 0 lỗi, gồm test cũ + không có test riêng cho nhánh huỷ gRPC mới — xem "Chưa làm"), `cargo build --workspace`, `cargo build -p tradar-server --no-default-features --features postgres,sqlite` (cfg không đổi gì ở phần này nên không cần kiểm riêng, chỉ xác nhận không vỡ), `cargo clippy --all-targets --workspace -- -D warnings`, `cargo fmt --all -- --check`, `make test-unit` đều sạch.

## Chưa làm

- Không có test Rust tự động cho nhánh huỷ `grpc.invoke` mới (`query_id`/`grpc_running`/đua `tokio::select!`) — xác nhận bằng smoke test headless (gửi+huỷ đua nhau, không treo) chứ chưa viết test tích hợp riêng trong `tests/rpc_grpc.rs` (cần một toy server trả lời chậm có chủ đích để test huỷ thật-sự-huỷ-được, không chỉ "không treo" — để sau).
- `render.lua`'s truncation không wcwidth-chính xác tuyệt đối cho text nhiều ký tự rộng gấp đôi (CJK) — đã ghi nhận trong code, không phải mục tiêu lần này.
- Không thêm resize cột tay cho buffer kết quả Neovim (TUI có `<`/`>`) — `K` đã là lối thoát, resize để sau nếu thấy cần.
- Tối ưu vẽ tăng dần chỉ áp dụng cho `table` (SQL) — `documents` hiện dạng bảng (Mongo/Elasticsearch) vẫn vẽ lại toàn bộ mỗi trang, như đã nêu ở trên.
