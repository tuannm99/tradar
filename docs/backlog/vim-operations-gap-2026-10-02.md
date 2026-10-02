# Thao tác vim còn thiếu (2026-10-02)

## Bối cảnh

Sau `connector-ux-gaps-2026-10-02.md`, người dùng chỉ đạo tiếp: rà các thao tác vim còn thiếu trong app — cả bên trong query editor (so với vim thật) và ở các panel/list khác (nhất quán `j`/`k`/`gg`/`G`/`Ctrl-d`/`Ctrl-u` giữa các component). Agent audit tìm được 8 gap cụ thể, người dùng chọn qua `AskUserQuestion`: "Làm tất cả 8, rank rẻ→đắt".

## Thay đổi

### 1. File picker "Open query": thêm `Ctrl-d`/`Ctrl-u`

`FilePickerComponent` (`crates/tradar-query-workbench/src/components/file_picker.rs`) luôn ở chế độ lọc (gõ là lọc, không có bar `/` riêng để tắt/mở như các list khác) — lý do chính đáng để không nhận `j`/`k`/`gg`/`G` (sẽ bị gõ thành chữ vào tên file). Nhưng lý do đó không áp dụng cho `Ctrl-d`/`Ctrl-u` (control chord, không ai gõ vào tên file), nên thêm hai arm nhận chúng làm `HalfPageDown`/`HalfPageUp` cùng `vim_list` như mọi list khác.

### 2. ERD overlay: sửa bug `Ctrl-d`/`Ctrl-u` chỉ cuộn 1 dòng

`ErdComponent`'s `ErdState::Viewing` (`crates/tradar-query-workbench/src/components/erd.rs`) không có field `visible_height` — `vim_list::apply` được gọi với hằng số `1` làm visible_height, nên `Ctrl-d`/`Ctrl-u` luôn di chuyển đúng 1 dòng, giống `j`/`k`, dù phím đã resolve đúng và không panic. Đây là bug thật (binding tồn tại, có vẻ hoạt động, nhưng không làm đúng việc nó nói sẽ làm), không chỉ là thiếu feature. Thêm field `visible_height: usize` vào variant, gán từ `block.inner(area).height` trong `draw()`, dùng field đó thay cho hằng số.

### 2.5. `X`: xoá ký tự trước con trỏ

`x` (`EditorDeleteChar`) chỉ xoá ký tự dưới con trỏ — vim thật còn có `X` (xoá ký tự *trước* con trỏ, không bao giờ vượt qua ranh giới dòng, giống `Backspace` ở Normal mode) mà editor chưa có. Thêm `Command::EditorDeleteCharBefore`, bind `X` trong `Context::VimNormal`, implement y hệt `x` nhưng dịch cursor trái 1 trước khi xoá.

### 3. Kafka Groups-mode / RabbitMQ: bảng chi tiết không cuộn được

Bảng lag của Kafka (Groups mode) và bảng message/binding của RabbitMQ (cả hai mode) là `Table::new` thuần, không `TableState`, không field scroll/selected nào — không có cách nào cuộn tới dòng ngoài trang đầu, kể cả `j`/`k` cơ bản. Trong khi mọi list/bảng khác trong app (results grid, navigator, browse sidebar, chính sidebar của hai connector này...) đều có `j`/`k`/`gg`/`G`/`Ctrl-d`/`Ctrl-u` qua `vim_list`.

Thêm `enum Focus { Sidebar, Detail }` cho cả `KafkaScreen`/`RabbitScreen`, chuyển bằng `tab`/`Command::CycleFocus` (cùng command/phím `query_screen.rs` dùng cho editor↔results) — bind mới trong `Context::Kafka`/`Context::Rabbit`. Khi `Focus::Detail`, `j`/`k`/`gg`/`G`/`Ctrl-d`/`Ctrl-u` áp dụng lên `lag_selected`/`detail_selected` (field mới) qua `vim_list::apply` với độ dài là số dòng bảng chi tiết, thay vì sidebar. `draw_lag`/`draw_main` đổi sang `TableState` + `.row_highlight_style()` để tô dòng đang chọn, panel border sáng theo `Focus` hiện tại. Kafka's Topics mode (bảng message tail real-time, đã có cơ chế pause/follow riêng bằng `space`) không đụng tới — nằm ngoài phạm vi gap này.

