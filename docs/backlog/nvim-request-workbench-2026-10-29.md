# Workbench request cho HTTP và gRPC (2026-10-29)

Vấn đề: HTTP và gRPC lệch khỏi cách dùng các DB khác. HTTP chỉ có **một** buffer request cho mỗi connection, gRPC mở một tab JSON riêng cho mỗi method; response nằm ở split riêng (`tradar://http`, split gRPC) nên không có tab theo target, history, pin hay diff; chọn method gRPC là mở tab ngay thay vì sinh câu vào workbench rồi `rr`.

Quyết định: một cú pháp chung kiểu file `.http` (REST Client), nhiều request trong một buffer, response vào result tabs.

- **Cú pháp** (`lua/tradar/reqparse.lua`, thuần, không RPC): block cách nhau bằng `###` (tiêu đề tuỳ chọn); HTTP `METHOD url` / `Tên: giá trị` / dòng trống / body; gRPC `GRPC service/Method` / metadata / `deadline: 5s` / dòng trống / body JSON. `@tên = giá trị` khai báo biến cho cả file, `{{tên}}` thay trong dòng request, header và body; có `{{$uuid}}`, `{{$timestamp}}`, `{{$isoTimestamp}}`, `{{$env.TÊN}}`. Biến chưa khai báo → báo tên, không gửi gì. Tên header khớp token RFC 7230 nên body dính ngay dòng request (`{"a":1}`) bị báo thiếu dòng trống thay vì bị đọc nhầm là header.
- **Workbench** (`lua/tradar/restbench.lua`): mỗi connection một buffer scratch (`filetype=http`), `rr` chạy block dưới con trỏ, `rx` (HTTP: ngừng chờ; unary: `cancel`; stream: dừng), `rc` đóng chiều gửi, `ry` copy `curl`/`grpcurl`. `http.lua` và `grpc.lua` chỉ còn connect/picker/proto; chọn method trong picker gRPC **thêm block** (kèm `grpc.skeleton`) thay vì mở tab, chọn lại thì nhảy tới block cũ. Stream giữ nguyên cách cũ (đẩy qua `grpc.stream.message`), chỉ khác đích: transcript ghi vào result tab.
- **Result tabs**: kind mới `text` trong `results.lua` (`show_text`, `text_append` gom repaint 40 ms, footer riêng). Tab đặt theo `METHOD /path` (bỏ query string và host) hoặc tên method gRPC (`reqparse.slot`), nên lần chạy lại thay tab và dồn lần trước vào history (`[R`/`]R`); `rD` diff theo dòng bằng `vim.diff`; `rR` gửi lại (`entry.rerun`); `rH` hiện/ẩn status + header; tab bar có badge status (`200`, `404`, `OK`, `⇄`). `rF`/`ro`/`rA` không áp dụng (không có lưới).
- **Body JSON** (`lua/tradar/jsonfmt.lua`): thụt lề lại chính văn bản server trả, **giữ thứ tự key và chữ số** (không qua `vim.json.decode` + encode nên số lớn không mất độ chính xác); màu bằng `jsonhl` (thêm tham số `from` để bỏ qua phần header). Body không phải JSON hiện nguyên văn.
- **Không đổi server / protocol**: toàn bộ là plugin; `http.send`, `grpc.invoke`, `grpc.stream.*`, `grpc.skeleton` giữ nguyên.

Bỏ: buffer `tradar://http`, split response gRPC, buffer request gRPC một-method-một-tab, `:TradarGrpcSend`/`Cancel`/`Close` theo buffer (giờ buffer-local trong workbench). Lệnh mới: `:TradarRequests [tên]`, `:TradarGrpcSend [tên]`.

Test: `rest_spec` (parser, biến, tab key, curl/grpcurl, jsonfmt), `restbench_spec` (server HTTP thật bằng python: block, biến, tab, history, diff, rerun, 404, text, copy curl; gRPC với `rpc.request` stub: thêm block từ method, unary với metadata + deadline, stream tới khi kết thúc).

Làm tiếp cùng ngày (phần "chưa làm" của bản đầu):

