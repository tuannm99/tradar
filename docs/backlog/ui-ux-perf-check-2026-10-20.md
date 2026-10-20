# Rà UI/UX + hiệu năng plugin Neovim (2026-10-20)

Đo trên `tradar-server` (release) + SQLite, bảng 300.000 dòng × 12 cột; nvim headless.

| Tình huống | Kết quả |
|---|---|
| Nạp plugin (`require('tradar')`, sau khi tách module) | ~3–4 ms |
| Query → vẽ xong, trang 20 dòng | 18–41 ms |
| Trang 1000 dòng × 12 cột | ~54 ms (đọc 20.000 dòng: ~730 ms), main loop đứng tối đa ~36 ms |
| Trang kế (+1000 dòng) | ~27 ms |
| 5 ô văn bản 360 KB | ~170 ms, đứng tối đa ~124 ms (ô bị cắt `…` khi vẽ) |
| Kết nối DB 3000 bảng | **2603 → 389 ms** |
| Mở navigator 3000 bảng | **2882 → 707 ms** |

**Đã sửa:** `SqliteDriver::list_schema` chạy 2 `PRAGMA` cho *mỗi* bảng (6000 round trip với 3000 bảng) và chạy lại ở mỗi `connect`/`schema`/`snippet`. Giờ là 3 truy vấn cố định (`sqlite_master` join `pragma_table_info` / `pragma_foreign_key_list`); thứ tự bảng/cột và kết quả giữ nguyên.

**Còn lại (chưa làm):**
- `snippet` (mở bảng từ navigator) gọi lại `list_schema()` mỗi lần chỉ để tìm một bảng; với Postgres/MySQL (vài truy vấn) chưa đáng kể, nhưng nên cache schema theo connection nếu gặp schema rất lớn.
- Lỗi database hiện nguyên văn (`error returned from database: (code: 1) no such table: nope`) — đủ dùng.

**Căn phải cột số (đã làm):** `render.number_columns` quyết định cột nào toàn số (bỏ rỗng/`NULL`, số 0 đứng đầu coi là chữ); `pad_or_truncate(.., right)` đệm bên trái; `header_lines`/`row_lines`/`table` nhận/trả `aligns`, lưu ở `r.aligns` (`r.taligns` cho bảng phẳng của documents). Trang nạp thêm chỉ được *hạ* một cột từ số xuống chữ (khi đó vẽ lại như khi cột rộng ra). Sửa/`:w` không đổi vì `gridedit`/`split_line` đã cắt khoảng trắng hai đầu; `rowedit` so header theo đúng `aligns`.

**Copy ô bị cắt `…` (đã làm):** `gy*` đã lấy giá trị gốc, nhưng yank thường của vim (`yy`/`y$`/visual `y`) chép chữ trên màn hình. `TextYankPost` trong buffer kết quả (`resultskeys.lua`) dùng `render.expand_truncated` để thay `…` bằng phần còn lại của giá trị trong register (và `+` nếu `clipboard` có `unnamed*`); bỏ qua yank theo khối và khi lưới có sửa chưa commit.

**Cắt ô theo độ rộng hiển thị (đã làm):** `pad_or_truncate` cũ cắt theo *số ký tự* (`strcharpart`), nên ô toàn chữ rộng đôi (CJK, emoji) vượt cột (dòng 89 cột thay vì 49) và đẩy mọi cột sau lệch. Giờ `cut()` lấy tiền tố dài nhất mà cùng `…` vẫn vừa `width` cột hiển thị (có thể chừa 1 cột trống khi gặp ký tự rộng đôi); `expand_truncated` dùng cùng `cut()` nên yank vẫn khôi phục đủ chữ.

**Đo thêm (20.000 dòng × 12 cột, `page_size = 20000`):** vẽ ~2,2 s (chủ yếu Lua dựng 240.000 ô; mặc định `page_size = 20` nên không gặp), chuyển JSON view ~0,6 s, yank/export CSV/JSON 110–220 ms, bảng 500 × 201 cột ~0,5 s.
