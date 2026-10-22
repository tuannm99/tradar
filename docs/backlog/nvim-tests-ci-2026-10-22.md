# Test plugin Neovim + CI (2026-10-22)

Bước 1 trong review kiến trúc 2026-10-22: plugin là giao diện chính nhưng không có test nào trong repo (các kịch bản headless nằm ngoài repo, mỗi lượt viết lại), và repo chưa có CI.

- `nvim/tests/run.sh`: mỗi `*_spec.lua` chạy trong Neovim headless riêng, với `tradar-server`, `XDG_CONFIG_HOME`, log và file SQLite riêng (connection `local`). Kết quả ghi vào file (`PASS`/`FAIL`, cuối cùng `DONE`), không dựa vào `print` (headless nối dòng). Spec lỗi giữa chừng không có `DONE` → tính là hỏng; khi hỏng in đuôi output Neovim và log server.
- `nvim/tests/helpers.lua`: `H.setup`, `H.sql` (seed/kiểm DB trực tiếp qua `python3 -I` + `sqlite3`), `H.run_until`, `H.mapped`, `H.check`, `H.done`.
- 9 spec chuyển từ các kịch bản smoke cũ: `render` (độ rộng cột, diff sửa lưới), `rowedit`, `results` (footer, phân trang, `max_rows`, export), `layout` (bottom/right/tab + zoom), `api` (modeline, schema, completion, keymap, explain, guard), `files`, `serverlog`, `idle` (`-- env: TRADAR_IDLE_TTL=3`), `cancel`. Tổng ~30 s.
- Hai kiểm tra cũ sai và đã sửa lại theo hành vi đúng: zoom kiểm `width >= W-1` nhưng cửa sổ còn lại giữ tối thiểu 1 cột + vách ngăn (đúng là `W-2`); sửa khoá `'^1'` không khớp vì cột số căn phải.
- `make test-nvim`; `.github/workflows/ci.yml` chạy `make check` + `make test-nvim` (Neovim v0.10.4) mỗi lần push/PR. Test cần Docker vẫn chưa vào CI.
- CI ghim Rust `1.94.1` (bản đang dùng local): lượt đầu với `stable` (1.99) đỏ vì lint clippy mới `double_must_use` bắt trong mã `async_trait` sinh ra. Nâng phiên bản thì nâng có chủ đích, kèm các sửa lint.