- **Biến theo connection**: `setup{ request_vars = { ['*'] = {...}, [conn] = {...} } }` (plugin, không đụng `connections.toml` vì `SavedConnection` chỉ có name/driver/target và lần lưu sẽ bỏ field lạ); thứ tự ưu tiên `'*'` < connection < `@x = …` trong file. `@x = ` để trống là dòng nhắc, không phải giá trị (dùng thì báo chưa khai báo).
- **Gợi ý** (`omnifunc` của workbench): động từ, header và giá trị thông dụng, `{{biến}}`, và dòng `GRPC` hỏi `grpc.services` / `grpc.methods` đồng bộ (`request_sync`, 1,5 s).
- **Cheatsheet** (`reqcheats.lua`, `rk`): danh sách tĩnh trong plugin chứ không qua server — HTTP/gRPC không phải `QueryDriver` nên không có `cheatsheet` method; test parse mọi entry thành request hợp lệ.
- **Lọc response** (`rF`, `jsonpath.lua`): đường dẫn kiểu jq hoặc chuỗi con theo dòng; `render.pretty` thôi viết `/` thành `\/`.

Đã thử rồi bỏ: picker endpoint từ OpenAPI (`:TradarHttpEndpoints`) — người dùng không cần, chỉ cần xuất curl (`ry` / `:TradarCopyCurl`).

Chưa làm: lọc phía server cho response lớn, cheatsheet của riêng người dùng cho HTTP/gRPC (hiện chỉ cần giữ file workbench).

Làm tiếp 2026-10-30: request không còn bó trong workbench.

- **Chỗ viết request** (`restbench.context`): workbench, buffer filetype `http` (`*.http` được đăng ký filetype; `# tradar: tên` là alias), block ` ```http [conn] ` / ` ```grpc [conn] ` của `.tdb`. `rr`/`rx`/`ry`/`rk`, omnifunc và blink đều định tuyến theo `restbench.applies(buf)` nên cùng phím với file query; block `.tdb` chỉ dùng `@var` và request của chính nó, biến đọc từ cả buffer. Khác SQL: không rơi về "connection chọn gần nhất" (thường là database).
- **URL thẳng, không connection**: server nhận tên connection `@đích` cho `http.*`/`grpc.*` (`Ctx::saved`): `@` cho HTTP (URL đầy đủ), `@host:port` cho gRPC; plugin dùng nó khi request có URL đầy đủ / `GRPC host:port service/Method` mà không có alias. Protocol 1.16 (cộng thêm); tên connection không có `@` vẫn phải có trong `connections.toml`.
- Thêm `:TradarGrpcCancel` / `:TradarGrpcClose` toàn cục (theo con trỏ) vì trong `.tdb`/`.http` `rc` vẫn là "chọn connection".
- Test: `restbench_spec` (file `.http` URL thẳng / alias / lỗi thiếu connection, `.tdb` trộn block http/sql, curl, gợi ý, cheatsheet vào trong block, gRPC thẳng), `rest_spec` (cú pháp `GRPC host:port …`), unit test `Ctx::saved`.
- **Màu** (`reqhl.lua`, `TradarReq*`): quét từng dòng đặt extmark (như `jsonhl`), không cần grammar `http`; áp dụng cho workbench, `*.http`, và chỉ các block ` ```http `/` ```grpc ` của `.tdb`; vẽ lại ~30 ms sau khi sửa. Body JSON dùng `jsonhl.tokens`. Test `reqhl_spec`.

Làm tiếp 2026-10-30: **không kiểm tra chứng chỉ TLS** (`insecure`).

- HTTP: `?insecure=1` ở cuối `target` (tách khỏi base URL bằng `tradar_connector_http::split_insecure`), `reqwest::ClientBuilder::danger_accept_invalid_certs`. gRPC: `TlsOptions.insecure`; tonic không tắt được verify nên `grpc/insecure.rs` tự dựng kết nối: rustls với `ServerCertVerifier` nhận mọi chứng chỉ (chữ ký handshake vẫn được kiểm), ALPN `h2`, dial bằng URL `http://` + `.origin(https://…)` vì tonic từ chối endpoint https không có TLS config; vẫn gửi được client certificate.
- `http.connect`/`grpc.connect` trả thêm `insecure` (protocol 1.17) và server ghi `warn` vào log; plugin báo `TLS certificate is NOT verified` một lần và ghi `TLS not verified` ở footer tab response.
- Request viết thẳng vào URL: dòng `@insecure = 1` trong file → connection tạm `@?insecure=1`.
- Test: `rpc_grpc` (insecure bỏ qua CA, kèm client cert, báo ở `connect`), unit test `split_target`/`split_insecure`, `tls_spec` (server HTTPS tự ký thật bằng openssl + python).
- Proxy: HTTP/Elasticsearch/RabbitMQ/ClickHouse dùng `reqwest` nên theo biến môi trường `HTTP_PROXY`/`HTTPS_PROXY`/`ALL_PROXY`/`NO_PROXY` của tiến trình `tradar-server`; gRPC (tonic) chưa hỗ trợ proxy. Chưa có cấu hình proxy riêng cho từng connection.
