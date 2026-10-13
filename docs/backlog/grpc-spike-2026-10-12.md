# gRPC: spike reflection + prost-reflect (2026-10-12)

## Bối cảnh

Sau khi chốt scope v1 cho gRPC (PR #32 — đích `tradar-server`/Neovim, chỉ unary, không metadata), bước kế tiếp theo gợi ý trong `docs/architecture.md` là spike/prototype phần **reflection + `prost-reflect::DynamicMessage`** trước khi cam kết thiết kế protocol/UI chi tiết — đây là connector rủi ro cao nhất trong nhóm phi-query, chưa từng dùng kỹ thuật này trong codebase.

## Câu hỏi spike cần trả lời

Có build được một client gRPC gọi được một method mà Rust **không biết shape lúc compile-time** không — tức không dùng `tonic`/`prost` codegen cố định từ `.proto` cho service cụ thể, mà build request/đọc response hoàn toàn động từ `DescriptorPool`, giống kỹ thuật `grpcurl` (Go) làm?

## Cách làm

Crate thử nghiệm riêng (**không** nằm trong workspace `tradar`, chỉ ở scratchpad phiên làm việc — xem "Chưa làm" về việc chưa commit vào repo), server + client trong cùng một binary:

- Server thật: một service `greeter.Greeter/SayHello` tầm thường (chỉ để có gì đó invoke), build qua `tonic_build` + `protox` (pure-Rust, không cần `protoc` cài sẵn — xác nhận đúng phần "ưu tiên pure Rust" đã chốt trong thiết kế gốc), bật `tonic-reflection` (`build_v1()`).
- Client: thử cả hai đường discovery độc lập, cùng dẫn tới một `DescriptorPool` dùng được bởi cùng một hàm invoke:
  1. **Server reflection thật**: `tonic_reflection::pb::v1::server_reflection_client::ServerReflectionClient` — gọi `ListServices` (xác nhận không cần biết tên service trước) rồi `FileContainingSymbol("greeter.Greeter")` lấy `FileDescriptorProto` (bytes), decode từng cái vào `DescriptorPool::decode_file_descriptor_proto`.
  2. **Fallback đọc `.proto`**: `protox::compile([...])` ra `FileDescriptorSet` trực tiếp, `DescriptorPool::from_file_descriptor_set(fds)` — không qua network, dùng khi server không bật reflection.
- Invoke động: `MethodDescriptor::input()`/`output()` cho `MessageDescriptor` của request/response; `DynamicMessage::deserialize(desc, json)` build request từ JSON người dùng gõ; gửi qua `tonic::client::Grpc::unary` với một `Codec` tự viết (`DynamicCodec`) vì `tonic::codec::ProstCodec` có sẵn yêu cầu `T: Default` để decode (gọi `T::default()` rồi merge) — `DynamicMessage` cần một `MessageDescriptor` ngay lúc dựng nên không có `Default` chung được cho mọi message; `DynamicCodec` giữ `response_desc: MessageDescriptor`, encoder/decoder gọi thẳng `DynamicMessage::encode`/`DynamicMessage::decode(desc, buf)`. Response convert lại JSON qua `serde_json::to_value(&reply)` (dùng `impl Serialize for DynamicMessage`, feature `serde` của `prost-reflect`).

## Kết quả

Cả hai đường discovery chạy thật, cùng cho một request JSON `{"name": "..."}` invoke đúng method `SayHello` qua kết nối gRPC thật (TCP loopback), nhận lại đúng response JSON (`{"message": "Hello, ...!", "length": N}`), không có type Rust `HelloRequest`/`HelloReply` nào được dùng ở phía gọi — toàn bộ qua `DynamicMessage`. Build sạch (`protox` thay `protoc`), chạy sạch, không warning (sau khi bỏ một import thừa).

**Kết luận: kỹ thuật khả thi, rủi ro đã được derisk.** Không có bất ngờ kiến trúc nào buộc phải quay lại chốt lại scope.

## Phần tái dùng được khi code thật (tóm tắt cho lúc bắt tay)

- `DynamicCodec`/`DynamicEncoder`/`DynamicDecoder` (code ở trên) — gần như copy thẳng được, chỉ cần parameterize path/descriptor theo method người dùng chọn lúc runtime (ở server thật sẽ là theo request RPC method của client Neovim, không phải hardcode).
- Pool từ reflection: `ListServices` → `FileContainingSymbol` cho từng service người dùng mở ra trong sidebar/picker — không cần tải toàn bộ schema server trước, giống cách HTTP/Mongo chỉ load theo nhu cầu.
- Pool từ `.proto`: `protox::compile` nhận trực tiếp đường dẫn file người dùng gõ qua prompt (`L` trong thiết kế gốc) — không cần thay đổi gì so với spike.
- `invoke_unary` (build request từ JSON, gửi, parse response JSON) là chính xác hình dạng method RPC mới mà `tradar-server` sẽ cần thêm (tương đương `execute`/`fetch` hiện có nhưng cho connector phi-`QueryDriver`).

## Chưa làm

- Code spike **không** commit vào repo chính (không phải crate thật trong workspace, chỉ để trả lời câu hỏi khả thi) — giữ nguyên ở scratchpad phiên làm việc, không push lên nhánh.
- Chưa test repeated/nested message, `oneof`, `enum`, hay message lồng map — spike chỉ dùng hai field string/int32 trần. Rủi ro còn lại trước khi code thật: xác nhận `DynamicMessage::deserialize` xử lý đúng các shape phức tạp hơn (nhiều khả năng ổn, vì đây là cơ chế chính `prost-reflect` quảng cáo, nhưng chưa tự kiểm chứng).
- Chưa test server-streaming (ngoài scope v1 đã chốt) hay lỗi gRPC thật (status code khác OK) — response lỗi sẽ cần map sang format lỗi của `tradar-server` (giống `error.data {line, column}` các driver khác đã làm, nhưng gRPC không có khái niệm vị trí trong text).
- Chưa thiết kế method RPC mới cụ thể ở `tradar-server` (tên method, request/response shape JSON-RPC) hay phần Neovim plugin (hiển thị sidebar service/method, soạn request JSON, xem response) — đây là việc tiếp theo khi bắt tay code thật.
