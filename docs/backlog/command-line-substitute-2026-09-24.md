# `:s/pat/repl/` trong query editor — xong (2026-09-24)

Gap nhỏ cuối cùng còn lại trong `docs/roadmap.md`'s nhóm editor trước khi tới mục lớn nhất (remap phím vim). Chưa có tiền lệ UI dòng lệnh `:` nào trong app từ trước — chốt qua `AskUserQuestion` (2 câu) trước khi code.

**Chốt phạm vi**:
1. **Theo đúng vim thật**: không tiền tố (`s/pat/repl/`) chỉ áp dụng **dòng hiện tại**; tiền tố `%` (`%s/pat/repl/`) mới áp dụng **toàn buffer** — không đơn giản hoá thành "luôn toàn buffer" như một lựa chọn đã cân nhắc, vì sai kỳ vọng của người quen dùng vim thật.
2. **Pattern là literal substring, không phải regex** — nhất quán với `/` search đã có (cũng literal, dùng `find()`), không thêm dependency `regex` vào `Cargo.toml` chỉ cho một tính năng nhỏ; vim regex cũng khác cú pháp Rust `regex` nên dù có thêm cũng không khớp hoàn toàn kỳ vọng vim thật.

Cờ `g` (mọi match trên dòng, không chỉ lần đầu) vẫn giữ đúng vim: không có `g` chỉ thay khớp **đầu tiên** mỗi dòng chạm tới.

**UI — dòng lệnh mới, tách khỏi `buffer_search`**: `QueryScreenComponent` thêm field `command_line: Option<ui::TextInput>`, trigger bằng `:` (`Command::EnterCommandLine` mới, context `Editor`) — **chỉ từ Normal mode**, khác `/` (đã mở rộng sang Visual ở mục trước) vì cú pháp range-từ-selection thật của vim (`:'<,'>s/.../.../ `) không nằm trong phạm vi lần này. Không có preview theo từng ký tự như `buffer_search` (tìm kiếm là thao tác đọc, an toàn để preview sống; `:s` sửa buffer thật nên không có gì hợp lý để preview giữa chừng) — chỉ parse và áp dụng khi `Enter`, `Esc` huỷ không đổi gì. Thanh dưới cùng của editor giờ dùng chung giữa `buffer_search` (`/`) và `command_line` (`:`) — hai cái loại trừ nhau (mỗi phím tự đóng overlay kia trước khi mở overlay của mình), nên vẫn chỉ cần đúng 1 dòng dành riêng, không phải 2.

**`parse_substitute()`** (hàm thuần trong `query_screen.rs`) parse text gõ sau `:` (bản thân `:` không nằm trong text, giống cách `buffer_search` không thấy `/` của chính nó) thành `(whole_buffer, pattern, replacement, all_on_line)`. Chỉ hỗ trợ `/` làm delimiter (vim thật cho chọn dấu câu bất kỳ) — không hỗ trợ escape `\/` bên trong pattern/replacement, xem "Chưa làm" bên dưới.

**`QueryEditorComponent::substitute()`** (method mới, cạnh `find()`) làm việc thật: literal `str::replace`/`str::replacen` theo từng dòng, `checkpoint()` đúng một lần cho toàn bộ lần gọi (không phải mỗi dòng) — nên sửa nhiều dòng cùng lúc vẫn undo được trong **một** bước `u`/`Ctrl+Z`, không phải undo từng dòng riêng lẻ. Không checkpoint gì nếu không có dòng nào khớp (trả về `0`), nên bấm `:s/khong-ton-tai/x/` xong `u` không vô tình lùi lại một bước edit thật trước đó.

**Im lặng khi xong, kể cả không khớp gì** — khác vim thật (báo "N substitutions" / "Pattern not found" ở dòng lệnh của chính nó). App chưa có status line dùng chung cho thông báo tạm thời kiểu đó, và undo chỉ cách một phím nếu gõ sai — quyết định không dựng thêm cơ chế thông báo chỉ cho một lệnh.

**Test**: `crates/tradar-query-workbench/src/components/query_editor.rs` — 9 test cho `substitute()` (mặc định chỉ dòng/match đầu tiên, cờ `g`, `%` toàn buffer × có/không `g`, không khớp gì thì không đổi gì + không tốn bước undo, pattern rỗng no-op, cursor nhảy tới dòng cuối bị đổi, undo gộp nhiều dòng thành một bước). `crates/tradar-query-workbench/src/components/query_screen.rs` — 5 test cho `parse_substitute()` (dòng hiện tại, `%s` + `g`, bỏ delimiter cuối, replacement rỗng, các trường hợp bị từ chối), 4 test tích hợp (`Enter` áp dụng, `Esc` huỷ không đổi gì, `:` trong Insert mode gõ literal, `:` trong Visual mode không mở — cố tình, đúng quyết định không mở rộng Visual).

**Chưa làm** (để lại, không tự chốt trước): escape `\/` để thay literal `/` trong pattern/replacement; delimiter khác `/` (vim thật cho chọn tuỳ ý, vd `:s#a#b#` khi pattern chứa `/`); range kiểu `:'<,'>s/.../.../ ` từ Visual selection; thông báo "N substitutions"/"Pattern not found"; regex thật (bị loại rõ ràng ở vòng scope).
