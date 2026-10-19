# gRPC: metadata và streaming (2026-10-31)

Trước đó gRPC chỉ có unary và không gửi được header. Yêu cầu: "Làm gRPC metadata và streaming".

## Đã làm

- **Metadata request**: `grpc.invoke` và `grpc.stream.start` nhận `metadata` (object `{tên: giá trị}`, ASCII, key tự về chữ thường; key `-bin` bị từ chối rõ ràng). Response unary trả thêm `headers`; stream đẩy `headers`/`trailers` như các item.
- **Streaming** qua `grpc.stream.start/send/close/stop`, đẩy kết quả bằng notification `grpc.stream.message` `{subscription, kind: headers|message|trailers|sent|end|error}` — cùng khuôn với Socket tail (`Subscriptions`, dọn khi client ngắt kết nối). Ba dạng: server-streaming (`request` là message duy nhất), client-streaming (message đầu tuỳ chọn, `send` thêm, `close` = half-close, reply duy nhất), bidi.
- `grpc.methods` báo `client_streaming`/`server_streaming`; `grpc.invoke` từ chối method streaming và `grpc.stream.start` từ chối unary, cả hai chỉ sang đúng method.
- Protocol `1.6` (thuần thêm).
- Plugin: dòng `# tên: giá trị` ở đầu buffer request là metadata; picker ghi `[server|client|bidi stream]`; response của stream là transcript chỉ-đọc (`‹`/`›`/`#`, dùng `tradar.tail`); `<leader>rr` bắt đầu/gửi thêm, `<leader>rc` (`:TradarGrpcClose`) đóng chiều gửi, `<leader>rx` dừng; đóng buffer request dừng stream.

## Kiểm chứng

- `tests/rpc_grpc.rs` (fixture `greeter.proto` thêm `Countdown`/`Collect`/`Chat`): metadata đi và về, từ chối chéo unary/stream, server-stream (3 reply + end), client-stream (a+b sau close, gửi sau half-close bị từ chối), bidi (echo, ngắt client dọn stream). `ClientConnection::with_inbox` mới cho test đọc notification.
- Smoke test Neovim headless với `tradar-server` thật + server gRPC mẫu qua reflection: unary có metadata, 3 loại stream, `rx` dừng — đều đạt.

## Chưa làm

- Key metadata `-bin`, metadata riêng cho mỗi message.

## Bổ sung: TLS và deadline

- Target `https://…` bật TLS (`tonic` features `tls` + `tls-webpki-roots`, root CA webpki đi kèm); chưa có CA riêng/mTLS. Chỉ test được trong sandbox là handshake thất bại có lỗi (không treo) khi trỏ `https://` vào server plaintext — chưa thử với server TLS thật.
- `timeout_ms` ở `grpc.invoke`/`grpc.stream.start` → `grpc-timeout`. Plugin: dòng `# deadline: 5s|500ms|2m` ở đầu buffer request. Server tonic trả `CANCELLED` khi hết hạn, server Go/Java trả `DEADLINE_EXCEEDED`.

## Bổ sung: gợi ý từ comment `.proto` (protocol 1.7)

`grpc.skeleton` nhận `comments: true` → thêm dòng `// …` trên mỗi trường có doc comment (`load_proto` giờ biên dịch kèm source info qua `protox::Compiler`; reflection không mang comment nên chỉ `.proto` nạp tay mới có), liệt kê giá trị enum và các nhánh `oneof`. Plugin luôn yêu cầu `comments` và bỏ dòng `//` đứng riêng khi gửi. Test: bỏ các dòng `//` thì ra đúng khung thường.

## Bổ sung: CA riêng, mTLS, `domain`

Tuỳ chọn TLS nằm trong query của `target` (không đổi schema `SavedConnection`): `ca`, `cert`+`key`, `domain`; `split_target` tách chúng khỏi URI, tham số khác được giữ nguyên. Dùng với target không phải `https://` thì báo lỗi. Test thật (rcgen sinh CA/server/client cert, server tonic có TLS): CA riêng + `domain` chạy được, thiếu CA thì từ chối, server đòi client cert thì thiếu cert bị từ chối còn có cert chạy được.
