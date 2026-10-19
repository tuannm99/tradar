# Cửa sổ kết quả: layout và zoom (2026-10-31)

Yêu cầu: "buffer kết quả có thể scale rộng ra được không, ít phải nửa buffer hiện có".

## Đã làm

- `<leader>rz` / `:TradarResultsZoom` (cả trong buffer SQL lẫn buffer kết quả): phóng cửa sổ kết quả ra toàn màn hình và nhảy vào đó; bấm lại trả về đúng cỡ cũ và quay về cửa sổ editor. Layout `tab` thì kết quả vốn đã chiếm cả màn hình nên chỉ báo.
- `setup{ results_layout = "bottom" | "right" | "tab" }` (mặc định `bottom`, như trước), `results_height` (mặc định 15), `results_width` (mặc định nửa màn hình, cho `right`).
- Sửa một hồi quy do ghi chú "running…" (cao 4 dòng) mở cửa sổ trước: kết quả đến sau dùng lại cửa sổ 4 dòng đó nên cửa sổ kết quả bị bé. Cửa sổ "dải mỏng" giờ được đánh dấu (`w:tradar_strip`) và đóng lại để mở đúng layout khi kết quả thật tới.

## Kiểm chứng

Smoke test Neovim headless + `tradar-server` thật + SQLite, ba layout: `bottom` đúng `results_height`; `right` kết quả nằm cạnh editor; `tab` ở tab riêng; zoom lấp đầy màn hình và focus, zoom lần hai trả đúng cỡ và quay về editor.
