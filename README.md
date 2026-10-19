# Tradar

Công cụ khám phá và truy vấn database trong terminal, theo tinh thần [LazyGit](https://github.com/jesseduffield/lazygit) / [k9s](https://k9scli.io/) nhưng dành cho database. Tradar giữ nguyên ngôn ngữ query gốc của từng backend — SQL thật, Mongo Shell JS thật, Elasticsearch Query DSL thật — thay vì bịa ra một ngôn ngữ chung.

**Giao diện chính: Neovim + `tradar-server`.** `tradar-server` là server JSON-RPC (unix socket) giữ mọi connection đang mở; Neovim là lớp soạn thảo/điều hướng/vim-motion bên trên, và nhiều phiên Neovim dùng chung một server. TUI (`tradar`) đã **đóng băng**: chỉ sửa lỗi, không thêm tính năng.

## Backend

PostgreSQL, MySQL/MariaDB, SQLite, MongoDB, Elasticsearch, Redis, Cassandra, ClickHouse (có ngôn ngữ query) · Kafka, RabbitMQ, HTTP, Socket (TCP thô), gRPC (không có ngôn ngữ query; gRPC chỉ có ở Neovim/server).

## Bắt đầu nhanh

```bash
cargo build -p tradar-server --release    # server (cần Rust, edition 2024)
```

Rồi thêm plugin vào Neovim (lazy.nvim), thêm connection và chạy câu đầu tiên — hướng dẫn từng bước ở [`docs/wiki/Cai-dat.md`](docs/wiki/Cai-dat.md).

Chỉ cần vài backend (tránh kéo `rdkafka`, cần `cmake`/`gcc`/`libcurl-dev`)?

```bash
cargo build -p tradar-server --no-default-features --features mongo,elasticsearch
```

## Tài liệu

| Muốn biết | Đọc |
|---|---|
| Cách dùng (cài đặt, kết nối, phím tắt, từng backend) | [`docs/wiki/Home.md`](docs/wiki/Home.md) |
| Tham chiếu plugin Neovim | [`nvim/README.md`](nvim/README.md) |
| Kiến trúc, protocol, trait `QueryDriver`/`Connector`/`Session` | [`docs/architecture.md`](docs/architecture.md) |
| Việc đang mở | [`docs/roadmap.md`](docs/roadmap.md) |
| Việc đã xong, vì sao, kiểm chứng thế nào | [`docs/backlog/`](docs/backlog/README.md) |
| TUI (đóng băng) | [`docs/wiki/TUI.md`](docs/wiki/TUI.md) |

## Triết lý

- **Keyboard-first, terminal-first** — mọi tính năng dùng được không cần chuột; khởi động nhanh, ít tốn bộ nhớ.
- **Ngôn ngữ query gốc** của từng backend.
- **Core không phụ thuộc database cụ thể** — mỗi connector là một crate riêng sau interface dùng chung; thêm backend mới không đụng tới phần còn lại.
- **Kết quả lớn phải nhẹ** — phân trang, không nạp thừa vào bộ nhớ.

## Phát triển

```bash
cargo test            # test (một số connector cần Docker; không có Docker: make test-unit)
cargo clippy --all-targets --workspace -- -D warnings
cargo fmt
```
