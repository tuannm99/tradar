# Multi-filter kết hợp — xong (2026-09-07)

Gap #10 trong đợt so sánh DataGrip/DBeaver/Studio3T (`docs/roadmap.md`), Tier 4 — "làm tiếp theo" sau khi #7 (Generate SQL từ UI) xong. Chốt phạm vi qua `AskUserQuestion` trước khi code, đúng pattern các mục Tier 4 khác: kết hợp cả hai hướng đề xuất (mở rộng cú pháp ô filter `/` hiện có + thêm panel xem/xoá từng điều kiện), và hỗ trợ cả `AND` lẫn `OR` chứ không chỉ `AND`.

**Cú pháp** (`crates/tradar-query-workbench/src/filter.rs`, module mới) — không dùng ngoặc, không cần quote:

- Một điều kiện là `cột:giá_trị` (tên cột khớp không phân biệt hoa thường với cột thật; không khớp cột nào thì rơi về khớp chuỗi thường trên mọi cell — giữ đúng hành vi cũ, và một giá trị tình cờ chứa `:` như timestamp/URL vẫn tìm được) hoặc một chuỗi trần (khớp bất kỳ cell nào, y hệt filter đơn cũ).
- Nhiều điều kiện nối bằng `AND`/`OR` (không phân biệt hoa thường, phải đứng riêng một từ — `Andes`/`oracle` không bị cắt nhầm thành từ khoá). `AND` ưu tiên hơn `OR` giống SQL: `a AND b OR c` đọc là `(a AND b) OR c` — kết quả dạng DNF, một dòng khớp khi **có ít nhất một** nhóm `OR` khớp, một nhóm khớp khi **mọi** điều kiện `AND` trong nhóm đó khớp.
- Filter rỗng vẫn hiện mọi dòng, y hệt trước đây (`ParsedFilter::is_empty`).

**Kiến trúc dữ liệu**: `ParsedFilter`/`FilterCondition` (`filter.rs`) tách khỏi `components::results` — cả `results.rs` lẫn panel mới `filter_conditions.rs` cùng dùng chung, không lệch cách hiểu cú pháp. `FilterCondition` chốt sẵn `(tên cột hiển thị, index cột)` lúc parse (không tra lại `columns` mỗi dòng mỗi frame) cộng `value_lower` tính sẵn một lần. `ParsedFilter::matches_row` dùng cho `QueryResult::Table`/Documents-table-view (có cột thật); `matches_text` dùng cho Documents ở JSON view (không có `columns`, mọi điều kiện tự rơi về chuỗi trần, kiểm tra trên text JSON cả document — vẫn giữ đúng short-circuit cũ: không stringify document khi filter rỗng).

**Điểm cắm dùng chung** — `filter_table_rows`/`visible_and_sorted_rows` (đã có từ mục sort-by-column) nay nhận thêm `columns: &[String]`, gọi `ParsedFilter::parse` bên trong thay vì so chuỗi trực tiếp; ba nơi gọi (`draw_table_body`, `visible_items()` nhánh `Table`, nhánh Documents-table-view) đều truyền đúng `columns` của kết quả đang có.

**Panel xem/xoá điều kiện** (`crates/tradar-query-workbench/src/components/filter_conditions.rs`, `FilterConditionsComponent` — không phải `Component`, do `QueryScreenComponent` điều khiển trực tiếp, cùng kiểu `HistoryPickerComponent`):

- `F3` (mới, `Context::Results`) mở panel — no-op nếu filter đang rỗng (không có gì để quản lý). Panel tự parse lại filter hiện tại (`ResultsComponent::filter()`/`columns()`) thành `ParsedFilter` riêng của nó, không đọc/ghi thẳng vào `ResultsComponent`.
- Mỗi dòng trong panel là một điều kiện, tự thêm tiền tố `AND `/`OR ` theo đúng quan hệ với dòng ngay trước nó (dòng đầu tiên không có tiền tố) — tính trong `flatten()`, không phải hai loại dòng riêng (điều kiện/dấu phân cách) để danh sách chọn (`j`/`k`) không phải nhảy qua dòng không chọn được.
- `d` xoá điều kiện đang chọn (`ParsedFilter::without`, tự bỏ luôn cả nhóm nếu đó là điều kiện cuối của nhóm), `render()` lại thành chuỗi filter mới, trả về qua `FilterConditionsOutcome::Changed(text)` — `QueryScreenComponent` áp lại qua **đúng** `ResultsComponent::set_filter` mà gõ tay vào ô filter cũng dùng, nên hai đường không thể lệch cách hiểu filter. Xoá hết điều kiện (filter rỗng) thì panel tự đóng luôn, không còn gì để quản lý. `Enter`/`Esc` đóng panel không đổi filter. Double-click một dòng cũng xoá, giống `d`.

**Test**: `filter.rs` — parse/match cho từng trường hợp cú pháp (bare/cột, AND, OR, AND ưu tiên OR, hoa/thường, từ chứa "and"/"or" không bị cắt nhầm, cột sai tên rơi về chuỗi trần, `render`/`without` round-trip). `filter_conditions.rs` — liệt kê đúng tiền tố AND/OR, xoá qua phím/double-click, xoá hết đóng panel, `Esc` không đổi gì. `results.rs` — 3 test tích hợp qua `ResultsComponent` thật (`col:value`, `AND`, `OR`) cạnh các test filter đơn đã có. `query_screen.rs` — `F3` mở/không mở panel, xoá một điều kiện áp lại filter còn lại và giữ panel mở, xoá điều kiện cuối đóng panel, `Esc` giữ nguyên filter.

**Xác minh tay** (tmux, sqlite thật, bảng 4 dòng `id/city/status`): `city:hanoi AND status:active` lọc đúng 1/4 dòng; `status:active OR status:pending` lọc đúng 3/4 dòng; `F3` liệt kê đúng 2 điều kiện kèm tiền tố `OR`; xoá điều kiện thứ hai, bảng kết quả cập nhật ngay thành 2/4 dòng (`status:active`), panel vẫn mở; `Esc` đóng panel, filter `status:active` vẫn còn nguyên trên tiêu đề Results.

README.md cập nhật đoạn mô tả panel Results (thêm mô tả cú pháp `cột:giá_trị`/`AND`/`OR` và `F3` cạnh đoạn `/` filter đã có).
