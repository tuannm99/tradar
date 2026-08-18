# Đợt polish UX chuột + báo hiệu tab nền (user yêu cầu 2026-08-18) — xong

Sau khi báo cáo hiện trạng, user chọn làm ngay 4 việc UX đã đề xuất (phần hiệu năng đẩy vào backlog, xử lý sau — xem `docs/backlog/keymap-and-performance-2026-08-18.md`, không làm trong đợt này).

### 1. Widget dùng chung `tradar_core::ui::DoubleClickTracker` — xong

Trước khi làm 3 việc double-click bên dưới, gom logic double-click (đã viết riêng cho `ConnectionPickerComponent` ở đợt trước) thành 1 struct dùng chung trong `tradar_core::ui` — `click(index) -> bool` (true = double-click), cộng `age_last_click(Duration)` (seam cho test, tránh phải sleep thật 500ms). `ConnectionPickerComponent` refactor sang dùng struct này thay vì field `last_click`/`DOUBLE_CLICK_WINDOW` riêng.

### 2. Double-click cho Navigator, History picker, Snippet picker — xong

- **Navigator**: `click()` giờ trả `bool` (double-click), `RootComponent` tách `apply_nav_outcome()` từ `handle_navigator_key` để cả phím lẫn double-click chuột dùng chung 1 đường xử lý outcome.
- **History picker** (`history_picker.rs`): trước đó **không có mouse handling nào cả** (kể cả single-click) — thêm `list_state`/`list_area` (trước dùng `ListState` cục bộ mỗi `draw()`, không giữ được offset để hit-test), `handle_mouse_event()`: click chọn dòng, double-click load = Enter.
- **Snippet picker** (`snippet_picker.rs`): cũng chưa có mouse handling nào. Thêm double-click = Insert, cộng **chuột phải mở context menu** (Insert/Rename/Delete) — tách `dispatch_command()` từ `handle_key_event` để phím tắt và menu dùng chung code, đúng pattern đã lập ở `ConnectionPickerComponent`.
- **Bug tiện thể bắt được lúc rà `query_screen.rs`'s `handle_mouse_event`**: `snippet_picker`/`snippet_prompt` không nằm trong danh sách "overlay chặn click xuyên" (chỉ có `prompt`/`history_picker`/`picker`/`row_edit`) — click khi snippet picker đang mở lọt xuống tận editor/results phía sau. Fix: route `history_picker`/`snippet_picker` mouse event riêng (trước khi tới guard chung), thêm `snippet_prompt` vào guard. Test: `clicking_while_the_snippet_picker_is_open_does_not_leak_through_to_the_results_pane`.

### 3. Ô filter (`/`) cho connection picker — xong

Trước đó chỉ Navigator có filter; connection picker (màn hình chính để mở connection) thì không, danh sách dài phải cuộn tay. Thêm y hệt pattern Navigator đã có (`TextInput`, substring không phân biệt hoa/thường theo tên **hoặc** driver, `Esc` xoá hẳn, `Enter` giữ + đóng bar).

**Phần khó nhất**: `selected: usize` (field `pub`, `RootComponent` ghi trực tiếp khi mở/restore tab) trước giờ luôn là index thẳng vào `connections`. Filter cần nó là index vào danh sách *đã lọc* — thêm `visible_indices()`/`selected_connection_index()` để dịch qua lại, sửa mọi chỗ đọc `self.selected`/`self.connections.len()` trực tiếp (`open_selected`, `delete_selected`, `apply_move`, `EditConnection`/`DeleteConnection`, hit-test chuột, badge `open_in_tab` — field này vẫn song song với `connections` gốc nên phải dịch riêng). An toàn cho toàn bộ test cũ vì `filter` rỗng mặc định → `visible_indices()` = `0..connections.len()` y hệt trước, `RootComponent`'s trực tiếp gán `.selected` vẫn đúng miễn chưa gõ filter.

### 4. Badge tab nền connect xong/lỗi — xong

Gap đã ghi nhận từ đợt tab/session trước ("không có badge/thông báo khi tab nền connect xong hoặc connect lỗi"). Thêm `Tab.unseen_outcome: Option<bool>` — set trong `RootComponent::update`'s `Opened`/`OpenFailed` khi `tab != active_tab`, hiện `✓`/`✗` màu `theme.accent`/`theme.error` trong tab bar, biến mất khi tab đó được vẽ làm active (không cần action dismiss riêng — nhìn thấy chính là acknowledge).

**Bug tự bắt được lúc verify tay qua tmux**: badge vẫn còn treo 1 frame sau khi đã chuyển sang tab đó — do `draw_tab_bar()` chạy *trước* dòng clear `unseen_outcome` trong `draw()`, nên frame đầu tiên sau khi chuyển tab vẫn đọc giá trị cũ. Fix: dời dòng clear lên đầu `draw()`, trước `draw_tab_bar()`. Test cũ `drawing_the_active_tab_clears_its_unseen_badge` (chỉ check state sau `draw()`) không bắt được lỗi này — thêm test chặt hơn `the_very_frame_that_switches_to_a_tab_already_omits_its_badge`, assert thẳng vào buffer của chính frame đó.

### 5. Redis browse sidebar — rà lại, thêm mouse — xong

`BrowseSidebarComponent` trước đó **không có mouse handling nào** (không cả single-click) — cùng tình trạng History/Snippet picker ở trên trước khi sửa. Thêm `list_state`/`list_area`, enum `BrowseClick { Missed, Selected, Activated }` (khác kiểu trả `bool` của Navigator vì `query_screen.rs` cần phân biệt "chỉ landed" (focus) và "double-click" (fetch key) — không gộp được thành 1 bool như Navigator vì Navigator chỉ cần double-click, không cần biết "landed nhưng không double-click"). `query_screen.rs`: click chọn key + focus panel, double-click fetch key = `Command::BrowseOpen`/Enter.

- Test mới: 4 test `click()` trong `browse_sidebar.rs` (select, activate, miss ngoài panel, miss khi đang hiện error), 2 test tích hợp trong `query_screen.rs` (click chọn không fetch, double-click fetch giống hệt Enter — dùng `redis_screen_with` fixture có sẵn).
- `cargo test -p tradar-core -p tradar-app -p tradar-query-workbench`: 109 + 118 + 378 pass. `cargo clippy --all-targets --workspace -- -D warnings` sạch. Verify tay qua tmux: filter picker (gõ `/mongo`, Enter, Enter → connect đúng connection đã lọc, không nhầm); mở nền `dev-redis` qua navigator trong khi tab khác vẫn active → thấy `✓ dev-redis` ở tab bar; chuyển sang tab đó → badge biến mất ngay từ frame đầu (sau khi fix bug thứ 2).

