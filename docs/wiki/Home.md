# Tradar — hướng dẫn sử dụng

Tradar là công cụ khám phá và truy vấn database ngay trong terminal/Neovim: một giao diện thống nhất để kết nối, browse và query nhiều loại backend, vẫn dùng **ngôn ngữ query gốc** của từng cái (SQL, Mongo shell, Query DSL của Elasticsearch...), không tự bịa ra ngôn ngữ riêng.

Có hai cách dùng, cùng chia sẻ một file connection và một bộ connector:

| | Mô tả | Trạng thái |
|---|---|---|
| **Neovim** (`tradar.nvim` + `tradar-server`) | Editor là buffer Neovim bình thường; một process nền giữ connection, nhiều session Neovim dùng chung | **Giao diện chính** |
| **TUI** (`tradar`) | Ứng dụng terminal độc lập | Đóng băng — chỉ sửa lỗi, không thêm tính năng mới |

## Bắt đầu nhanh (Neovim)

1. [Cài đặt](Cai-dat) — build `tradar-server`, thêm plugin.
2. [Kết nối](Ket-noi) — khai báo connection trong `~/.config/tradar/connections.toml`.
3. Mở một file `.sql`, gõ dòng đầu `-- tradar: tên-connection`, đặt con trỏ vào câu lệnh, bấm `<leader>rr`.

## Mục lục

- [Cài đặt](Cai-dat)
- [Kết nối](Ket-noi) — định dạng `connections.toml`, `target` của từng loại backend, gắn file với connection
- [Truy vấn SQL trong Neovim](Truy-van-SQL) — chạy, kết quả, sửa dòng, schema, completion, an toàn
- [Mongo, Redis, Elasticsearch](Mongo-Redis-Elasticsearch)
- [Phím tắt](Phim-tat) — mọi phím và lệnh theo từng loại buffer
- [File `.tdb`](File-tdb) — nhiều dialect/connection trong một file
- [Kafka, RabbitMQ, HTTP, Socket, gRPC](Kafka-RabbitMQ-HTTP-Socket-gRPC) — các backend không có ngôn ngữ query
- [TUI](TUI) và [cấu hình `config.toml`](TUI#cấu-hình)
- [Xử lý sự cố](Xu-ly-su-co)

> Đây là hướng dẫn **dùng**. Thiết kế hệ thống nằm ở [`docs/architecture.md`](../architecture.md), việc đang mở ở [`docs/roadmap.md`](../roadmap.md), lịch sử từng tính năng ở [`docs/backlog/`](../backlog/README.md).
