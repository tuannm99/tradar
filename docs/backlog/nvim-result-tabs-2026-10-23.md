# Tab kết quả theo đích (2026-10-23)

Yêu cầu: mỗi collection/bảng xem một kết quả riêng, chuyển giữa các buffer kết quả dễ dàng.

## Quyết định

- Log tuyến tính 10 kết quả (`[r`/`]r`) đổi thành **slot theo đích**: khoá = `(connection, target)`, `target` rút ra từ text câu lệnh (`slots.target_of`: bảng đầu của `SELECT … FROM`, `db.coll.…`/`db.getCollection`, index ES, key Redis). Phân tích bằng pattern, không gọi server — cần biết trước khi chạy, cho mọi driver. Câu không có đích → khoá theo chính câu đó.
- Chạy lại cùng đích thay slot đó (giữ vị trí tab, buffer); đích mới thêm slot; quá `results_kept` thì bỏ slot dùng lâu nhất (đóng cursor, xoá buffer).
- Mỗi slot một buffer `tradar://results/<đích>`; message ("running…", lỗi, "N row(s) affected") dùng buffer riêng nên không đè lưới. `state.results_buf` luôn là buffer đang hiện, nên các module khác không đổi.
- Thanh tab = `winbar` (`slots.bar`), click qua `%@v:lua.__tradar_result_click@`. Khi query chạy, winbar là tiến trình (`conn.lua`), xong thì thanh tab trả lại.
- Cursor trên server: chỉ `results_cursors` (4) slot dùng gần nhất giữ cursor mở (còn tải thêm/sửa được); slot khác thành chỉ đọc như trước.
- Con trỏ trong từng tab được nhớ; `]r`/`[r` đi vòng.

## Chưa làm

- Lịch sử các lần chạy cũ của cùng một đích (chỉ giữ lần mới nhất).
- Pin tay một kết quả ra khỏi cơ chế thay thế.
