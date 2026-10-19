# Kết quả nặng, query lâu, chọn nhiều dòng

Từ phản hồi dùng thật: bảng > 50 dòng render chậm, query lâu bị khựng/không có tiến trình/làm việc khác bị lag, view JSON `gg` bị cuộn xuống, chọn nhiều dòng phải hiểu `;`.

## Đã đo

`render.flatten`/`render.table`/`render.documents` chỉ vài chục ms cho 1000 dòng (headless), và đường SQL 1000 dòng → vẽ xong ~100 ms, độ trễ vòng lặp chính tệ nhất ~30 ms — nên phần "chậm" nằm ở những thứ quanh render, không phải hàm vẽ:

- **Tự tải trang khi cuộn**: `CursorMoved` ở cuối buffer gọi `M.more` → vẽ lại cả buffer dưới tay người dùng (documents luôn vẽ lại toàn bộ), và vị trí con trỏ được khôi phục từ lúc bắt đầu tải (nguồn gốc `gg` bị kéo xuống ở view JSON). Nay **bỏ tự tải**: trang kế là phím `<leader>rm` (có trong buffer kết quả), chân trang nói rõ "N more: <leader>rm loads the next page". `setup{ auto_load_more = true }` giữ hành vi cũ. Trang mặc định 200 → 100 → 20 (`setup{ page_size }`, theo yêu cầu).
- **Mongo gọi `list_schema` sau mỗi kết quả** (để tìm cột khoá) — mỗi lần lấy mẫu một document từ *mọi* collection, tranh connection với query kế tiếp. `edit_key_columns` của Mongo nay trả `["_id"]` nên plugin không cần schema nữa.
- **Syntax JSON bằng regex** trên buffer lớn chậm: `synmaxcol=200`, và tắt hẳn khi > 3000 dòng.
- **Spinner `redrawstatus` 10 lần/giây** buộc statusline (lualine & co.) tính lại liên tục suốt thời gian query → gõ phím lag. Nay thanh tiến trình là `winbar` của cửa sổ kết quả (chỉ vẽ lại một cửa sổ, 5 lần/giây) và `redrawstatus` chỉ 1 lần/giây.

## Tiến trình query

Cửa sổ kết quả hiện `⠋ running 3.2s on <conn> — <leader>rx cancels` (winbar) suốt lúc query chạy, xoá khi xong; kết quả cũ vẫn thấy. Cửa sổ đang đóng thì mở ra với dòng "running…".

## Chọn nhiều dòng

Visual `<leader>rr` trước đây gửi cả vùng chọn như một câu. Nay coi như "chạy tất cả" trên vùng chọn: server `split` theo dialect (`;` với SQL), chạy lần lượt, dừng ở lỗi đầu tiên, một hộp xác nhận cho cả lô nếu có câu nguy hiểm.

## Kiểm chứng

Headless Neovim + server thật + SQLite 5000 dòng: trang đầu 100 dòng (lúc đó mặc định 100; nay 20); `G` không tải thêm; `<leader>rm` ra 200; winbar có `running … cancels` khi query chậm rồi xoá; vùng chọn 3 dòng/4 câu (create, 2 insert, select) chạy đủ và ra `count = 2`. Unit test `edit_key_columns` của Mongo. **Chưa tái hiện được "50 dòng chậm" ở môi trường này** — nếu còn chậm, cần biết kết quả đó là SQL hay Mongo/ES, số cột, và plugin nào khác đang gắn vào buffer `tradar://results` (indent guides, colorizer, illuminate… thường là thủ phạm với dòng rất rộng).
