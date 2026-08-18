# Layout ngang/dọc + zoom cho query/HTTP screen, chuột trái/phải/giữa (user yêu cầu 2026-08-16) — xong

User yêu cầu: query editor/results (và sau khi hỏi lại, cả HTTP request/response) đổi được layout ngang↔dọc và zoom in/out; đồng thời mọi hành động bàn phím quan trọng cũng phải làm được bằng chuột — trái/phải/giữa dùng hợp lý. Chốt qua `AskUserQuestion` trước khi code: zoom = phóng to panel đang focus (resize tỉ lệ, không full-screen ẩn panel kia); áp dụng cho cả 2 screen; chuột phải = context menu, chuột giữa = paste.

Thiết kế + chi tiết implementation đầy đủ nằm trong `docs/architecture.md`, mục **"Query/HTTP screen: layout ngang/dọc + zoom, chuột trái/phải/giữa"**. Tóm tắt:

- 2 widget dùng chung mới trong `tradar_core::ui`: `SplitPane` (split 2 pane, đổi orientation, zoom kẹp trong [20,80]%) và `ContextMenu` (popup `(label, Command)` tại điểm click, không qua keymap).
- `QueryScreenComponent`/`HttpScreen` đều dùng `SplitPane` thay layout cố định cũ; phím `F6`/`Ctrl+Up`/`Ctrl+Down` giống nhau ở cả hai.
- **Tái cấu trúc quan trọng**: tách phần dispatch command ra hàm riêng `dispatch_command()` ở cả hai screen, để phím tắt và context-menu-confirm chạy chung một code path — không có nhánh logic riêng cho chuột.
- Right-click: 5 mục trên 1 row kết quả (Edit cell/Delete row/Yank/Toggle preview/Toggle table-JSON), 1 mục trên response HTTP (Yank body).
- Middle-click = paste, qua `arboard` (dependency mới, `default-features = false` — chỉ cần đọc text, không cần hỗ trợ ảnh) — khác `yank_to_clipboard` (OSC52, một chiều) vì paste cần đọc clipboard hệ điều hành thật.
- Mở rộng whitelist mouse-event của `main.rs` (trước chỉ `Down(Left)`/`ScrollDown`/`ScrollUp`) thêm `Down(Right)`/`Down(Middle)`.
- Test: 30 test unit mới trong `tradar-core` (`SplitPane`/`ContextMenu`/`TextInput::insert_str`/`TextArea::insert_str`), 7 test mới trong `query_screen.rs`, 6 test mới trong `HttpScreen`'s `screen.rs` — tất cả pass, không có regression trong 339+ test cũ.
- **Verify tay qua tmux với sqlite thật, phát hiện một điều đáng ghi nhớ về cách test mouse qua tmux**: `tmux send-keys -H` (hex, nhiều byte) gửi escape sequence SGR mouse **không hoạt động đúng** — có vẻ tmux tách timing giữa các byte khiến parser CSI của crossterm không ghép lại thành một sự kiện trọn vẹn (từng khiến `Esc` đơn lẻ bị nhận nhầm, đẩy app quay lại picker giữa chừng test). Cách đúng: `tmux send-keys -l $'...'` (chuỗi literal ANSI-C, gửi nguyên khối). Sau khi sửa cách test: `F6` đổi layout ngang/dọc đúng ngay; `Ctrl+Up` phóng to đúng pane đang focus; right-click một row mở đúng menu 5 mục; bấm "Delete row" trong menu chạy đúng `dispatch_command` — bị từ chối đúng lý do "không có khoá chính" giống hệt phím `d`, xác nhận chuột và bàn phím dùng chung code.
- **Chưa làm** (biết trước, chưa đủ lý do làm ngay): right-click chưa có ở Navigator/ConnectionPicker/HistoryPicker (mẫu `ContextMenu` đã sẵn để mở rộng); middle-click paste chưa có ở các `TextInput` khác ngoài HTTP fields (form connection, các prompt một dòng) — hạ tầng `TextInput::insert_str` đã có, chỉ cần gọi thêm khi cần; Browse mode (Redis) cố tình không có zoom/orientation vì không phải cặp editor/results.

