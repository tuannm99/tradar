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

- Key metadata `-bin`, metadata riêng cho mỗi message, CA riêng/mTLS.
- Gợi ý trường từ comment trong `.proto`.

## Bổ sung: TLS và deadline

- Target `https://…` bật TLS (`tonic` features `tls` + `tls-webpki-roots`, root CA webpki đi kèm); chưa có CA riêng/mTLS. Chỉ test được trong sandbox là handshake thất bại có lỗi (không treo) khi trỏ `https://` vào server plaintext — chưa thử với server TLS thật.
- `timeout_ms` ở `grpc.invoke`/`grpc.stream.start` → `grpc-timeout`. Plugin: dòng `# deadline: 5s|500ms|2m` ở đầu buffer request. Server tonic trả `CANCELLED` khi hết hạn, server Go/Java trả `DEADLINE_EXCEEDED`.
