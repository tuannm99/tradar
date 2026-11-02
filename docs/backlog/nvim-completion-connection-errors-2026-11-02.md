# Completion báo lỗi kết nối (2026-11-02)

Triệu chứng: database sập (hoặc đường dẫn sai) thì `.tdb`/file query chỉ "không có gợi ý" — không cách nào biết do đâu.

- `complete.lua` (`M.completion_failed`): khi `ensure_connected` của connection trong block thất bại, hoặc `complete` (blink lẫn omnifunc) trả lỗi, hiện một `notify` mức WARN: ``completion: `tên` is not available -- lý do``. Lỗi cũ trả mảng rỗng như trước nên gõ không bị gián đoạn.
- Một lần mỗi connection (`state.completion_warned[conn]`), vì hàm này chạy theo từng phím; xoá cờ khi một lần `complete` thành công, nên connection hỏng lại sau đó được báo lại.
- Không đổi server/protocol. Test: `blink_spec` (connection trỏ database không tồn tại: rỗng + cảnh báo đúng một lần dù gõ tiếp, connection khác không bị báo, tạo database rồi hoàn thành chạy lại và cờ được xoá).
