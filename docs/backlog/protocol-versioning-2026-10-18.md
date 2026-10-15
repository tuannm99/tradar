# Phiên bản hoá protocol RPC (2026-10-18)

Một trong ba mục "đóng gói" còn mở trong roadmap (cùng socket path dài và Windows named pipe, cả hai vẫn để sau — chưa có nhu cầu thật). Vấn đề cụ thể: `tradar-server` và plugin Neovim thường ship từ cùng một commit (một monorepo), nhưng `rpc.lua`'s `M.server_cmd` lại ưu tiên `$PATH` trước một build local trong repo (`cmd = M.server_cmd(opts)` — explicit option, rồi PATH, rồi mới tới `<repo>/target/{release,debug}`) — nếu máy có sẵn một `tradar-server` cài qua package manager hoặc build cũ còn sót trên `$PATH`, plugin mới tự động nói chuyện với một server cũ hơn mà không hề biết. Trước đợt này protocol JSON-RPC hoàn toàn không có khái niệm version nào — không có cách nào client phát hiện lệch pha ngoài việc gặp lỗi khó hiểu (field thiếu, method không tồn tại) đúng tại chỗ dùng, sau khi đã hỏng việc.

## Thiết kế

- **`protocol::PROTOCOL_VERSION: (u32, u32)`** (major, minor) trong `crates/tradar-server/src/protocol.rs` — quy ước bump giống semver nhưng chỉ áp cho bề mặt JSON-RPC (không phải crate version):
  - **Bump major** khi một client đã viết sẵn có thể hiểu sai nếu tin vào hình dạng cũ: method bị xoá, param bắt buộc thêm mới hoặc đổi kiểu, field kết quả bị xoá hoặc đổi kiểu, ý nghĩa một lỗi đổi.
  - **Bump minor** khi thêm thuần tuý và an toàn để bỏ qua: method mới, param/field tuỳ chọn mới, hành vi đổi mà client không cần sửa gì vẫn chạy đúng.
  - Bắt đầu ở `(1, 0)`, không phải `(0, 1)`: quy ước này chưa từng tồn tại trước đây nên không có lịch sử version cũ để tính, và nhìn lại toàn bộ method đã thêm từ trước tới giờ (`grpc.*`/`kafka.*`/`socket.*`/`rabbitmq.*`/`http.*`, field `truncated`/`field_order` trên kết quả...) đều mang hình dạng "minor" nếu quy ước này có từ đầu — bản này là đường gãy đầu tiên thật sự breaking-change-shaped trong lịch sử bề mặt wire, xứng đáng là `1.0`.
- **Method mới: `version`** → `{"major": N, "minor": M}`, không tham số. Không nằm sau feature gate nào (khác gần như mọi method khác trong `server.rs`), để cả bản build tối giản nhất (`--no-default-features --features postgres,sqlite`) vẫn trả lời được — cùng nhóm "core/protocol-level" với `connectors.list`/`connections.list`/`shutdown`, nên đặt tên trần, không tiền tố `<driver>.`.
- **Cố ý chỉ cảnh báo, không chặn bất cứ lời gọi nào** — khác hẳn kiểu "version gate cứng" của một API công khai nhiều client độc lập cùng gọi vào. Đây là công cụ một người dùng cục bộ, server/plugin thường cùng một commit; lệch version là trường hợp hiếm (build riêng rẽ, `$PATH` trỏ tới bản khác), không phải chuyện thường gặp cần chặn mọi request để phòng. Đúng tinh thần đã có sẵn ở chỗ khác trong hệ thống — liveness ping (`QueryEngine::tick`, mỗi 15s) chỉ gắn badge "disconnected" khi mất kết nối, không chặn dùng tiếp.

## Plugin (`rpc.lua`)

