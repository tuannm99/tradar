# TTL cho connection (2026-10-22)

User: connection không chạy một thời gian thì tự disconnect để giảm RAM.

- `server/idle.rs`: `Server::touch` ghi `last_used` mỗi lần có request nêu `connection` (trước và sau khi chạy, để query dài không bị tính là nhàn rỗi), **trừ** `status`/`version`/`disconnect` — `status` là poll 15 s của plugin, tính vào thì không connection nào idle. `reap_idle(ttl, now)` đóng connection quá hạn, bỏ qua khi đang transaction, có query đang chạy (`state.running`) hoặc có lời gọi đang giữ driver (`Arc::strong_count > 1`). `reap_loop` chạy nền trong `main` (kiểm 4 lần mỗi ttl, 5–60 s).
- Cấu hình toàn server: `TRADAR_IDLE_TTL` (mặc định 30 phút); plugin `setup{ idle_ttl }` truyền qua env lúc spawn. Protocol 1.12: `version.idle_ttl_secs`.
- Plugin không đổi logic: poll `status` thấy `not connected` thì bỏ cờ connected; `execute` gặp lỗi đó tự `connect` lại rồi thử lại sẵn có.

Chưa làm: TTL riêng theo từng connection (cần thêm field vào `SavedConnection`, đụng ~26 chỗ dựng struct và TUI); áp cho connection của Kafka/RabbitMQ/HTTP/Socket/gRPC (session nằm trong extension, không qua `state.drivers`); giải phóng cursor kết quả cũ (RAM do `MAX_CURSOR_BYTES` giữ, tách khỏi connection).
