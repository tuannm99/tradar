# Lịch sử, ghim, lọc/sắp xếp, diff kết quả (2026-10-24)

Tiếp theo tab theo đích (`nvim-result-tabs-2026-10-23.md`).

- **Lịch sử theo tab:** khi một kết quả thay slot cùng khoá, lần chạy cũ chuyển vào `entry.history` (tối đa `results_history` = 5, mới nhất trước), cursor của nó đóng như trước. `[R`/`]R` (`M.step_history`) đặt `state.result` thành lần cũ trong cùng buffer; `entry.hidx` nhớ vị trí, `show_entry` đặt lại. Refresh/watch thay slot theo danh tính (`refresh.replacing`) và **không** thêm lịch sử (một watch 5 giây sẽ tràn).
- **Ghim:** `entry.pinned`; `log_result` bỏ qua slot đã ghim khi tìm cùng khoá (kết quả mới thành slot riêng) và khi chọn slot để loại.
- **Lọc/sắp xếp** (`gridview.lua`, thuần): chỉ trên dòng đã tải. `r.filter`/`r.sort` + `r.vrows` (table) hay `r.tvrows` (documents dạng bảng); `tabular`/`grid_of` trả dòng đang hiện nên yank/`gK`/`gd` đúng dòng. Lưới chỉ đọc khi bật vì dòng sửa được theo vị trí dòng trong `r.rows`. Export giữ toàn bộ dòng. Refresh giữ bộ lọc. Đường paint tăng dần (`new_from`) bị bỏ qua khi bật.
- **Diff** (`rdiff.lua`, thuần): so theo khoá (`edit.keys`, hoặc cột chung đầu tiên), khoá lặp ghép theo thứ tự; cột chỉ ở một bên được liệt kê. Báo cáo ở buffer `tradar://diff`.
- **Tham số `${}`:** `<Tab>` trong câu hỏi gợi ý 8 giá trị đã dùng (`state.param_history`).
- **Kiểm tra đích trên các backend** (`targets_spec`, từ các `crud_snippet`/browse command của connector): tìm ra `GET key` của Redis bị nhận nhầm là Elasticsearch, khoá Redis có dấu cách/nháy (`GET "a b"`) và tên `"schema"."table"` có dấu nháy. Quy tắc phân biệt: `GET` chỉ là ES khi có `/` đầu hoặc `_endpoint`.

## Chưa làm

- Lọc/sắp xếp đẩy xuống server (`WHERE`/`ORDER BY`) cho kết quả lớn.
- Diff cạnh nhau (hai cửa sổ) và diff Mongo theo cấu trúc lồng nhau (hiện so các cột đã phẳng hoá).
