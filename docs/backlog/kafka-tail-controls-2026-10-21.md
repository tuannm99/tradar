# Kafka tail: tạm dừng, lọc, xoá, nối lại

Mục 3 của đợt rà UX (`nvim-connections-tail-fixes-2026-10-20.md`). Chỉ đổi plugin (`nvim/lua/tradar/kafka.lua`, `rpc.lua`).

- Mỗi tail buffer có state (`new_tail`): `entries` (mọi tin đã nhận, tối đa `tail.MAX_LINES`) để dựng lại view khi đổi bộ lọc; `held` giữ tin đến lúc tạm dừng.
- `p` tạm dừng/tiếp tục, `f` lọc theo chuỗi con (không phân biệt hoa thường, áp cả cho tin mới), `c` xoá. Winbar hiện connection/topic, `PAUSED (n held)`, bộ lọc, trạng thái kết nối.
- Nối lại: `rpc.on_disconnect(cb)` (mới) báo khi pipe tới server đứt (không báo khi `rpc.close()` chủ động). Tail huỷ subscription cũ, chờ ~1,5s rồi `rpc.ensure` (tự khởi động lại server) → `kafka.connect` → `kafka.tail.start from=latest`; ghi `[reconnected — messages while disconnected are not shown]`. Lỗi thì winbar `DISCONNECTED — R to retry`, `R` thử lại. Chọn `latest` vì phát lại từ vị trí cũ cần offset phía server mà subscription mới không có; tin trong lúc mất kết nối bị bỏ — transcript nói rõ điều đó.
- Kiểm chứng: smoke test headless Neovim với `rpc` được stub (không cần broker): hiển thị/bỏ qua subscription lạ, pause/resume, lọc và rebuild, clear, mất kết nối → tự nối lại → tin của subscription cũ bị bỏ, nối lại thất bại → `R`. Không chạy lại trên broker thật (test e2e `make test-kafka-server` không đụng plugin).
