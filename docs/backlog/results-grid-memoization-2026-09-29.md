# Cache `visible_items()`/`json_lines()` trong results grid — xong (2026-09-29)

Người dùng: "tiếp theo còn gì, t muốn tối ưu hơn là add connector" (sau khi hoàn tất connector ClickHouse + Socket cùng ngày). Chốt qua `AskUserQuestion` — trình bày phát hiện cụ thể tìm được khi rà lại `results.rs` trước, người dùng chọn làm ngay thay vì rà rộng thêm.

## Vấn đề

`ResultsComponent::visible_items()` (lọc + sort) và hàm tự do `json_lines()` (pretty-print từng document trong JSON view) bị tính lại **từ đầu mỗi lần gọi** — không có cache nào, khác hẳn `fold_cache`/`highlight_cache` đã có sẵn trong `query_editor.rs` cho đúng loại vấn đề này (rà lại hiệu năng lần trước, 2026-08-18, đã nêu rõ nguyên tắc: đừng tính lại O(kích thước kết quả) mỗi frame `draw()` — kết quả một query có thể lớn, và `draw()` chạy ~20 lần/giây lúc spinner đang quay).

Cả hai bị gọi nhiều lần mỗi frame (thanh tiêu đề đếm số dòng, gutter số thứ tự, phần vẽ bảng/JSON) **cộng thêm** mỗi lần bấm `j`/`k`/cuộn (qua `apply_move` → `cursor_count`). Với kết quả `Documents` (Mongo/Elasticsearch) lớn có filter, một lần bấm `j` từng khiến `json_lines()` serialize + pretty-print lại **toàn bộ** tập document đang hiển thị.

**Phát hiện thêm khi đọc kỹ**: `draw_table_body` (hàm tự do vẽ bảng, tách khỏi `ResultsComponent` vì lý do borrow — xem doc comment của chính nó) tự gọi `visible_and_sorted_rows(...)` **lần thứ ba**, hoàn toàn tách biệt khỏi `self.visible_items()` — nghĩa là một frame vẽ bảng có filter đang bật từng lọc/sort lại dữ liệu tới 2-3 lần độc lập, không chỉ 1.

## Sửa

Thêm 1 field `version: u64` trên `ResultsComponent`, bump ở mọi mutator có thể đổi kết quả của `visible_items()`/`json_lines()`: `set_result` (qua đó cả `set_result_keeping_cursor`), `set_filter`, `sort_by_column`, `toggle_document_view`, và `set_column_types` **chỉ khi type thật sự đổi** (screen gọi hàm này vô điều kiện đúng 1 lần mỗi query outcome — xem comment tại chỗ gọi trong `query_screen.rs` — nên bump vô điều kiện ở đây sẽ vô hiệu hoá cache ngay).

2 cache mới, đúng khuôn `Memoized<T>` của `query_editor.rs` (`RefCell<Option<(version, value)>>`, so khớp version, trả `.clone()` nếu khớp): `visible_items_cache` (`Vec<usize>`), `json_lines_cache` (`Vec<String>`). `visible_items()` cũ đổi tên thân hàm thành `compute_visible_items()` (private), method `visible_items()` giờ chỉ là wrapper kiểm tra cache. Tương tự thêm `cached_json_lines(&self, docs)`, 3 chỗ gọi `json_lines(docs, &self.visible_items())` trực tiếp đổi sang gọi `self.cached_json_lines(docs)`.

`draw_table_body`: đổi tham số `filter: &str` thành `visible_rows: &[usize]`, bỏ hẳn lệnh gọi `visible_and_sorted_rows(...)` nội bộ — `ResultsComponent::draw()` giờ tính `self.visible_items()` **một lần** trước `match result` rồi truyền xuống cho cả hai lần gọi `draw_table_body` (kết quả `Table` thật và `Documents` ở table view). Kết quả: một frame vẽ bảng có filter giờ chỉ lọc/sort đúng 1 lần (qua cache, hoặc tính thật nếu version vừa đổi), không phải 2-3 lần như trước.

## Test

`crates/tradar-query-workbench/src/components/results.rs` — toàn bộ 77 test cũ giữ nguyên, không sửa, vẫn pass (bằng chứng cache không đổi hành vi quan sát được). Thêm 1 test mới, `visible_items_and_json_lines_never_serve_a_stale_cache_across_mutations`: gọi liên tiếp `set_filter` với 2 giá trị khác nhau, `sort_by_column` hai lần liên tiếp (asc rồi desc), và `set_filter` trên một kết quả `Documents` — xác nhận mỗi lần đều phản ánh đúng state hiện tại, không trả về kết quả đã cache từ lần gọi trước. Test này thất bại ngay nếu sau này ai thêm một mutator mới ảnh hưởng tới `visible_items()`/`json_lines()` mà quên bump `version`.

`cargo fmt`/`clippy -D warnings`/`make test-unit` sạch (543 test, 542 cũ + 1 mới).

## Chưa làm (để lại, không tự chốt trước)

- Chưa đo benchmark thật (trước/sau) — sandbox không chạy được TUI tương tác để cảm nhận độ trễ, sửa dựa trên đọc code + đúng nguyên tắc đã áp dụng thành công cho `query_editor.rs`/Mongo `list_schema` ở đợt rà 2026-08-18, không phải số đo cụ thể.
- Chưa rà thêm các connector/component khác (ClickHouse, Socket, navigator, table-designer, migrations...) — người dùng chọn làm thẳng phát hiện này, chưa yêu cầu rà rộng hơn.
