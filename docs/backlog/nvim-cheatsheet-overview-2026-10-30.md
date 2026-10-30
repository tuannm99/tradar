# Cheatsheet xem tổng quan trong một buffer (2026-10-30)

Telescope chỉ hợp để tìm cái đã biết tên; muốn nhìn cả bảng (có gì, nhóm nào) thì phải mở từng entry.

- `:TradarCheatsheetFile` / `<leader>rK`: dựng cả cheatsheet thành một buffer `tradar://cheatsheet/<connection>` (filetype `tdb`) trong tab riêng: `## nhóm`, `### tên` và một block ` ```<dialect> <connection> ` cho mỗi entry (`★` cho entry của người dùng). Vì là `.tdb` nên `rr` chạy block ngay trên connection ghi ở fence, `${tên}` vẫn được hỏi, màu/markdown như `.tdb`.
- Gập bằng `foldexpr` (`cheatsheet.foldexpr`: `## ` mức 1, `### ` mức 2), mặc định mở hết (`foldlevel=99`); `zM` còn tiêu đề, `zR` mở lại.
- Dùng cùng dữ liệu với picker (method `cheatsheet` của server, gồm `cheatsheet.toml`); trong workbench/`.http`/block `http`|`grpc` thì dùng `reqcheats.lua` (fence ` ```http <conn> `). Gọi lại thì về đúng buffer; chưa sửa gì thì vẽ lại, đã sửa thì giữ nguyên.
- Không đổi server/protocol. Test: `cheatsheet_spec` (mọi entry có mặt, block đúng dạng, gập, `rr` chạy từ overview), `restbench_spec` (bản request).