- `CLIENT_PROTOCOL_MAJOR = 1` — hằng số local, đồng bộ **tay** với `protocol::PROTOCOL_VERSION.0`, bump cùng lúc khi phía server bump major (không có cơ chế tự động đồng bộ nào — một dòng doc comment ở mỗi phía trỏ sang phía kia).
- `check_version()` — chạy đúng **một lần mỗi khi kết nối mới mở** (gọi từ nhánh thành công của `attempt` trong `M.ensure`, không phải mỗi request): gọi `version`, so khớp `major`.
  - Lỗi với code `-32601` (`METHOD_NOT_FOUND`, copy từ `protocol::METHOD_NOT_FOUND`) → hiểu là server cũ tiền-version (major ngầm định `0`, vì không bản nào từng có method `version` trước khi field này tồn tại).
  - `major` trả về khác `CLIENT_PROTOCOL_MAJOR` → cảnh báo.
  - Cả hai trường hợp: `vim.notify(..., WARN)` đúng một lần (`warned_version` flag, reset khi đóng/mất kết nối để lần kết nối lại sau được check lại). Không giữ `done()` của `M.ensure` lại chờ — kết nối vẫn dùng được ngay trong lúc check chạy song song.
- `on_line`'s callback shape mở rộng thêm tham số thứ 4 (`code`, lấy từ `msg.error.code`) để client phân biệt "method không tồn tại" với lỗi khác mà không cần parse chuỗi tiếng Anh của `message` — trước đó callback chỉ có `(err, result, data)`. Thêm tham số ở cuối là thay đổi tương thích ngược (Lua bỏ qua return value/arg thừa không ai nhận).
- `M.request_sync` cũng trả thêm `code` làm giá trị thứ ba — dùng ở `:checkhealth`.

## `:checkhealth tradar`

Thêm một mục `ok`/`warn` cho version ngay sau "server reachable", trước "saved connections" — cùng cách các mục khác trong `health.lua` đã làm (socket path dài, không tìm thấy binary, module optional thiếu...). `warn` kèm gợi ý sửa cụ thể: rebuild `tradar-server`, hoặc cập nhật plugin, tuỳ bên nào cũ hơn.

## Test

- `version_reports_the_compiled_in_protocol_version` (`crates/tradar-server/src/server.rs`'s own `mod tests`) — không cần feature connector nào (`Server::new(HashMap::new(), None)` đủ, giống `is_ddl`'s test bên cạnh), xác nhận method `version` trả đúng hằng số `PROTOCOL_VERSION` đã compile vào binary. Không đặt trong `tests/rpc_*.rs` vì mọi file đó đều `#![cfg(feature = "...")]`-gated theo một connector cụ thể — `version` thì ngược lại, phải đúng ngay cả khi *không* feature nào bật.
- `cargo build`/`clippy --all-targets -- -D warnings`/`fmt --all -- --check` sạch ở ba cấu hình quen thuộc (workspace mặc định, `-p tradar-server --features kafka`, `--no-default-features --features postgres,sqlite`).
- `make test-unit` xanh.

## Chưa làm / cố ý để sau

- Minor version chưa được dùng để feature-detect gì cả (client chỉ so `major`) — chưa có nhu cầu: mọi method hiện có đều hoặc tồn tại hoặc không (method-not-found đã là tín hiệu đủ rõ), chưa tới lúc một client cần hỏi "server có hỗ trợ field X trong method Y không" mà không thử gọi trước.
- Không có cơ chế tự động đồng bộ `CLIENT_PROTOCOL_MAJOR` (Lua) với `PROTOCOL_VERSION.0` (Rust) — vẫn là một hằng số chép tay ở mỗi phía, giống nhiều hằng số dùng-chung-ý-nghĩa khác trong repo này (ví dụ `PEEK_COUNT` của RabbitMQ lặp lại ở cả TUI và server). Một build script sinh hằng số Lua từ Rust là khả thi nhưng không đáng công cho một con số thay đổi hiếm.
- Hai mục còn lại của "đóng gói" (socket path dài, Windows named pipe) vẫn mở, không đụng tới trong đợt này.
