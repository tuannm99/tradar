# Màu của view JSON bị vỡ (2026-10-28)

Triệu chứng: buffer kết quả ở view JSON lúc có màu lúc không, có chỗ tô sai, tuỳ dữ liệu.

- **Nguyên nhân:** màu do `syntax json` (regex của Vim, có giới hạn thời gian `redrawtime`): kết quả lớn / lồng sâu thì Vim bỏ giữa chừng; trên 3000 dòng thì code chủ động tắt hẳn (`synmaxcol = 200` còn cắt màu ở cột 200). Thử riêng chuỗi dài (> 200 cột) thì màu các dòng sau vẫn đúng, nên không phải thủ phạm.
- **Sửa:** `lua/tradar/jsonhl.lua` quét từng dòng và đặt màu bằng extmark (`TradarJsonKey|String|Number|Const|Func`, mặc định link `Label|String|Number|Boolean|Function`, `default = true` nên colorscheme ghi đè được). Chi phí tuyến tính theo số dòng, không phụ thuộc cấu trúc; tới 100000 dòng; `ObjectId(`/`ISODate(` là tên hàm chứ không phải lỗi. `syntax` của buffer để trống (không còn regex engine). Dòng trống và footer cuối không tô.
- **Test:** `jsonhl_spec` (token từng loại, chuỗi escape, dòng rất dài, 20000 dòng, chuyển qua lại bảng/JSON).
