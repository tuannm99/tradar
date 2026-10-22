# Socket quá dài + test picker telescope (2026-10-22)

Hai việc nhỏ cuối danh sách đề xuất sau đợt rà UX.

**Đường dẫn socket quá dài.** Tái hiện: server thoát với `path must be shorter than SUN_LEN` + backtrace, plugin chỉ báo "tradar-server did not come up at …".
- Server: `bind` kiểm độ dài trước (`MAX_SOCKET_PATH` = 107 Linux / 103 macOS, `sun_path` trừ NUL) và báo "socket path is N bytes, but … at most M … pass a shorter path". Test `a_socket_path_too_long_for_the_os_is_a_clear_error` (cả đường dẫn đúng bằng giới hạn vẫn bind được).
- Plugin: `rpc.check_path` chặn trong `ensure` (báo ngay, không đợi server không bao giờ lên); đường dẫn mặc định tự rơi về `/tmp/tradar-<uid>/server.sock` khi `$XDG_RUNTIME_DIR` quá dài — thư mục do server tạo `0700`, an toàn như đường dẫn dài; đường dẫn tự đặt trong `setup{ socket }` không bao giờ bị đổi. `:checkhealth` đo đường dẫn thật đang dùng (trước đây chỉ đoán `#XDG_RUNTIME_DIR > 70`). Spec `socket_spec` (9).

**Picker telescope chưa từng chạy với telescope thật** (sandbox/CI không có). `nvim/tests/deps.sh` tải telescope 0.1.8 + plenary v0.1.4 vào `.deps/` (gitignore) — `run.sh` gọi nó mỗi lần, idempotent. `telescope_spec` (18) lái picker thật bằng phím: tables (lọc, `<CR>` chèn tên, `<C-o>` mở bảng chạy SELECT, `<C-g>` sinh CRUD), history (`<CR>` dán), `:TradarOpen` (liệt kê thư viện lồng nhau, previewer, `<CR>`, `<C-s>`, `<C-x>` xoá sau khi confirm). Thử phá `<C-g>`/`<C-x>` thì spec đỏ đúng chỗ.
Bẫy khi viết: `run_until(sql, 'rows')` dừng ngay vì footer kết quả cũ cũng chứa "rows" — chờ một tên cột chỉ kết quả đó có.