### 4-11. Query editor: word motion, operator+motion, text object, `J`/`~`/`>>`/`<<`, `.` repeat

Phần lớn effort của đợt này. Chi tiết trong doc comment của từng hàm (`crates/tradar-query-workbench/src/components/query_editor.rs`); tóm tắt:

- **Word motion `w`/`b`/`e`** (gap rẻ nhất trong nhóm editor, nhưng là tiền đề cho mọi thứ sau) — `move_word_forward`/`move_word_backward`/`move_word_end`, dựa trên 3 lớp ký tự (`CharClass::Word`/`Punct`/`Space`, tái dùng `is_word_char` đã có cho completion) và các hàm điều hướng buffer thuần (`next_pos`/`prev_pos`/`class_at`/`advance_while`/`retreat_while`/`run_start`/`run_end`). Điểm tinh tế nhất: một "run" cùng loại ký tự **không bao giờ** được phép vượt qua ranh giới dòng (vim luôn coi newline là một separator, kể cả giữa hai dòng cùng bắt đầu bằng chữ) — bug này bắt được qua test `w_crosses_a_line_boundary`/`b_crosses_a_line_boundary` lúc đầu fail (hợp nhất "id" và "from" của hai dòng khác nhau thành một "word"), sửa bằng cách bắt `advance_while`/`run_end` dừng ngay sau khi vượt dòng, không tiếp tục so khớp class ở dòng mới.

- **Operator tổng quát `d`/`c`/`y`** — thay hẳn 2 binding cố định `"dd"`/`"yy"` bằng 3 binding đơn `"d"`/`"c"`/`"y"` (`Command::EditorOperator*`) trong `Context::VimNormal`, vì keymap tự nó không có khái niệm "operator đang chờ" — logic này giờ nằm hoàn toàn ở `QueryEditorComponent` (`pending_operator`, `complete_operator`, `operator_motion`). Phím thứ hai lặp lại (operator giống nhau) → cả dòng hiện tại (`dd`/`cc`/`yy`, `cc` mới thêm, xoá nội dung dòng nhưng giữ dòng lại rồi vào Insert); là motion (`w`/`b`/`e`/`0`/`$`) → tính range bằng cách **chạy thật** motion đó, đọc vị trí cursor mới, rồi trả cursor lại — tái dùng nguyên `delete_selection`/`yank_selection` của Visual mode qua một `visual_anchor` giả lập, thay vì viết code xoá/copy riêng. Độ inclusive/exclusive của từng motion theo đúng vim (`w`/`b`/`0` exclusive — trừ ký tự đích; `e`/`$` inclusive — tính cả ký tự đích), tính bằng quy tắc chung "luôn trừ đầu lớn hơn của range nếu exclusive", đúng cho cả 2 chiều (forward/backward) mà không cần case riêng. `D`/`C`/`Y` là shortcut gọi thẳng `operator_motion(op, EditorLineEnd)`. **Cố ý không làm**: `h`/`l` làm operator motion (không có count nên `dh`/`dl` luôn đúng 1 ký tự, đã có `x`/`X`) và motion theo dòng (`dj`/`dG` — `dd`/`cc`/`yy` đã lo "dòng hiện tại", range nhiều dòng là việc khác với nhiều edge case riêng, vd dòng cuối buffer).

- **Text object `iw`/`aw`** — `i`/`a` sau operator không cần Command mới: tái dùng chính `Command::EditorEnterInsert`/`EditorAppend` (bind sẵn cho `i`/`a`) làm tín hiệu "inner"/"around", qua field `pending_scope` mới (song song `pending_operator`, không `.take()` ngay để giữ lại operator cho tới khi object thật tới ở phím thứ ba). `word_text_object(scope)` dùng lại đúng bộ hàm class-run của word motion — inner là chạy chính class tại vị trí cursor (Word/Punct/Space đều hợp lệ, `iw` trên khoảng trắng chọn đúng khoảng trắng đó), around mở rộng thêm khoảng trắng theo sau (hoặc trước, nếu không có theo sau — đúng fallback thật của vim). Chỉ hoàn thành với `w` (từ) — bất kỳ phím khác sau `i`/`a` cancel im lặng. **Cố ý không làm**: text object theo ngoặc/dấu nháy (`i(`, `a"`,...) — cần một bộ scan bracket-matching riêng, không tái dùng được maý có sẵn (auto-close chỉ biết "ký tự này mở/đóng gì", không biết "cặp NÀY đang bao quanh cursor").

