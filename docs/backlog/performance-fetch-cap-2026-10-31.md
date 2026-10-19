# Hiệu năng: giới hạn số dòng đọc và bộ nhớ server (2026-10-31)

Phản hồi: query trên Neovim/workbench chậm hơn client native cỡ 30 lần, server `tradar-server` ngốn ~1.5 GB.

## Đo được (release, SQLite, bảng 100k dòng x 22 cột, `select * from t`, page 20)

| | Trước | Sau |
|---|---|---|
| Thời gian `execute` | 500-600 ms (đọc và stringify 10 000 dòng) | 26-42 ms (1000 dòng) |
| RSS tăng mỗi query | ~15 MB | ~1.5 MB |
| Native (python) cho cùng 10 000 dòng | 47 ms | |

Chi phí nằm ở **đọc, stringify và giữ** 10 000 dòng dù chỉ hiện 20; vẽ lại ở phía Neovim chỉ đụng trang đầu (đã đo ở lần trước: vài chục ms cho 1000 dòng). Bộ nhớ: tối đa 16 cursor, mỗi cái tới 10 000 dòng, và plugin không bao giờ đóng cursor cũ — với cột JSON/text lớn thì mỗi cursor hàng trăm MB.

## Đã làm

- **`max_rows`** (protocol 1.10): `execute` nhận số dòng tối đa đọc từ database, mặc định server 1000, trần 100 000; plugin `setup{ max_rows }`. Chuyền tới driver qua `tokio::task_local` (`query_driver::max_rows()`), nên không đổi chữ ký `QueryDriver`; TUI/test không đặt thì vẫn `MAX_ROWS` = 10 000. Áp dụng cho SQLite, Postgres, MySQL, Cassandra, ClickHouse, Mongo (find/aggregate/distinct).
- **`RowBudget`**: ngoài số dòng còn có trần 64 MB chữ cho một kết quả (một dòng quá lớn không thể làm cạn bộ nhớ); báo `truncated`.
- **Bộ nhớ cursor của server**: tổng chữ các cursor giữ tối đa 256 MB (cursor cũ nhất bị loại trước, cái mới nhất luôn ở lại) ngoài giới hạn 16 cursor. Plugin gọi `cursor.close` cho kết quả cũ khi kết quả mới tới (trước đây không bao giờ đóng).
- SQLite: `stringify_column` decode giá trị đã cầm trong tay thay vì `try_get` (tra cột và kiểm tra kiểu hai lần mỗi ô): nhanh hơn ~20%.
- Footer ghi `stopped at N rows; setup{ max_rows } reads more` khi bị cắt.

Đo lại sau thay đổi, ép `max_rows = 10000` và chạy 40 query liên tiếp: RSS dừng ở ~390 MB thay vì tăng không chặn.

## Chưa làm / ghi chú

- Chi phí còn lại nằm ở sqlx (mỗi dòng ~3 µs, mỗi ô ~1.5 µs). Muốn nhanh hơn nữa phải đọc lười (giữ stream mở, đọc thêm khi cần): vướng pool một connection của SQLite và transaction, nên chưa làm.
- Postgres/MySQL/Cassandra/ClickHouse chưa đo thật (không có backend trong sandbox); chúng dùng cùng cơ chế giới hạn nên lợi ích về số dòng và bộ nhớ như nhau.
- TUI (`tradar-app`) giữ trần 10 000 dòng: đóng băng, chưa đổi hành vi.
- `glibc` có thể không trả bộ nhớ đã giải phóng về hệ điều hành ngay, nên RSS có thể không giảm sau khi cursor bị loại.
