# Đổi tên `tradar-connector-api` → `tradar-connector-spi` (user báo 2026-08-16) — xong

Hệ quả trực tiếp của việc đổi tên connector ở mục ngay trên: sau khi 9 connector thật đều mang prefix `tradar-connector-<tên>`, crate SPI (`Connector`/`Session`/`ConnectorDescriptor`) cũng đang tên `tradar-connector-api` — nhìn trong danh sách crate rất dễ tưởng nhầm đây là một connector tên "api". User báo đúng vấn đề này. Chốt tên mới qua `AskUserQuestion`: `tradar-connector-spi` (giữ prefix `tradar-connector-` nhưng đổi hậu tố `api` → `spi`, đúng thuật ngữ "SPI" đã dùng sẵn trong architecture.md để mô tả crate này — "Connector/Session là một SPI dành riêng cho connector").

Đổi cơ học, không đụng trait/API nào:
- `git mv crates/tradar-connector-api crates/tradar-connector-spi`, đổi `name` trong `Cargo.toml` của chính nó.
- Mọi `Cargo.toml` phụ thuộc nó (10 crate: `tradar-app`, `tradar-query-workbench`, cả 9 connector) đổi cả path lẫn tên package.
- Mọi `use tradar_connector_api::...` trong `.rs` (14 file) đổi thành `tradar_connector_spi` — Rust tự đổi `-` thành `_` nên identifier trong code đổi theo tên crate.
- `Makefile`: target `test-connector-api` → `test-connector-spi`.
- Doc comment tham chiếu tên cũ bằng chữ (không qua Cargo nên compiler không bắt lỗi): `CLAUDE.md` (2 chỗ, thêm luôn 1 câu giải thích lý do đổi tên ngay tại chỗ liệt kê 13 crate — để ai đọc lần đầu không phải tự hỏi tại sao lại là `-spi` không phải `-api`), `docs/architecture.md` (9 chỗ, gồm cả cây ASCII "Bố cục workspace" và đoạn giải thích quyết định tách crate gốc 2026-08-08 — chêm thêm một câu **Đổi tên 2026-08-17** ngay sau đoạn đó để không phải viết lại lý do tách, chỉ ghi thêm lý do đổi tên).
- Verify: `cargo build --workspace`/`fmt --check`/`clippy -D warnings`/`make test-unit` sạch sau khi đổi.