- **`J`/`~`/`>>`/`<<`** — bốn lệnh độc lập, không phụ thuộc nhau: `J` nối dòng (trim khoảng trắng đầu dòng dưới, chèn đúng 1 khoảng trắng tại điểm nối — không chèn gì nếu một bên trống), `~` đổi hoa/thường ký tự dưới cursor rồi dịch phải, `>>`/`<<` thêm/bớt `INDENT_WIDTH` (hằng số mới, 4 — app chưa có config `[editor]` nào cho indent width) khoảng trắng đầu dòng, `<<` tolerant (bớt bao nhiêu có bấy nhiêu nếu ít hơn 4).

- **`.` repeat** — cố ý scope hẹp: chỉ nhớ/replay các lệnh **không liên quan gõ chữ** (`x`/`X`/`dd`/`yy`/`p`/`P`/`~`/`J`/`>>`/`<<`/operator+motion hoặc operator+text-object dạng Delete/Yank). Mọi lệnh vào Insert mode (`i`/`a`/`o`/`O`, và **mọi** dạng `c`/`cc`/`ciw`/...) không được ghi nhận — replay nguyên một phiên gõ chữ đúng nghĩa vim thật là một tính năng lớn hơn hẳn (phải ghi lại từng ký tự gõ), không làm ở đây; `last_action` vẫn giữ giá trị cũ nếu lệnh vừa chạy là dạng Change, tức `.` sau `cw` sẽ lặp lại hành động Delete/Yank *trước đó*, không phải `cw` và không phải no-op — đây là một giới hạn đã biết, không phải bug (test `dot_does_not_repeat_cw_since_change_is_out_of_scope` khoá hành vi này lại).

## Test

- `cargo test -p tradar-query-workbench --lib`: 614/614 pass (570 trước đó + 44 test mới cho các gap trên).
- `cargo test -p tradar-core --lib`: 125/125 pass (124 trước đó + 1 test mới cho remap `editor-operator-delete`, cộng 1 test cũ đổi sang remap `editor-toggle-fold` vì `editor-delete-line` không còn tồn tại).
- `cargo test -p tradar-connector-kafka --lib`: 21/21 pass (cần `libcurl4-openssl-dev` cài sẵn trong sandbox từ trước).
- `cargo test -p tradar-connector-rabbitmq --lib`: 17 pass + 9 Docker-integration fail như kỳ vọng (không đổi).
- `cargo build --workspace`, `cargo clippy --all-targets --workspace -- -D warnings`, `make test-unit`, `cargo fmt --all -- --check`: sạch.

## Chưa làm

- Text object theo ngoặc/dấu nháy (`i(`, `a(`, `i"`, `a"`,...) — cần bracket-matching scanner riêng, xem mục operator/text-object ở trên.
- `h`/`l` làm operator motion, và motion theo dòng (`dj`/`dG`-style multi-line range) làm operator motion.
- `.` không replay được một phiên Insert (mọi `c`-family, `i`/`a`/`o`/`O`) — xem mục `.` repeat ở trên, giới hạn cố ý.
- Count prefix (`3w`, `d2w`,...) — editor này chưa có khái niệm count ở đâu cả, không riêng cho các lệnh mới ở đây.
- Named register (`"ayy`, `"ap`,...) — vẫn chỉ một register thầm lặng (`"` mặc định) như từ đầu.
- Kafka's Topics-mode message table (tail real-time) chưa có `Focus`/scroll qua `j`/`k` — pause/follow riêng (`space`) đã có từ trước, thêm scroll thật sẽ cần nghĩ lại tương tác giữa hai cơ chế, nằm ngoài phạm vi gap #3.
