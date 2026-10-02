# 7 gap UX rải khắp app (2026-10-02)

## Bối cảnh

Sau khi merge PR cho `connector-ux-gaps-2026-10-02.md`, người dùng tiếp tục chỉ đạo "tối ưu UI/UX" và để agent tự chọn vùng audit tiếp (lượt trước đã chọn sẵn cho người dùng hai lần liên tiếp). Agent audit lần này không giới hạn theo connector mà rà các mảng còn sót: độ sâu HTTP connector, table designer/migration tracking, kỷ luật dùng theme thay vì màu cứng, results grid với dataset lớn, friction config/docs, độ sâu picker snippet/history, nhất quán thông báo lỗi. Tìm được 7 gap cụ thể. Người dùng chọn qua `AskUserQuestion`: "Làm tất cả 7, rank rẻ→đắt".

## Thay đổi

### 1. `tradar --help`/`--version` không in gì, launch TUI luôn

`clap` đã là dependency của `tradar-app` (ghi trong `CLAUDE.md`'s "Present in `tradar-app/Cargo.toml` but not yet used by any code") nhưng chưa từng được gọi — `std::env::args()` không được đọc ở đâu cả, nên `tradar --help` launch thẳng TUI như không có argument gì.

Thêm struct `Cli` rỗng (`#[derive(Parser)]`, `name`/`version`/`about`) và gọi `Cli::parse()` ở đầu `main()`, trước mọi setup terminal/theme/config — `clap` tự in và exit cho `--help`/`--version`, không cần field nào để bind vì chưa có subcommand/flag thật.

### 2. HTTP/Socket: lỗi không có màu phân biệt

HTTP's response pane hiện lỗi (`session.error`) bằng text thường, không dùng `theme().error` như error message ở results grid/Kafka/RabbitMQ sidebar. Socket's log panel khi mất kết nối đổi title thành "disconnected" nhưng border vẫn màu thường, khác với quy ước `QueryEditorComponent::draw` đã có cho badge mất kết nối SQL/Mongo/ES (border đổi màu `theme.error`).

HTTP (`draw_response` trong `crates/tradar-connector-http/src/screen.rs`): render lỗi bằng `Paragraph` riêng, `Style::default().fg(theme().error)`, return sớm để thay hẳn (không vẽ cùng) response cũ — xem gap #3. Socket (`draw_log` trong `crates/tradar-connector-socket/src/screen.rs`): đổi `block` thành `mut`, thêm `block.border_style(Style::default().fg(theme().error))` khi `!self.session.connected`.

### 3. HTTP: resend lỗi vẫn hiện response thành công cũ

`HttpSession::tick()`'s `HttpEvent::Response(Err(e))` arm set `self.error` nhưng không clear `self.response` — gửi lại request cũ bị lỗi (mất mạng, sai URL...) vẫn hiện nguyên response thành công lần trước, trong khi title đổi thành "error": nhìn như app đang nói dối về kết quả.

Thêm `self.response = None;` ngay khi gán lỗi (`crates/tradar-connector-http/src/lib.rs`). Kết hợp với gap #2's early-return trong `draw_response`, lỗi giờ thay hẳn response cũ trên UI, không hiện song song.

### 4. History/snippet picker không filter được

`HistoryPickerComponent` và `SnippetPickerComponent` không có `/`-filter, trong khi navigator/Kafka/RabbitMQ sidebar/Redis browse-sidebar/HTTP request-picker đều đã có từ các lượt trước — picker lịch sử query hoặc snippet nhiều mục thì không lọc được là gap rõ.

Cùng pattern filter-bar đã chuẩn hoá: `filter: String` + `filter_input: Option<TextInput>`, `open_filter()`/`is_filtering()`/`filter_key_event()`, `visible_entries()` lọc case-insensitive (`SnippetPickerComponent` lọc theo cả tên và nội dung snippet). Khác biệt so với các picker trước: `/` không bind được vào `Context::Prompt`/`Context::List` sẵn có — cả hai context này còn bị các form nhập text thuần khác dùng chung (rename, connection string, table name...), bind `/` vào đó sẽ làm chữ `/` không gõ được nữa ở những form đó. Thêm `Context::History` mới (riêng cho `HistoryPickerComponent`, và lây sang ERD's table picker vì nó tái dùng `HistoryPickerComponent::with_title`) — cùng lý do `Context::Snippets` đã tách riêng từ trước, không phải quyết định mới. `SnippetPickerComponent` vốn đã có `Context::Snippets` riêng nên chỉ cần thêm binding `/` vào context đó.

Tất cả `vim_list::apply` trong hai file đổi sang dùng `self.visible_entries().len()` (tính riêng ra biến `let len = ...` trước, không inline — inline gây lỗi borrow E0502 giữ 2 borrow cùng lúc trên `self`). Load/rename/xoá một entry khi đang filter đều map qua `visible_entries()` để không bao giờ chọn nhầm entry theo raw index.

9 test mới tổng (5 `history_picker.rs`, 4 `snippet_picker.rs`): filter lọc đúng case-insensitive, Esc xoá+reset cursor, Enter giữ, load/rename khi đang filter trúng đúng entry đã lọc chứ không phải theo index gốc.

### 5. HTTP: response pane không search được, dòng dài bị cắt cứng

Response pane render bằng `List`/`ListItem` 1 dòng — dòng dài hơn chiều rộng terminal bị cắt cứng ở lề phải, không có cách nào xem phần còn lại; cũng không có `/`-search như query editor (`buffer_search`) hay JSON view của results grid.

Thêm `response_search`/`response_search_origin`/`last_response_search`, tái dùng pattern search-từ-origin của `QueryScreenComponent::buffer_search` (mỗi phím gõ search lại từ origin, không từ vị trí match trước, tránh trôi; Esc về lại origin; Enter chốt pattern cho `n`/`N` lặp lại) — dùng chung `Command::SearchInBuffer`/`SearchNext`/`SearchPrev` (không đặt tên riêng cho HTTP) với binding mới trong `Context::HttpResponse`.

Đổi hẳn cách render body: từ `List`/`ListItem` mỗi dòng sang một `Paragraph` duy nhất nhận toàn bộ text từ `response_scroll` trở xuống (không cắt sẵn theo `inner.height`), `.wrap(Wrap { trim: false })` — `Paragraph` tự clip theo `Rect` nên không cần tính tay `skip().take()`, và tự động wrap dòng dài xuống dòng kế tiếp thay vì cắt mất. Cân nhắc rồi bỏ qua: đổi `response_scroll` thành chỉ số dòng-sau-khi-wrap (visual row) để chính xác hơn khi có dòng dài — không làm vì sẽ phải tự tính lại wrap-width giống ratatui nội bộ, rủi ro lệch với search vốn đang lập chỉ mục theo dòng logic gốc; giữ `response_scroll` là chỉ số dòng logic, đơn giản và nhất quán với search.

5 test mới: resend lỗi hiện lỗi không phải response cũ (gap #3), dòng dài 300 ký tự vẫn còn đủ trên buffer sau khi wrap (không bị cắt), `/` mở search và nhảy tới match, Esc revert scroll về origin, `n` lặp lại search cuối.

### 6. Migration panel's file list không cuộn được

`MigrationsComponent`'s `Stage::Listing`/`Stage::Running` liệt kê toàn bộ file migration (pending hoặc đang chạy) bằng một `Paragraph` cố định — nhiều file hơn chiều cao popup thì tràn ra ngoài, không có cách xem hết, không giống các overlay list khác trong app (đều đã có `vim_list`).

Thêm `scroll`/`pending_g`/`visible_height`, tái dùng đúng shape `SchemaDiffComponent` đã dùng cho report thuần cuộn-không-chọn (không phải list có cursor như `ResultsComponent`/`HistoryPickerComponent` — ở đây không có gì để "chọn", chỉ có nội dung để đọc). `handle_key_event` đổi signature thêm `modifiers: KeyModifiers` (trước chỉ nhận `code`, không nhận diện được `Ctrl-d`/`Ctrl-u`); `draw` đổi từ `&self` sang `&mut self` để cache `visible_height` — cập nhật theo ở `query_screen.rs`'s 2 call site thật (`handle_key_event`/`draw`) và 2 test helper. Tách phần build `Vec<Line>` ra hàm riêng `display_lines()` dùng chung cho cả `draw` (render) và `line_count()` (biết độ dài trước khi `draw()` chạy lần đầu, để `Ctrl-d`/`Ctrl-u`/`G` tính đúng ngay cả trước frame đầu tiên). `Stage::Listing`/`Running` thử nhận diện `vim_list::recognize` trước — một motion (kể cả `g` đơn đang đợi `g` thứ hai) không bao giờ bị coi là "phím khác" để đóng panel hay (ở `Listing`) chạy `y`/`Y`; scroll reset về 0 mỗi khi đổi stage (`start_running`/`advance`/`fail`) vì nội dung mỗi stage khác hẳn nhau.

7 test mới: `j` cuộn không đóng panel, `G` nhảy đúng cuối danh sách dài, `g` đơn (nửa `gg`) không đóng panel, `Ctrl-d` cuộn nửa trang ở `Stage::Running`, đổi stage reset scroll.

### 7. MongoDB `find`/`aggregate` không giới hạn số document tải vào RAM

`find`/`aggregate` trong `crates/tradar-connector-mongo/src/lib.rs` dùng `while let Some(doc) = cursor.try_next().await?` không giới hạn — một collection lớn load hết vào `Vec` một lần, đúng rủi ro "Support large result sets efficiently" trong `CLAUDE.md` mà các driver SQL đã tự giới hạn qua `query_driver::MAX_ROWS` (10,000) từ trước, Mongo thì chưa.

Thêm `truncated: bool` vào `QueryResult::Documents` (đổi từ tuple variant `Documents(Vec<Value>)` sang struct variant `Documents { items, truncated }`, giống hệt `Table::truncated` đã có) — đổi một phát 72 chỗ dùng trên 6 file (`tradar-connector-mongo`, `tradar-connector-elasticsearch`, `tradar-connector-redis`, và 3 chỗ trong `tradar-query-workbench`: `results.rs`/`query_engine.rs`/`export.rs`), phần việc tốn công nhất trong cả 7 gap như audit đã cảnh báo trước ("medium-large"). Elasticsearch/Redis/RabbitMQ luôn trả `truncated: false` vì chúng tự giới hạn ở tầng request riêng (`_search` size, một lệnh `SCAN`/peek) từ trước, không cần sửa logic, chỉ đổi cú pháp constructor.

`find`'s cursor loop: nhánh `.count()` (`wants_count`) phải scan hết cursor để ra số đúng (không giới hạn được, bản chất `.count()` là tổng thật) nhưng không materialize document nào — chỉ tăng biến `total`, không push vào `docs`, nên vẫn giữ RAM thấp dù phải đọc hết; nhánh không có `.count()` dừng ngay khi `docs.len() >= MAX_ROWS`, set `truncated = true`, `break` sớm (không đọc tiếp phần còn lại của cursor, khác với nhánh count). `aggregate`'s loop đơn giản hơn (không có biến thể count), cùng kiểu cap+break.

`ResultsComponent`'s title bar thêm nhánh "— truncated" cho `Documents` giống `Table` đã có ("Results (first N documents — truncated)"), theo đúng quy tắc "nói lớn, không âm thầm cắt" `Table::truncated` đã đặt ra.

3 test mới trong `tradar-connector-mongo` (cần Docker, không chạy được trong sandbox này — xem "Chưa làm"): `find()` chèn `MAX_ROWS + 1` document rồi xác nhận chỉ tải đúng `MAX_ROWS`, `truncated: true`; `find().count()` trên cùng dataset vẫn ra số chính xác `MAX_ROWS + 1` (không bị cap) và `truncated: false`; `find()` dưới cap không bị đánh dấu truncated. 1 test tương tự cho `aggregate()`.

## Test

- `cargo build --workspace`: sạch.
- `cargo clippy --all-targets --workspace -- -D warnings`: sạch.
- `cargo fmt --all -- --check`: sạch.
- `make test-unit`: toàn bộ pass (bao gồm `tradar-core` 125, `tradar-query-workbench` 627, `tradar-connector-socket` 19, `tradar-app` theo feature list không-kafka, các crate còn lại loại trừ vì cần Docker).
- `cargo build -p tradar-connector-mongo --tests`: biên dịch sạch (test thật cần Docker, không chạy được trong sandbox này, sẽ chạy ở CI).
- Per-crate trước khi gộp: `tradar-query-workbench` 627/627, `tradar-core` 125/125, `tradar-connector-http` 27/27 (không-Docker), `tradar-connector-socket` 19/19, `tradar-connector-redis`/`tradar-connector-elasticsearch`/`tradar-connector-mongo` build+clippy sạch (test Docker không chạy được, không phải regression).

## Chưa làm

- MongoDB's 3 test truncation mới chưa chạy được trong sandbox này (không có Docker daemon) — sẽ xác nhận ở CI; thiết kế cap đã review kỹ bằng tay (nhánh `.count()` không bị cap, nhánh thường break sớm đúng tại `MAX_ROWS`).
- `QueryResult::Documents`'s đổi shape (tuple → struct variant) chỉ thêm `truncated`, không thêm field nào khác dù về lý Elasticsearch/Redis cũng có thể muốn báo "đã cắt theo size limit riêng của chúng" trong tương lai — hiện tại luôn `false` vì cả hai tự giới hạn ở tầng request, chưa thấy nhu cầu thật.
- Migration panel's scroll chỉ áp dụng cho `Stage::Listing`/`Running` (nơi danh sách file có thể dài) — `Stage::Done`/`Blocked` giữ nguyên "phím nào cũng đóng", không thêm scroll vì nội dung luôn ngắn (vài dòng cố định).
- HTTP response's search vẫn lập chỉ mục theo dòng logic (trước wrap), không theo dòng hiển thị sau wrap — xem lý do ở gap #5, đây là đánh đổi có chủ đích để giữ search đơn giản và đúng, không phải thiếu sót.
