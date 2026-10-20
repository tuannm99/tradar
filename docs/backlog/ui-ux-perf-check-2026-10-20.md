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
- Cột số căn trái (`134.36424411240122` / `93.8595867742349` lệch hàng thập phân); căn phải cột số sẽ dễ đọc hơn nhưng đụng `gridedit` (ánh xạ ô theo vị trí) nên cần làm riêng, có test.
- Lỗi database hiện nguyên văn (`error returned from database: (code: 1) no such table: nope`) — đủ dùng.
