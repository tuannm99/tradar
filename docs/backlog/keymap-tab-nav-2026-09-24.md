# Điều hướng tab kiểu vim + đổi default navigator + nhảy tab theo số — xong (2026-09-24)

Phản hồi trực tiếp từ user sau khi dùng thử (không phải mục roadmap đã lên kế hoạch trước) — một loạt điểm chưa quen tay, trong đó vài điểm hoá ra đã có sẵn (làm rõ trước khi code) và vài điểm là yêu cầu thật cần đổi keymap mặc định. Chốt qua `AskUserQuestion` (4 câu) trước khi code.

**Làm rõ trước — đã có sẵn, không phải thiếu:**
- **Vim mode** trong query editor đã có, nhưng là cấu hình khởi động (`~/.config/tradar/config.toml`, `[editor]` `vim-mode = true`), áp dụng đều cho mọi driver kể cả Elasticsearch — không có toggle sống trong app (chủ ý, xem doc comment `QueryEditorComponent::new()`). User chọn "bật thử vim-mode có sẵn trước" thay vì nhúng vim/nvim thật hay viết thêm — không đổi gì trong sub-project này.
- **Lưu file** (`Ctrl+S`) đã show dòng preview đường dẫn sống (`FilePromptComponent::draw`), mặc định `~/.config/tradar/queries/`.

**Đổi thật — 3 phần:**

1. **`Ctrl+B` → `Ctrl+N`** cho `ToggleNavigator` (context `Global`). `docs/backlog/known-issues.md` từng ghi "đừng tự đổi default nếu không có yêu cầu mới" (user khác, lý do tmux prefix trùng `ctrl-b`) — đây là yêu cầu mới thật, lý do khác (mnemonic "n" cho "navigator" hợp lý hơn), nên đổi hẳn default, không giữ song song `ctrl-b` (user chọn "Đổi hẳn", không phải "thêm phím phụ"). `ctrl-b` bỏ trống, vẫn remap được qua `[keymap.global]` cho ai muốn dùng lại.

2. **`Ctrl+H`/`Ctrl+L` chuyển tab** (context `Global`), thêm bên cạnh `Ctrl+Left`/`Ctrl+Right` chứ không thay — khớp hướng trái/phải giống vim-tmux-navigator. `PrevTab`/`NextTab` giờ có 2 cặp phím cùng trỏ một lệnh.

3. **`Ctrl+1`..`Ctrl+9` nhảy thẳng tới tab theo số** — ý gốc user là "list connection phía dưới chọn theo index" nhưng làm rõ qua hỏi lại thì quy về đúng nhu cầu "nhảy nhanh giữa các tab đang mở theo số", không cần dựng thêm panel/overlay mới. Lệnh mới `Command::GoToTab1`..`GoToTab9` (9 variant unit rời, không phải `GoToTab(u8)` — khớp cách mọi `Command` khác trong file đều là unit variant, và bảng binding mặc định vẫn phải liệt kê đủ 9 cái dù enum có data hay không nên gộp lại không lợi gì). `RootComponent::go_to_tab(n)` — `n` 1-indexed, quá số tab đang mở thì kẹp về tab cuối (giống cách `next_tab()` đã kẹp thay vì không làm gì).

   **Ghi chú kỹ thuật quan trọng**: user gõ "Ctrl+Shift+số" trong câu trả lời, nhưng `KeyPress::new()` (`keymap.rs`) chủ động bỏ `SHIFT` khỏi modifier cho mọi `KeyCode::Char(_)` (comment gốc: một chữ hoa đã tự mang thông tin shift trong chính ký tự, không cần giữ cờ riêng) — hệ quả phụ là `ctrl-shift-1` và `ctrl-1` luôn resolve về đúng một `Binding`, không thể phân biệt trong keymap này. Default thật sự dùng là **`Ctrl+1`..`Ctrl+9`** (không cần Shift).

**Xung đột phải xử lý** (vì `Context::Global` luôn được resolve trước mọi context khác — `RootComponent::handle_key_event` return ngay khi Global khớp, xem comment gốc ở default binding `ctrl-left`/`ctrl-right`): claim mới trên `ctrl-h`/`ctrl-l`/`ctrl-n`/`ctrl-1..9` làm chết 3 binding cũ, phải dời:
- `Context::QueryScreen`'s `OpenSnippets` (`ctrl-l`) → `f7`.
- `Context::Http`'s `HttpOpenRequests` (`ctrl-l`) → `f7` (khác context với QueryScreen nên không đụng nhau, cùng ký hiệu "mở thư viện đã lưu" nhất quán với `Ctrl+K` "lưu" đã dùng chung ở cả hai).
- `Context::Http`'s `HttpNextMethod` (`ctrl-n`) → `f3` (`HttpPrevMethod` ở `ctrl-p` giữ nguyên, không còn là cặp ctrl-p/ctrl-n đối xứng nữa).
- `Context::Completion`'s `NextCompletion` mất hẳn `ctrl-n` (không có key thay thế) — `down` đã là binding thứ hai từ trước nên vẫn dùng được, chỉ bớt một phím phụ.

**Test**: `crates/tradar-core/src/keymap.rs` — 3 test default binding mới (`ctrl-h`/`ctrl-l` → Prev/NextTab, `ctrl-1`/`ctrl-9` → GoToTab1/9, `ctrl-n` → ToggleNavigator). `crates/tradar-app/src/components/mod.rs` — 2 test wiring (`ctrl-h`/`ctrl-l` chuyển tab giống `ctrl-left`/`ctrl-right`; `ctrl-1..9` nhảy đúng tab, quá số kẹp về tab cuối) + 3 test cũ đổi `ctrl-b` → `ctrl-n`. `crates/tradar-query-workbench/src/components/query_screen.rs` — 3 test đổi `ctrl-l` → `f7` cho snippet picker, 1 test đổi `ctrl-n` → `down` cho completion. `crates/tradar-connector-http/src/screen.rs` — 2 test đổi `ctrl-n`/`ctrl-l` → `f3`/`f7`. `crates/tradar-core/src/config/mod.rs` — test override ví dụ đổi từ `ctrl-n` sang `ctrl-g` (tránh trùng default mới khi test tự chọn key demo).

**Chưa làm** (để lại, không tự chốt trước): panel liệt kê connection riêng ở đáy màn hình (được làm rõ là không cần, quy về nhảy tab theo số là đủ); nhúng vim/nvim binary thật vào editor (user chọn thử vim-mode có sẵn trước); các gap nhỏ khác trong roadmap (remap phím vim *bên trong* editor, resize cột, `:s/pat/repl/`) không đụng tới trong đợt này.
