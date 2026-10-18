# Comment trong body Elasticsearch; filetype `.tdb` bị mất

## Elasticsearch: comment

Body có `// …`, `/* … */` hay dòng `# …` (Kibana Dev Tools cho phép cả ba) bị chặn ở bước kiểm JSON local (`invalid JSON body`), và dòng `#` đầu request làm `parse_query` đọc nhầm method. Nay `strip_comments` chạy trong `parse_query` (mọi đường vào: `execute`, `edit_source`, …): dòng bắt đầu bằng `#` bị bỏ ở mọi nơi (chuỗi JSON không xuống dòng được nên không dòng body hợp lệ nào bắt đầu bằng `#`); `//` và `/* */` chỉ tìm trong body, nhận biết chuỗi `"…"`, nên `GET a//b/_search` và `"http://x # y"` giữ nguyên. Test: 2 ca mới trong `tradar-connector-elasticsearch`.

## `.tdb` mất filetype → không tô màu, `rr` gửi cả fence

- Gốc: `vim.filetype.add` nằm trong `plugin/tradar.lua`, chỉ chạy khi plugin load. Nếu buffer được đọc trước đó (plugin lazy, session restore) Neovim tự đoán — với `.tdb` thường là `conf` (dòng đầu `#`) — và block/highlight đều tắt (`block_at` yêu cầu `filetype == 'tdb'`), nên cả buffer (kể cả dòng ```) đi lên server như một query.
- Sửa: `ftdetect/{tdb,mongo,redis,esq}.lua` (nằm trong runtimepath từ lúc khởi động); `plugin/tradar.lua` đặt lại filetype cho buffer đang mở khi load và qua `BufRead/BufNewFile *.tdb`; tự `vim.treesitter.start(buf, 'markdown')` (fallback `syntax=markdown`) thay vì trông chờ vào cấu hình auto-start của nvim-treesitter; `M.is_tdb(buf)` nhận `.tdb` theo tên file nên block/chạy/completion vẫn đúng dù filetype bị ghi đè; `M.setup` và `maybe_attach` gắn keymap cho buffer đã mở trước `setup()`.
- Kiểm chứng (headless): đọc buffer trước khi plugin vào runtimepath → `conf`; sau khi `plugin/tradar.lua` chạy → `tdb`, `syntax=markdown`, đã attach; mở lại qua autocmd → `tdb`; filetype bị ghi đè thành `text` mà `rr` trong block vẫn chạy đúng câu.
