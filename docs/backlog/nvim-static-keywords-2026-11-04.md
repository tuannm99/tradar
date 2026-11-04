# Keyword tĩnh khi chưa kết nối (2026-11-04)

Trước đây mọi gợi ý (bảng, cột, `alias.`, JOIN theo FK) lấy từ schema của connection đang mở, nên connection sập/chưa mở/không tồn tại = danh sách rỗng.

- `DriverOpener::keywords()` (mặc định rỗng) — mỗi connector trả từ vựng ngôn ngữ của mình (nguồn duy nhất, cùng mảng mà `QueryDriver::keywords` dùng); không cần kết nối.
- Protocol 1.19: `keywords` nhận `driver` thay cho `connection`.
- Plugin: khi `ensure_connected` thất bại, server báo lỗi, hoặc block không có connection, `complete`/`omnifunc` rơi về từ khoá của driver (từ fence: `sql` → bất kỳ SQL, `mongo`, `redis`, `esq`/`elasticsearch`…), lọc theo từ đang gõ (không gợi ý khi từ rỗng); cache theo driver trong `state.keywords`. Nguồn blink bật cả ở block có ngôn ngữ nhận ra được nhưng không có connection.
- Test: `nvim/tests/blink_spec.lua`, `server::tests::keywords_of_a_driver_need_no_connection`.
