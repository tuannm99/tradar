# gRPC: sinh khung request

Việc "chưa làm" của `grpc-nvim-plugin-2026-10-14.md`: buffer request mở trên `{}` nên phải biết trước shape của message.

- Server: `grpc.skeleton {connection, service, method}` → `{skeleton: <text>}`. `grpc::skeleton` duyệt `MessageDescriptor` của input và in JSON thụt lề **bằng tay** (không qua `serde_json::Value`, vì đó không hứa giữ thứ tự trường) theo thứ tự khai báo, tên trường theo `json_name` (camelCase, form proto3 JSON). Giá trị: số `0`, `bool false`, `string`/`bytes` `""`, enum = giá trị đầu, `repeated` `[]`, `map` `{}`, message lồng được điền đệ quy.
- Điểm cần quyết: `oneof` chỉ lấy thành viên đầu (nhiều hơn một thì request không hợp lệ khi deserialize); message tự tham chiếu (`Node.next`) dừng ở lần lặp thứ hai (`{}`) và có trần độ sâu; well-known types có dạng JSON riêng (`Timestamp` `"1970-01-01T00:00:00Z"`, `Duration` `"0s"`, wrappers, `Struct`/`Any`/`Empty` `{}`, `Value` `null`).
- Plugin: `open_request` mở buffer trên `{}` rồi thay bằng khung khi server trả lời — chỉ nếu buffer vẫn còn đúng `{}` (không đè thứ người dùng đã gõ), im lặng nếu server cũ không có method. Protocol `1.4`.
- Kiểm chứng: unit test trên một `.proto` có lồng/repeated/map/enum/oneof/optional/Timestamp/tự tham chiếu (kể cả: khung deserialize được vào đúng message của nó); test RPC với toy server greeter; stub-rpc cho buffer Neovim.
- Chưa làm: gợi ý giá trị theo comment trong `.proto`; metadata/header; streaming (xem scope v1 trong `docs/architecture.md`).
