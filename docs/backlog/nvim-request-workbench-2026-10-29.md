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
- **Endpoint từ OpenAPI** (`openapi.lua`, `:TradarHttpEndpoints`): thử `/openapi.json`, `/swagger.json`, `/v3/api-docs`, `/v2/api-docs`, `/api-docs` qua `http.send`; liệt kê bằng `vim.ui.select`; block sinh ra có `@id = `, `{{id}}`, query/header bắt buộc, body mẫu từ schema (`$ref`, `allOf`, `oneOf` nhánh đầu, enum, format, độ sâu 4). Chỉ JSON.
- **Gợi ý** (`omnifunc` của workbench): động từ, header và giá trị thông dụng, `{{biến}}`, và dòng `GRPC` hỏi `grpc.services` / `grpc.methods` đồng bộ (`request_sync`, 1,5 s).
- **Cheatsheet** (`reqcheats.lua`, `rk`): danh sách tĩnh trong plugin chứ không qua server — HTTP/gRPC không phải `QueryDriver` nên không có `cheatsheet` method; test parse mọi entry thành request hợp lệ.
- **Lọc response** (`rF`, `jsonpath.lua`): đường dẫn kiểu jq hoặc chuỗi con theo dòng; `render.pretty` thôi viết `/` thành `\/`.

Chưa làm: lọc phía server cho response lớn, đọc OpenAPI dạng YAML, cheatsheet của riêng người dùng cho HTTP/gRPC (hiện chỉ cần giữ file workbench).
