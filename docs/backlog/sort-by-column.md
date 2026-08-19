# Sort theo cột (click header) — xong (2026-08-19)

Gap #9 trong đợt so sánh DataGrip/DBeaver/Studio3T (`docs/roadmap.md`), Tier 1 -- việc đầu tiên trong thứ tự đã chốt vì rẻ và không phụ thuộc gì. Client-side trên `QueryResult` đã tải, không phải `ORDER BY` gửi lại DB -- cùng hạ tầng với filter `/` đã có.

**Kiến trúc dữ liệu**

- `ResultsComponent` thêm `sort: Option<(usize, SortDirection)>` (`SortDirection::{Asc, Desc}`). Reset về `None` trong `set_result()` (một kết quả mới có thể không còn cột đó, hoặc cột đó không còn nghĩa cũ), **giữ nguyên** trong `set_result_keeping_cursor()` -- cùng lifetime với `filter`, vì đây là refresh cùng shape chứ không phải kết quả mới.
- `sort_by_column(index)` cycle: khác cột đang sort → `Asc`; cùng cột → `Asc → Desc → None`.

**Điểm cắm dùng chung** -- gộp thành một hàm `visible_and_sorted_rows(rows, filter, sort, column_types) -> Vec<usize>` (filter trước, sort sau), gọi từ cả `ResultsComponent::visible_items()` (selection/edit/yank) lẫn `draw_table_body()` (vẽ) -- tránh đúng cái bẫy đã lường trước lúc lên plan: hai nơi tính filtered-indices độc lập mà lệch nhau.

**So sánh giá trị**: cột có `column_types[index]` biết được và là kiểu số (dùng lại `type_icon(..) == Some("#")`, đã có sẵn từ mục hiện type icon trong header) → parse `f64` rồi so số; một giá trị parse lỗi rơi về so chuỗi cho đúng cặp đó thôi, không kéo cả cột về chuỗi. Kiểu không phải số, hoặc không biết kiểu (luôn vậy với Documents-table-view, không có schema SQL) → so chuỗi. Giá trị rỗng luôn xếp cuối bất kể `Asc`/`Desc` -- xử lý **tách riêng** khỏi phần so sánh giá trị thật (`compare_cell`) rồi mới `.reverse()` theo chiều cho phần đó, vì nếu để chung một `Ordering` rồi reverse cả khối thì `Desc` sẽ đẩy giá trị rỗng lên đầu thay vì giữ ở cuối -- bug thực sự gặp lúc code, sửa bằng cách match `(a.is_empty(), b.is_empty())` trước, chỉ reverse nhánh "cả hai đều không rỗng".

**Bất biến giữ nguyên**: gutter số thứ tự đọc theo vị trí gốc trong result (sort chỉ tráo thứ tự `Vec<usize>`, không đụng `rows[]`); edit-cell/delete-row đi qua đúng index gốc nên không bị ảnh hưởng.

**Kích hoạt**

- Phím `s` trong `Context::Results` → `Command::SortColumn` (mới, `crates/tradar-core/src/keymap.rs`), tác động lên `selected_col` hiện tại.
- Chuột: `ResultsComponent::click_header(column, row) -> bool` hit-test hàng `rows_area.y - 1` (đúng vị trí header) bằng `column_spans` ghi lúc `draw()`. `QueryScreenComponent::handle_mouse_event` gọi trước `results.click(...)` trên cùng một `if`/`||` (gộp lại sau khi clippy bắt lỗi `if_same_then_else` vì cả hai nhánh chỉ set `self.focus = Focus::Results`).

**Hiển thị**: mũi tên `▲`/`▼` nối sau tên/icon cột đang sort trong header cell, tính trước khi đo `column_widths` (không phá truncate). Tiêu đề panel Results nối thêm `— sort: <col> ▲` khi đang sort và đang ở view có cột thật (`!self.columns().is_empty()` -- guard này chặn suffix hiện sai lúc đang ở JSON view dù `self.sort` vẫn còn giá trị cũ từ lần ở table view).

**Scope**: chỉ `QueryResult::Table` và `Documents` ở table-view (`t`) -- JSON view và `Affected` không có header để sort.

**Test**: `crates/tradar-query-workbench/src/components/results.rs` -- cycle asc/desc/off, đổi cột reset về asc, sort số vs chuỗi (test riêng để phân biệt: dùng số cố tình lệch thứ tự string vs numeric), giá trị thiếu xếp cuối cả hai chiều, gutter giữ đúng vị trí gốc, `set_result` xoá sort / `set_result_keeping_cursor` giữ, click header đúng cột, click vào thân bảng không kích hoạt sort. `crates/tradar-query-workbench/src/components/query_screen.rs` -- phím `s` qua `dispatch_command` thật (không chỉ qua `sort_by_column` trực tiếp).

README.md cập nhật đoạn mô tả panel Results (thêm mô tả `s`/click header cạnh đoạn `/` filter đã có).
